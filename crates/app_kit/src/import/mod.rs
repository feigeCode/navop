//! 连接导入/导出与从全量产品迁移。
//!
//! 三个产品共用同一份迁移包格式（见 [`bundle`]），全量产品还可以直接指定
//! `one-hub.db` 只读迁移（见 [`sqlite`]），连接密码与被引用的钥匙串条目会
//! 在提供源环境主密钥时一并解开并重建。导入先做纯函数规划
//! （判重、重命名、工作区匹配），再逐条落库并广播事件，保证列表即时刷新。

pub(crate) mod bundle;
mod dialog;
mod kinds;
mod sqlite;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use chrono::Utc;
use gpui::App;
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event_from_app};
use one_core::crypto;
use one_core::storage::traits::Repository as _;
use one_core::storage::{
    ConnectionRepository, ConnectionType, CredentialEntry, CredentialReference,
    CredentialRepository, GlobalStorageState, StorageManager, StoredConnection, Workspace,
    WorkspaceRepository, has_connection_decrypt_failure, redact_sensitive_params,
};

use self::bundle::plan::{
    ImportPlan, PlanInput, PlannedConnection, PlannedWorkspace, normalize_name, plan_import,
};

pub use self::bundle::{
    BUNDLE_VERSION, BundleConnection, BundleWorkspace, ConnectionBundle, ImportReport,
};
use self::bundle::{
    BundleCredential, CREDENTIALS_FORMAT, CREDENTIALS_PAYLOAD_VERSION, ConnectionCredentials,
    CredentialsPayload, EncryptedCredentials, workspace_bundle_id,
};
pub use self::dialog::open_import_dialog;
pub use self::sqlite::{
    detect_full_product, full_product_candidates, is_sqlite_database, read_bundle_from_sqlite,
};

pub(crate) use self::kinds::supported_connection_types;

/// 从全量产品的数据库文件一键迁移（按全量产品自己的目录规则自动探测源库）。
pub fn import_from_full_product(cx: &mut App, source_key: Option<&str>) -> Result<ImportReport> {
    let path = detect_full_product().ok_or_else(|| {
        anyhow!(
            "未自动找到全量产品的 one-hub.db，请点「选择文件…」手动指定（常规安装位置：~/.config/navop/one-hub.db；便携版在安装目录 data/config/one-hub.db）"
        )
    })?;
    import_from_database(&path, source_key, cx)
}

/// 从用户指定的数据库文件迁移（只读打开，不改动源库）。
pub fn import_from_database(
    path: &Path,
    source_key: Option<&str>,
    cx: &mut App,
) -> Result<ImportReport> {
    let migration = self::sqlite::read_migration_from_sqlite(path, source_key)?;
    ensure_supported_version(&migration.bundle)?;
    import_bundle_with_credentials(migration.bundle, migration.credentials, cx)
}

/// 从用户指定的源文件迁移：SQLite 库按全量产品库读取，其余按迁移包 JSON 读取。
///
/// 独立产品没有「全量产品目录」可探测时（换机、便携版、全量版已卸载），这是唯一
/// 的迁移入口，因此不能只看扩展名：`one-hub.db` 常被重命名或没有扩展名。
///
/// `password` 只在源文件是 v3 迁移包且包含加密凭据时需要；`source_key` 用于
/// 直接读取 SQLite 源库时解开源环境主密钥加密的凭据。
pub fn import_from_source_file(
    path: &Path,
    password: Option<&str>,
    source_key: Option<&str>,
    cx: &mut App,
) -> Result<ImportReport> {
    if is_sqlite_database(path) {
        return import_from_database(path, source_key, cx);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("读取「{}」失败", path.display()))?;
    import_bundle(&text, password, cx)
}

/// 展开用户手填/粘贴的路径：去空白与引号、去 `file://` 前缀、展开 `~` 与 `%VAR%`。
///
/// 弹窗会优先让用户点「选择文件…」，但输入框仍允许手写，而手写时常见的
/// `"~/Desktop/one-hub.db"`（从 Finder 复制/拖入带引号）与 Windows 的
/// `%USERPROFILE%\...` 必须能读通。
pub fn expand_source_path(input: &str) -> PathBuf {
    let trimmed = input.trim().trim_matches(|ch| ch == '"' || ch == '\'');
    let without_scheme = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    let expanded = expand_env_vars(without_scheme);
    if expanded == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(expanded));
    }
    if let Some(rest) = expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(expanded)
}

/// 展开 `%VAR%`（Windows 手写路径的常见写法）；不认识的变量原样保留。
fn expand_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        let Some(end_offset) = rest[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + end_offset;
        let name = &rest[start + 1..end];
        out.push_str(&rest[..start]);
        match std::env::var(name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => out.push_str(&rest[start..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// 迁移包里的 JSON 字符串必须是合法 JSON，否则说明包已被破坏。
fn validate_params(connection: &BundleConnection) -> Result<()> {
    if connection.params.trim().is_empty() {
        return Ok(());
    }
    serde_json::from_str::<serde_json::Value>(&connection.params)
        .map(|_| ())
        .with_context(|| format!("连接「{}」的 params 不是合法 JSON", connection.name))
}

/// 把当前产品的连接与工作区导出成迁移包 JSON。
///
/// `export_password` 由用户输入，用来加密迁移包里的连接凭据（密码/密钥等）。
/// 可见的 `params` 会被脱敏，导入方必须提供同一密码才能还原凭据，因此迁移包
/// 即使被转发或误上传，也不会直接泄漏明文密码。
pub fn export_bundle(cx: &App, export_password: &str) -> Result<String> {
    let connections = connection_repository(cx)?
        .list()
        .map_err(|error| anyhow!("读取连接失败：{error}"))?;
    let workspaces = workspace_repository(cx)?
        .list()
        .map_err(|error| anyhow!("读取工作区失败：{error}"))?;
    // 只导出被连接引用到的钥匙串条目：没有引用时连仓库都不碰，避免主密钥
    // 未解锁时无谓地阻断导出；有引用时列表解密失败说明本机凭据不可读，必须
    // 报错而不是静默导出「空密码」。
    let (credential_ids, credential_cloud_ids) = collect_credential_references(&connections);
    let vault_credentials = if credential_ids.is_empty() && credential_cloud_ids.is_empty() {
        Vec::new()
    } else {
        // 只按引用逐条取，避免钥匙串里未被引用的坏条目阻断导出。
        let repository = credential_repository(cx)?;
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        for id in &credential_ids {
            let entry = repository.get(*id).map_err(|error| {
                anyhow!("读取钥匙串凭据失败：{error}，请先在设置中设置并解锁主密钥后再导出")
            })?;
            let Some(entry) = entry else {
                continue;
            };
            if entry.id.is_some_and(|id| seen.insert(id)) {
                entries.push(entry);
            }
        }
        for cloud_id in &credential_cloud_ids {
            let entry = repository.get_by_cloud_id(cloud_id).map_err(|error| {
                anyhow!("读取钥匙串凭据失败：{error}，请先在设置中设置并解锁主密钥后再导出")
            })?;
            let Some(entry) = entry else {
                continue;
            };
            if entry.id.is_some_and(|id| seen.insert(id)) {
                entries.push(entry);
            }
        }
        entries
    };
    let bundle = build_bundle(
        &connections,
        &workspaces,
        &vault_credentials,
        crate::identity().map(|identity| identity.app_id.to_string()),
        export_password,
    )?;
    serde_json::to_string_pretty(&bundle).context("序列化迁移包失败")
}

/// 生成迁移包（纯函数，便于单测覆盖「不泄漏明文 + 密码可还原」）。
fn build_bundle(
    connections: &[StoredConnection],
    workspaces: &[Workspace],
    vault_credentials: &[CredentialEntry],
    app_id: Option<String>,
    export_password: &str,
) -> Result<ConnectionBundle> {
    if export_password.is_empty() {
        bail!("导出密码不能为空");
    }
    let workspace_names: HashMap<i64, String> = workspaces
        .iter()
        .filter_map(|workspace| Some((workspace.id?, workspace.name.clone())))
        .collect();

    for connection in connections {
        // 先确认所有连接的本机凭据都能解开：主密钥未解锁时 ENC: 字段会解成空串，
        // 直接导出会把「空密码」写进迁移包，导入方还以为迁移成功。
        if has_connection_decrypt_failure(connection.connection_type, &connection.params) {
            bail!(
                "连接「{}」的密码/密钥无法解密，请先在设置中解锁主密钥后再导出",
                connection.name
            );
        }
        // 非 JSON 参数无法识别敏感字段，脱敏会原样放行；宁可拒绝导出，也不把
        // 可能含密码的原始内容写进可见参数。
        if !connection.params.is_empty()
            && serde_json::from_str::<serde_json::Value>(&connection.params).is_err()
        {
            bail!(
                "连接「{}」的配置不是合法 JSON，无法安全脱敏，请修复后再导出",
                connection.name
            );
        }
    }

    // 只带上本包连接真正引用到的条目，避免把整个钥匙串（包括与本产品无关的
    // 凭据）复制进迁移包。连接可能只带 cloud id，因此两种引用都要匹配。
    let (referenced_ids, referenced_cloud_ids) = collect_credential_references(connections);
    let credentials: Vec<BundleCredential> = vault_credentials
        .iter()
        .filter_map(|entry| entry.id.map(|id| (id, entry)))
        .filter(|(id, entry)| {
            referenced_ids.contains(id)
                || entry
                    .cloud_id
                    .as_ref()
                    .is_some_and(|cloud_id| referenced_cloud_ids.contains(cloud_id))
        })
        .map(|(id, entry)| bundle_credential(id, entry))
        .collect();

    // 迁移包必须自洽：引用不到本机条目的连接无法在导入端重建引用，硬导会
    // 让目标端按数字 id 撞上无关凭据，因此这里直接拒绝并指出连接。
    validate_credential_references(connections, vault_credentials)?;

    let payload = CredentialsPayload {
        version: CREDENTIALS_PAYLOAD_VERSION,
        connections: connections
            .iter()
            .enumerate()
            .map(|(index, connection)| ConnectionCredentials {
                index,
                params: connection.decrypt_params(),
            })
            .collect(),
        credentials,
    };
    let payload = serde_json::to_string(&payload).context("序列化连接凭据失败")?;
    let ciphertext = crypto::encrypt_with_passphrase(&payload, export_password)
        .map_err(|error| anyhow!("加密连接凭据失败：{error}"))?;

    Ok(ConnectionBundle {
        version: BUNDLE_VERSION,
        exported_at: Utc::now().timestamp(),
        app_id,
        workspaces: workspaces.iter().map(to_bundle_workspace).collect(),
        connections: connections
            .iter()
            .map(|connection| to_bundle_connection(connection, &workspace_names))
            .collect(),
        encrypted_credentials: Some(EncryptedCredentials {
            format: CREDENTIALS_FORMAT.to_string(),
            ciphertext,
        }),
    })
}

/// 递归收集连接参数里所有 `credential_reference` 指向的本地 id / 云 id。
fn collect_credential_references(
    connections: &[StoredConnection],
) -> (HashSet<i64>, HashSet<String>) {
    collect_references_from_params(
        connections
            .iter()
            .map(|connection| connection.params.as_str()),
    )
}

/// 从任意一批 `params` JSON 中收集钥匙串引用；迁移包与 SQLite 直读共用。
pub(super) fn collect_references_from_params<'a>(
    params: impl IntoIterator<Item = &'a str>,
) -> (HashSet<i64>, HashSet<String>) {
    let mut ids = HashSet::new();
    let mut cloud_ids = HashSet::new();
    for params in params {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(params) else {
            // 非 JSON 参数在导出/导入前会单独拒绝，这里跳过即可。
            continue;
        };
        collect_references_from_value(&value, &mut ids, &mut cloud_ids);
    }
    (ids, cloud_ids)
}

fn collect_references_from_value(
    value: &serde_json::Value,
    ids: &mut HashSet<i64>,
    cloud_ids: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(reference) = object.get("credential_reference").and_then(|reference| {
                serde_json::from_value::<CredentialReference>(reference.clone()).ok()
            }) {
                ids.insert(reference.credential_id);
                if let Some(cloud_id) = reference.credential_cloud_id {
                    cloud_ids.insert(cloud_id);
                }
            }
            for nested in object.values() {
                collect_references_from_value(nested, ids, cloud_ids);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_references_from_value(item, ids, cloud_ids);
            }
        }
        _ => {}
    }
}

fn validate_credential_references(
    connections: &[StoredConnection],
    entries: &[CredentialEntry],
) -> Result<()> {
    let available_ids: HashSet<i64> = entries.iter().filter_map(|entry| entry.id).collect();
    let available_cloud_ids: HashSet<&str> = entries
        .iter()
        .filter_map(|entry| entry.cloud_id.as_deref())
        .collect();
    for connection in connections {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&connection.params) else {
            continue;
        };
        if let Some((credential_id, cloud_id)) =
            unresolved_credential_reference(&value, &available_ids, &available_cloud_ids)
        {
            match cloud_id {
                Some(cloud_id) => bail!(
                    "连接「{}」引用了不存在的钥匙串条目（cloud id：{cloud_id}），请先在钥匙串中重新选择凭据后再导出",
                    connection.name
                ),
                None => bail!(
                    "连接「{}」引用了不存在的钥匙串条目（id：{credential_id}），请先在钥匙串中重新选择凭据后再导出",
                    connection.name
                ),
            }
        }
    }
    Ok(())
}

/// 返回第一个解析不到本机条目的引用；cloud id 存在时以 cloud id 为准，
/// 与运行时解析顺序保持一致。
pub(super) fn unresolved_credential_reference(
    value: &serde_json::Value,
    available_ids: &HashSet<i64>,
    available_cloud_ids: &HashSet<&str>,
) -> Option<(i64, Option<String>)> {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(reference) = object.get("credential_reference").and_then(|reference| {
                serde_json::from_value::<CredentialReference>(reference.clone()).ok()
            }) {
                let resolved = match reference.credential_cloud_id.as_deref() {
                    Some(cloud_id) => available_cloud_ids.contains(cloud_id),
                    None => available_ids.contains(&reference.credential_id),
                };
                if !resolved {
                    return Some((reference.credential_id, reference.credential_cloud_id));
                }
            }
            for nested in object.values() {
                if let Some(missing) =
                    unresolved_credential_reference(nested, available_ids, available_cloud_ids)
                {
                    return Some(missing);
                }
            }
            None
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(missing) =
                    unresolved_credential_reference(item, available_ids, available_cloud_ids)
                {
                    return Some(missing);
                }
            }
            None
        }
        _ => None,
    }
}

fn bundle_credential(id: i64, entry: &CredentialEntry) -> BundleCredential {
    BundleCredential {
        id,
        name: entry.name.clone(),
        username: entry.username.clone(),
        password: entry.password.clone(),
        private_key_path: entry.private_key_path.clone(),
        private_key_content: entry.private_key_content.clone(),
        passphrase: entry.passphrase.clone(),
        ssh_expect: entry.ssh_expect.clone(),
        sync_enabled: entry.sync_enabled,
        forward_to_agent: entry.forward_to_agent,
        cloud_id: entry.cloud_id.clone(),
    }
}

fn to_bundle_workspace(workspace: &Workspace) -> BundleWorkspace {
    BundleWorkspace {
        id: workspace.id.map(workspace_bundle_id),
        parent: workspace.parent_id.map(workspace_bundle_id),
        name: workspace.name.clone(),
        color: workspace.color.clone(),
        icon: workspace.icon.clone(),
    }
}

fn to_bundle_connection(
    connection: &StoredConnection,
    workspace_names: &HashMap<i64, String>,
) -> BundleConnection {
    BundleConnection {
        name: connection.name.clone(),
        connection_type: connection.connection_type.to_string(),
        // 明文凭据走 encrypted_credentials，可见参数只保留非敏感字段用于展示。
        params: redact_sensitive_params(&connection.params),
        workspace: connection
            .workspace_id
            .and_then(|id| workspace_names.get(&id).cloned()),
        remark: connection.remark.clone(),
        selected_databases: connection.selected_databases.clone(),
        sync_enabled: connection.sync_enabled,
    }
}

/// 从迁移包 JSON 导入连接，返回统计结果。
///
/// `password` 用于解开 v3 包的加密凭据；包内没有凭据时可为 `None`。
pub fn import_bundle(json: &str, password: Option<&str>, cx: &mut App) -> Result<ImportReport> {
    let bundle: ConnectionBundle =
        serde_json::from_str(json).context("迁移包 JSON 解析失败，请确认文件内容")?;
    import_parsed_bundle(bundle, password, cx)
}

/// 迁移包版本不能高于当前产品支持的版本。
fn ensure_supported_version(bundle: &ConnectionBundle) -> Result<()> {
    if bundle.version > BUNDLE_VERSION {
        let (found, supported) = (bundle.version, BUNDLE_VERSION);
        bail!("迁移包版本 {found} 高于当前支持的 {supported}，请升级后再导入");
    }
    Ok(())
}

/// 用用户输入的密码解开 v3 包的凭据，并把完整 `params` 回填到对应连接。
///
/// 解密发生在任何数据库写入之前：密码错误时整个导入直接失败，不会留下
/// 「连接建好了但密码是空的」的半成品。全部校验通过后才清空密文字段。
fn apply_bundle_credentials(
    bundle: &mut ConnectionBundle,
    password: Option<&str>,
) -> Result<Vec<BundleCredential>> {
    let Some((format, ciphertext)) = bundle
        .encrypted_credentials
        .as_ref()
        .map(|encrypted| (encrypted.format.clone(), encrypted.ciphertext.clone()))
    else {
        return Ok(Vec::new());
    };
    if format != CREDENTIALS_FORMAT {
        bail!("不支持的迁移包凭据格式：{format}");
    }
    let Some(password) = password.filter(|password| !password.is_empty()) else {
        bail!("迁移包包含加密的连接凭据，请输入导出时设置的密码后再导入");
    };
    let payload = crypto::decrypt_with_passphrase(&ciphertext, password)
        .map_err(|_| anyhow!("密码错误，或迁移包已损坏，无法解密连接凭据"))?;
    let payload: CredentialsPayload =
        serde_json::from_str(&payload).context("迁移包的加密凭据内容已损坏")?;
    if payload.version > CREDENTIALS_PAYLOAD_VERSION {
        let (found, supported) = (payload.version, CREDENTIALS_PAYLOAD_VERSION);
        bail!("迁移包凭据版本 {found} 高于当前支持的 {supported}，请升级后再导入");
    }
    // 解密成功后才丢弃密文；失败路径保持原样，方便用户修正密码后重试。
    bundle.encrypted_credentials = None;
    let CredentialsPayload {
        connections,
        credentials,
        ..
    } = payload;
    let mut covered = vec![false; bundle.connections.len()];
    for credential in connections {
        let Some(connection) = bundle.connections.get_mut(credential.index) else {
            bail!(
                "迁移包的加密凭据引用了不存在的连接（下标 {}）",
                credential.index
            );
        };
        if std::mem::replace(&mut covered[credential.index], true) {
            bail!("迁移包的加密凭据重复引用了连接「{}」", connection.name);
        }
        connection.params = credential.params;
    }
    // 导出端会为每个连接写入完整参数；缺项说明包不完整，继续导入会得到
    // 「密码被清空」的连接，因此直接拒绝。
    if let Some(missing) = covered.iter().position(|covered| !covered) {
        bail!(
            "迁移包的加密凭据缺少连接「{}」的参数，请重新导出",
            bundle.connections[missing].name
        );
    }
    Ok(credentials)
}

/// 校验版本并落库。
fn import_parsed_bundle(
    mut bundle: ConnectionBundle,
    password: Option<&str>,
    cx: &mut App,
) -> Result<ImportReport> {
    ensure_supported_version(&bundle)?;
    let credentials = apply_bundle_credentials(&mut bundle, password)?;
    import_bundle_with_credentials(bundle, credentials, cx)
}

/// 校验导入包里的引用都能在本包的钥匙串条目里找到。
///
/// 只在新包携带凭据时启用；旧 v3 包没有凭据字段，保持原有的宽松导入行为。
fn validate_bundle_credential_references(
    connections: &[BundleConnection],
    credentials: &[BundleCredential],
) -> Result<()> {
    let available_ids: HashSet<i64> = credentials.iter().map(|credential| credential.id).collect();
    let available_cloud_ids: HashSet<&str> = credentials
        .iter()
        .filter_map(|credential| credential.cloud_id.as_deref())
        .collect();
    for connection in connections {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&connection.params) else {
            continue;
        };
        if let Some((credential_id, cloud_id)) =
            unresolved_credential_reference(&value, &available_ids, &available_cloud_ids)
        {
            match cloud_id {
                Some(cloud_id) => bail!(
                    "连接「{}」引用了迁移包中不存在的钥匙串条目（cloud id：{cloud_id}），请重新导出",
                    connection.name
                ),
                None => bail!(
                    "连接「{}」引用了迁移包中不存在的钥匙串条目（id：{credential_id}），请重新导出",
                    connection.name
                ),
            }
        }
    }
    Ok(())
}

/// 用已经解开的凭据落库；迁移包与全量产品数据库直读共用这一段。
fn import_bundle_with_credentials(
    mut bundle: ConnectionBundle,
    credentials: Vec<BundleCredential>,
    cx: &mut App,
) -> Result<ImportReport> {
    if !credentials.is_empty() {
        validate_bundle_credential_references(&bundle.connections, &credentials)?;
    }
    for connection in &bundle.connections {
        validate_params(connection)?;
    }
    // 先建钥匙串条目拿到目标端 id，再把连接参数里的 credential_reference
    // 重映射过去；之后 plan/apply 只会看到可用的本地引用。
    let credential_ids = import_credentials(&credentials, cx)?;
    if !credential_ids.is_empty() {
        for connection in &mut bundle.connections {
            connection.params = remap_credential_references(&connection.params, &credential_ids)?;
        }
    }
    let connections_repo = connection_repository(cx)?;
    let workspaces_repo = workspace_repository(cx)?;
    let existing_connections = connections_repo
        .list()
        .map_err(|error| anyhow!("读取已有连接失败：{error}"))?;
    let existing_workspaces = workspaces_repo
        .list()
        .map_err(|error| anyhow!("读取已有工作区失败：{error}"))?;

    let existing_names: Vec<String> = existing_connections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let existing_identities: Vec<(String, String)> = existing_connections
        .iter()
        .map(|c| (normalize_name(&c.name), c.connection_type.to_string()))
        .collect();
    let existing_workspace_names: Vec<String> =
        existing_workspaces.iter().map(|w| w.name.clone()).collect();
    let plan = plan_import(
        &bundle,
        PlanInput {
            existing_names: &existing_names,
            existing_identities: &existing_identities,
            existing_workspaces: &existing_workspace_names,
        },
        &supported_connection_types(),
    );
    let mut workspace_ids: HashMap<String, i64> = existing_workspaces
        .iter()
        .filter_map(|workspace| Some((normalize_name(&workspace.name), workspace.id?)))
        .collect();
    apply_plan(plan, &mut workspace_ids, cx)
}

/// 逐条写入连接与新建工作区，并广播事件。
fn apply_plan(
    plan: ImportPlan,
    workspace_ids: &mut HashMap<String, i64>,
    cx: &mut App,
) -> Result<ImportReport> {
    let report = plan.report.clone();
    create_workspaces(plan.workspaces, workspace_ids, cx)?;
    create_connections(plan.connections, workspace_ids, cx)?;
    Ok(report)
}

fn create_workspaces(
    workspaces: Vec<PlannedWorkspace>,
    workspace_ids: &mut HashMap<String, i64>,
    cx: &mut App,
) -> Result<()> {
    let repo = workspace_repository(cx)?;
    for planned in workspaces {
        // 父工作区一定已经落库（规划阶段按「父在前」排序），这里直接按名称取本地 id。
        let parent_id = planned
            .parent_key
            .as_ref()
            .and_then(|key| workspace_ids.get(key).copied());
        let mut workspace = Workspace {
            id: None,
            name: planned.name,
            color: planned.color,
            icon: planned.icon,
            parent_id,
            created_at: None,
            updated_at: None,
            cloud_id: None,
            last_synced_at: None,
            sort_order: None,
            sidebar_collapsed: false,
        };
        let name = workspace.name.clone();
        let id = repo
            .insert(&mut workspace)
            .map_err(|error| anyhow!("创建工作区「{name}」失败：{error}"))?;
        workspace_ids.insert(normalize_name(&name), id);
        emit_connection_event_from_app(
            ConnectionDataEvent::WorkspaceCreated { workspace_id: id },
            cx,
        );
    }
    Ok(())
}

fn create_connections(
    connections: Vec<PlannedConnection>,
    workspace_ids: &HashMap<String, i64>,
    cx: &mut App,
) -> Result<()> {
    let repo = connection_repository(cx)?;
    for planned in connections {
        let workspace_id = planned
            .workspace_key
            .as_ref()
            .and_then(|key| workspace_ids.get(key).copied());
        let mut connection = StoredConnection {
            id: None,
            credential_revision: None,
            name: planned.name,
            connection_type: ConnectionType::from_str(&planned.connection_type),
            params: planned.params,
            workspace_id,
            selected_databases: planned.selected_databases,
            remark: planned.remark,
            sync_enabled: planned.sync_enabled,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
            preferred_open_mode: None,
            ai_access_level: Default::default(),
        };
        let name = connection.name.clone();
        repo.insert(&mut connection)
            .map_err(|error| anyhow!("导入连接「{name}」失败：{error}"))?;
        emit_connection_event_from_app(ConnectionDataEvent::ConnectionCreated { connection }, cx);
    }
    Ok(())
}

/// 把迁移包里的钥匙串条目建到本机，返回「源端 id -> (目标端 id, 目标端 cloud id)」。
///
/// cloud id 命中本机已有条目时复用，避免同一账号重复导入产生重复凭据；否则
/// 新建本地条目并清空云同步元数据（team/owner/时间戳），防止把源账号的归属
/// 带到目标账号。新条目统一由本机主密钥加密后落库。
fn import_credentials(records: &[BundleCredential], cx: &mut App) -> Result<CredentialMapping> {
    if records.is_empty() {
        return Ok(CredentialMapping::default());
    }
    if records.iter().any(credential_has_secrets) && !crypto::has_master_key() {
        bail!("导入内容包含钥匙串凭据，请先在设置中设置并解锁主密钥后再导入");
    }
    let repository = credential_repository(cx)?;
    let mut mapping = CredentialMapping::default();
    for record in records {
        if let Some(cloud_id) = record.cloud_id.as_deref() {
            let existing = repository
                .get_by_cloud_id(cloud_id)
                .map_err(|error| anyhow!("读取已有钥匙串凭据失败（可能未解锁主密钥）：{error}"))?;
            if let Some(existing) = existing {
                let target_id = existing
                    .id
                    .ok_or_else(|| anyhow!("已有钥匙串凭据「{}」缺少本地 id", existing.name))?;
                mapping.insert(
                    record.id,
                    record.cloud_id.as_deref(),
                    target_id,
                    existing.cloud_id,
                );
                continue;
            }
        }
        let mut entry = CredentialEntry {
            id: None,
            name: record.name.clone(),
            username: record.username.clone(),
            password: record.password.clone(),
            private_key_path: record.private_key_path.clone(),
            private_key_content: record.private_key_content.clone(),
            passphrase: record.passphrase.clone(),
            ssh_expect: record.ssh_expect.clone(),
            sync_enabled: record.sync_enabled,
            forward_to_agent: record.forward_to_agent,
            cloud_id: None,
            last_synced_at: None,
            team_id: None,
            owner_id: None,
            created_at: None,
            updated_at: None,
        };
        let target_id = repository
            .insert(&mut entry)
            .with_context(|| format!("导入钥匙串条目「{}」失败", record.name))?;
        emit_connection_event_from_app(
            ConnectionDataEvent::CredentialCreated {
                credential_id: target_id,
            },
            cx,
        );
        mapping.insert(record.id, record.cloud_id.as_deref(), target_id, None);
    }
    Ok(mapping)
}

fn credential_has_secrets(record: &BundleCredential) -> bool {
    record
        .password
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || record
            .private_key_content
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || record
            .passphrase
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || !record.ssh_expect.is_empty()
}

/// 迁移包里源端凭据 id / cloud id 到目标端条目的映射。
#[derive(Default)]
struct CredentialMapping {
    by_id: HashMap<i64, CredentialTarget>,
    by_cloud_id: HashMap<String, CredentialTarget>,
}

type CredentialTarget = (i64, Option<String>);

impl CredentialMapping {
    fn is_empty(&self) -> bool {
        self.by_id.is_empty() && self.by_cloud_id.is_empty()
    }

    fn insert(
        &mut self,
        source_id: i64,
        source_cloud_id: Option<&str>,
        target_id: i64,
        target_cloud_id: Option<String>,
    ) {
        let target = (target_id, target_cloud_id);
        self.by_id.insert(source_id, target.clone());
        if let Some(cloud_id) = source_cloud_id {
            self.by_cloud_id.insert(cloud_id.to_string(), target);
        }
    }

    /// 优先按 cloud id 匹配（运行时解析也优先 cloud id），否则退回本地 id。
    fn resolve(
        &self,
        source_id: Option<i64>,
        source_cloud_id: Option<&str>,
    ) -> Option<&CredentialTarget> {
        if let Some(cloud_id) = source_cloud_id {
            return self.by_cloud_id.get(cloud_id);
        }
        source_id.and_then(|id| self.by_id.get(&id))
    }
}

/// 改写连接参数里所有 `credential_reference` 的 id/cloud id。
///
/// 只处理键名恰为 `credential_reference` 的对象，避免误改其它业务字段里
/// 恰好叫 `credential_id` 的普通数字。
fn remap_credential_references(params: &str, mapping: &CredentialMapping) -> Result<String> {
    if params.trim().is_empty() {
        return Ok(params.to_string());
    }
    let mut value: serde_json::Value =
        serde_json::from_str(params).context("连接参数不是合法 JSON，无法重映射钥匙串引用")?;
    remap_references_in_value(&mut value, mapping);
    serde_json::to_string(&value).context("序列化连接参数失败")
}

fn remap_references_in_value(value: &mut serde_json::Value, mapping: &CredentialMapping) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(serde_json::Value::Object(reference)) =
                object.get_mut("credential_reference")
            {
                let source_id = reference
                    .get("credential_id")
                    .and_then(serde_json::Value::as_i64);
                let source_cloud_id = reference
                    .get("credential_cloud_id")
                    .and_then(serde_json::Value::as_str);
                if let Some((target_id, target_cloud_id)) =
                    mapping.resolve(source_id, source_cloud_id)
                {
                    reference.insert("credential_id".to_string(), serde_json::json!(target_id));
                    match target_cloud_id {
                        Some(cloud_id) => {
                            reference.insert(
                                "credential_cloud_id".to_string(),
                                serde_json::json!(cloud_id),
                            );
                        }
                        None => {
                            reference.remove("credential_cloud_id");
                        }
                    }
                }
            }
            for nested in object.values_mut() {
                remap_references_in_value(nested, mapping);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remap_references_in_value(item, mapping);
            }
        }
        _ => {}
    }
}

fn credential_repository(cx: &App) -> Result<Arc<CredentialRepository>> {
    storage(cx)?
        .get::<CredentialRepository>()
        .ok_or_else(|| anyhow!("钥匙串仓库未初始化"))
}

fn connection_repository(cx: &App) -> Result<Arc<ConnectionRepository>> {
    storage(cx)?
        .get::<ConnectionRepository>()
        .ok_or_else(|| anyhow!("连接仓库未初始化"))
}

fn workspace_repository(cx: &App) -> Result<Arc<WorkspaceRepository>> {
    storage(cx)?
        .get::<WorkspaceRepository>()
        .ok_or_else(|| anyhow!("工作区仓库未初始化"))
}

fn storage(cx: &App) -> Result<&StorageManager> {
    cx.try_global::<GlobalStorageState>()
        .map(|state| &state.storage)
        .ok_or_else(|| anyhow!("本地存储尚未初始化"))
}

#[cfg(test)]
mod tests;
