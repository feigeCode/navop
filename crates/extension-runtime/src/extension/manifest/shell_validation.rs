use std::collections::HashSet;
use std::path::{Component, Path};

use semver::Version;

use super::parser::validate_extension_path_containment;
use super::{Manifest, ShellAppContrib, ShellHostModule};

const SHELL_EXEC_PERMISSION: &str = "shell:exec";
const RESERVED_BACKEND_ALIASES: &[&str] = &["host", "navop", "default"];
/// navop 内置的 nsh 运行时版本（`nsh-platform-gpui` 的 minor）。
const NSH_VERSION: &str = "0.3.0";

pub(crate) fn validate_shell_app(manifest: &Manifest) -> Result<(), ShellAppValidationError> {
    let Some(app) = manifest.contributes.shell_app.as_ref() else {
        return Ok(());
    };
    if !manifest
        .permissions
        .iter()
        .any(|permission| permission == SHELL_EXEC_PERMISSION)
    {
        return Err(error("/permissions", "shellApp requires `shell:exec`"));
    }
    validate_engine_versions(manifest)?;

    let ipc_ids: HashSet<&str> = manifest
        .runtime
        .ipc
        .iter()
        .map(|runtime| runtime.id.as_str())
        .collect();
    validate_shell_app_fields(manifest, app, &ipc_ids)
}

fn validate_shell_app_fields(
    manifest: &Manifest,
    app: &ShellAppContrib,
    ipc_ids: &HashSet<&str>,
) -> Result<(), ShellAppValidationError> {
    if app
        .title
        .as_deref()
        .is_some_and(|title| title.trim().is_empty())
    {
        return Err(error("/contributes/shellApp/title", "must not be empty"));
    }
    validate_relative_path(&app.entry, app_field("entry"), "entry")?;
    if manifest.manifest_dir.exists() {
        let entry_path = manifest.manifest_dir.join(&app.entry);
        if !entry_path.is_file() {
            return Err(error(
                app_field("entry"),
                format!("shell app entry does not exist: {}", entry_path.display()),
            ));
        }
        validate_extension_path_containment(
            &manifest.manifest_dir,
            &entry_path,
            app_field("entry"),
            "shell app entry",
        )
        .map_err(|error| error_from_manifest(error, app_field("entry")))?;
    }
    if let Some(icon) = app.icon.as_deref() {
        validate_relative_path(icon, app_field("icon"), "icon")?;
        if manifest.manifest_dir.exists() {
            let icon_path = manifest.manifest_dir.join(icon);
            if !icon_path.is_file() {
                return Err(error(
                    app_field("icon"),
                    format!("shell icon does not exist: {}", icon_path.display()),
                ));
            }
            validate_extension_path_containment(
                &manifest.manifest_dir,
                &icon_path,
                app_field("icon"),
                "shell icon",
            )
            .map_err(|error| error_from_manifest(error, app_field("icon")))?;
        }
    }
    validate_backends(app, ipc_ids)?;
    validate_modules(manifest, app)?;
    if let Some(category) = app.category.as_deref() {
        validate_identifier(category, app_field("category"), "toolbox category")?;
    }
    Ok(())
}

/// shellView 必须声明它跑在哪个 UI 平面上，并且那个平面得够新。
///
/// 现在只有 nsh 一个平面：声明了 `engines.nsh` 的扩展才会被登记出
/// shellView。旧的 `engines.gpui_shell` 随 gpui-shell 一起删掉了——
/// 宿主已经没有那条渲染路径，再接受这个门只会让扩展以为自己能跑。
fn validate_engine_versions(manifest: &Manifest) -> Result<(), ShellAppValidationError> {
    let nsh = manifest.engines.nsh.trim();
    if nsh.is_empty() {
        return Err(error("/engines/nsh", "shellApp requires `engines.nsh`"));
    }
    validate_engine_version(nsh, NSH_VERSION, "/engines/nsh", "nsh")
}

fn validate_engine_version(
    required: &str,
    host: &str,
    field: &str,
    engine: &str,
) -> Result<(), ShellAppValidationError> {
    let required = Version::parse(required).map_err(|_| {
        error(
            field,
            format!("shellApp requires a semantic {engine} version"),
        )
    })?;
    let current = Version::parse(host).expect("host shell version must be semantic");
    let compatible = if required.major == 0 {
        current.major == 0 && current.minor == required.minor
    } else {
        current.major == required.major
    };
    if compatible && current >= required {
        return Ok(());
    }
    Err(error(
        field,
        format!("requires {engine} {required}, host provides {current}"),
    ))
}

fn error_from_manifest(error: super::ManifestError, field: String) -> ShellAppValidationError {
    match error {
        super::ManifestError::InvalidField { reason, .. } => {
            ShellAppValidationError { field, reason }
        }
        other => ShellAppValidationError {
            field,
            reason: other.to_string(),
        },
    }
}

fn validate_backends(
    app: &ShellAppContrib,
    ipc_ids: &HashSet<&str>,
) -> Result<(), ShellAppValidationError> {
    for (alias, runtime_id) in &app.backends {
        let field = format!("{}/backends/{alias}", app_field("backends"));
        validate_identifier(alias, field.clone(), "backend alias")?;
        if RESERVED_BACKEND_ALIASES.contains(&alias.as_str()) {
            return Err(error(field, format!("reserved backend alias `{alias}`")));
        }
        if !ipc_ids.contains(runtime_id.as_str()) {
            return Err(error(
                field,
                format!("unknown IPC runtime `{runtime_id}` in this manifest"),
            ));
        }
    }
    Ok(())
}

fn validate_modules(
    manifest: &Manifest,
    app: &ShellAppContrib,
) -> Result<(), ShellAppValidationError> {
    let mut modules = HashSet::new();
    for module in &app.modules {
        // 这张表是**宿主支持面**的清单（`ShellHostModule::is_supported`）：新增
        // 一个 host 模块必须在那里显式登记，否则声明了它的扩展在**安装期**就被
        // 拒（而不是装上了、用到才发现）。
        if !module.is_supported() {
            return Err(error(
                app_field("modules"),
                removed_module_reason(*module).unwrap_or_else(|| {
                    format!("shell host module `{module:?}` is not supported by this host")
                }),
            ));
        }
        // `dev` 暴露的是开发者工具专用宿主函数，只给宿主内置扩展。安装期就
        // 拒掉，别等运行期 `navop.dev` 检查再报 PERMISSION_DENIED。
        if *module == ShellHostModule::Dev
            && !BUILTIN_DEV_EXTENSION_IDS.contains(&manifest.id.as_str())
        {
            return Err(error(
                app_field("modules"),
                format!(
                    "模块 `dev` 只对宿主内置扩展开放（当前白名单：{}）；第三方扩展请从 modules 里去掉它",
                    BUILTIN_DEV_EXTENSION_IDS.join("、")
                ),
            ));
        }
        if !modules.insert(*module) {
            return Err(error(
                app_field("modules"),
                format!("duplicate shell host module `{module:?}`"),
            ));
        }
    }
    if app.backends.is_empty()
        && app
            .modules
            .iter()
            .copied()
            .any(ShellHostModule::requires_backend)
    {
        return Err(error(
            app_field("backends"),
            "resource/job/event/blob modules require at least one backend",
        ));
    }
    Ok(())
}

/// 允许声明 `dev` 模块的内置扩展 id。
///
/// 宿主 catalog 目前没有 builtin/is_builtin 标志（`ExtensionSummary` 不带内置
/// 身份），所以先按 id 白名单；等 catalog 暴露内置身份后改成读标志，扩展清单
/// 侧不需要新增任何声明。
const BUILTIN_DEV_EXTENSION_IDS: &[&str] = &["com.navop.devtools"];

/// v2 删掉的旧模块 → 给扩展作者可自行修复的安装期说明。
///
/// 返回 `None` 表示它不是「已删除的旧值」，调用方落到通用的「不支持」错误。
fn removed_module_reason(module: ShellHostModule) -> Option<String> {
    match module {
        ShellHostModule::Context => Some(
            "模块 `context` 在 ABI v2 已删除：页面上下文改从 host.params 读；请从 modules 里去掉它"
                .to_string(),
        ),
        ShellHostModule::I18n => Some(
            "模块 `i18n` 在 ABI v2 已删除：界面语言由 nsh 原生提供；请从 modules 里去掉它"
                .to_string(),
        ),
        ShellHostModule::Theme => Some(
            "模块 `theme` 在 ABI v2 已删除：组件主题由 nsh 原生提供；请从 modules 里去掉它"
                .to_string(),
        ),
        _ => None,
    }
}

fn validate_identifier(
    value: &str,
    field: String,
    kind: &str,
) -> Result<(), ShellAppValidationError> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-_".contains(&byte)
        })
        || value.starts_with(['.', '-', '_'])
        || value.ends_with(['.', '-', '_'])
        || value.contains("..")
    {
        return Err(error(
            field,
            format!("{kind} may contain only lowercase ASCII, digits, '.', '-' and '_'"),
        ));
    }
    Ok(())
}

fn validate_relative_path(
    value: &str,
    field: String,
    kind: &str,
) -> Result<(), ShellAppValidationError> {
    if value.trim().is_empty() {
        return Err(error(field, format!("{kind} must not be empty")));
    }
    let path = Path::new(value);
    let has_windows_prefix = value.starts_with("\\\\")
        || value.starts_with("//")
        || value.as_bytes().get(1) == Some(&b':');
    if path.is_absolute() || has_windows_prefix {
        return Err(error(field, format!("{kind} must be a relative path")));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(error(
            field,
            format!("{kind} path must not escape the extension root"),
        ));
    }
    Ok(())
}

fn app_field(field: &str) -> String {
    format!("/contributes/shellApp/{field}")
}

fn error(field: impl Into<String>, reason: impl Into<String>) -> ShellAppValidationError {
    ShellAppValidationError {
        field: field.into(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellAppValidationError {
    pub field: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 用 JSON 造清单：`manifest_dir` 反序列化后是空 `PathBuf`（`exists()` 为
    /// false），校验不会去磁盘上找 entry/icon，测试因此不依赖临时目录。
    fn manifest(id: &str, modules: serde_json::Value, backends: serde_json::Value) -> Manifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "id": id,
            "name": "测试扩展",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0", "nsh": "0.3.0" },
            "permissions": ["shell:exec"],
            "runtime": {
                "ipc": [{ "id": "provider", "entry": { "command": "./bin/provider" } }]
            },
            "contributes": {
                "shellApp": {
                    "title": "测试 app",
                    "entry": "ui/dist/bundle.js",
                    "modules": modules,
                    "backends": backends
                }
            }
        }))
        .expect("测试清单必须能被反序列化")
    }

    fn validate_error(manifest: &Manifest) -> ShellAppValidationError {
        validate_shell_app(manifest).expect_err("应当被安装期校验拒绝")
    }

    #[test]
    fn v2_modules_pass_install_validation() {
        let manifest = manifest(
            "com.example.tools",
            json!(["resource", "job", "event", "blob", "log", "runtime"]),
            json!({ "main": "provider" }),
        );
        validate_shell_app(&manifest).expect("v2 模块都属于宿主支持面");
    }

    /// v2 删掉的旧模块：报错必须让扩展作者知道「为什么没了 + 怎么改」，
    /// 不能只说一句 invalid module。
    #[test]
    fn legacy_modules_are_rejected_with_a_fix_hint() {
        for (module, hint) in [
            ("context", "host.params"),
            ("i18n", "nsh 原生"),
            ("theme", "nsh 原生"),
        ] {
            let manifest = manifest("com.example.tools", json!([module]), json!({}));
            let error = validate_error(&manifest);
            assert_eq!("/contributes/shellApp/modules", error.field);
            assert!(error.reason.contains(module), "{}", error.reason);
            assert!(error.reason.contains("ABI v2 已删除"), "{}", error.reason);
            assert!(error.reason.contains(hint), "{}", error.reason);
            assert!(error.reason.contains("去掉"), "{}", error.reason);
        }
    }

    /// `dev` 门禁是安装期拒绝（运行期还有 abi.rs 的 PERMISSION_DENIED）。
    #[test]
    fn dev_module_is_only_for_the_builtin_extension() {
        let builtin = manifest("com.navop.devtools", json!(["dev", "log"]), json!({}));
        validate_shell_app(&builtin).expect("内置开发者工具扩展可以声明 dev");

        let third_party = manifest("com.example.tools", json!(["dev"]), json!({}));
        let error = validate_error(&third_party);
        assert_eq!("/contributes/shellApp/modules", error.field);
        assert!(error.reason.contains("内置扩展"), "{}", error.reason);
        assert!(
            error.reason.contains("com.navop.devtools"),
            "{}",
            error.reason
        );
        assert!(error.reason.contains("去掉"), "{}", error.reason);
    }

    #[test]
    fn backend_modules_still_require_at_least_one_backend() {
        let manifest = manifest("com.example.tools", json!(["resource"]), json!({}));
        let error = validate_error(&manifest);
        assert_eq!("/contributes/shellApp/backends", error.field);
        assert!(error.reason.contains("require at least one backend"));
    }

    #[test]
    fn empty_title_is_rejected_but_missing_title_falls_back_to_name() {
        let manifest: Manifest = serde_json::from_value(json!({
            "schema_version": 1,
            "id": "com.example.tools",
            "name": "测试扩展",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0", "nsh": "0.3.0" },
            "permissions": ["shell:exec"],
            "contributes": { "shellApp": { "title": "", "entry": "ui/dist/bundle.js" } }
        }))
        .expect("清单可反序列化");
        let error = validate_error(&manifest);
        assert_eq!("/contributes/shellApp/title", error.field);
    }

    #[test]
    fn unknown_module_spellings_are_rejected_as_unknown_variants() {
        let error = serde_json::from_value::<ShellAppContrib>(json!({
            "title": "测试 app",
            "entry": "ui/dist/bundle.js",
            "modules": ["blobstore"]
        }))
        .expect_err("未知模块名必须在反序列化阶段就被拒绝");
        assert!(
            error.to_string().contains("unknown variant `blobstore`"),
            "{error}"
        );
    }
}
