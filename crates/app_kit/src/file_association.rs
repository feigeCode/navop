//! 按产品身份修复并注册文件关联。
//!
//! 产品接管哪些扩展名由 [`AppIdentity::file_associations`] 声明，ProgID、
//! 描述与 MIME 类型由本模块的目录补齐；未声明扩展名的产品（例如只做壳或
//! 只做数据面板的产品）不做任何注册。

use anyhow::{Context as _, Result, bail};
use domain_api::AppIdentity;
use gpui::{App, AppContext};
use one_core::storage::manager::get_config_dir;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ASSOCIATION_SCHEMA_VERSION: u32 = 3;
const REGISTRATION_STAMP_FILE: &str = "file-association-registration-v3";
/// 上次写入系统的 URL scheme 清单，用于关闭开关时精确摘除注册。
const REGISTERED_URL_SCHEMES_FILE: &str = "file-association-schemes-v1";
/// 反向域名 `app_id` 里不参与 ProgID 的根段：`com.navop.shell` → `NavopShell`。
const REVERSE_DNS_ROOT: &str = "com";
/// Linux 注册文件名里追加的扩展名。
#[cfg(any(target_os = "linux", test))]
const LINUX_DESKTOP_SUFFIX: &str = ".desktop";
#[cfg(any(target_os = "linux", test))]
const LINUX_MIME_SUFFIX: &str = ".xml";
#[cfg(any(target_os = "linux", test))]
const LINUX_DESKTOP_TEMPLATE: &str = include_str!("../resources/linux/navop.desktop");
#[cfg(any(target_os = "linux", test))]
const LINUX_MIME_TEMPLATE: &str = include_str!("../resources/linux/navop.xml");

/// 产品可以声明的扩展名及其注册信息。
///
/// `AppIdentity::file_associations` 只列扩展名，剩下的注册细节（ProgID 类型段、
/// 描述、MIME 类型）由这张表补齐，避免每个平台各写一份扩展名清单。
struct AssociationKind {
    /// 不带点号的扩展名，与 `AppIdentity::file_associations` 对齐。
    name: &'static str,
    /// 注册表键与图标命令使用的带点号扩展名。
    extension: &'static str,
    /// ProgID 的类型段，例如 `SQLiteDatabase`。
    type_name: &'static str,
    /// Windows 注册表里展示给用户的描述。
    description: &'static str,
    /// 该扩展名对应的 MIME 类型（Linux 默认关联用）。
    #[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
    mime_types: &'static [&'static str],
}

const ASSOCIATION_CATALOG: &[AssociationKind] = &[
    AssociationKind {
        name: "db",
        extension: ".db",
        type_name: "SQLiteDatabase",
        description: "SQLite Database",
        mime_types: &["application/vnd.sqlite3", "application/x-sqlite3"],
    },
    AssociationKind {
        name: "duckdb",
        extension: ".duckdb",
        type_name: "DuckDBDatabase",
        description: "DuckDB Database",
        mime_types: &["application/x-duckdb", "application/vnd.duckdb"],
    },
    AssociationKind {
        name: "md",
        extension: ".md",
        type_name: "MarkdownDocument",
        description: "Markdown Document",
        mime_types: &["text/markdown"],
    },
    AssociationKind {
        name: "cast",
        extension: ".cast",
        type_name: "TerminalRecording",
        description: "Terminal Recording",
        mime_types: &["application/x-asciicast"],
    },
];

/// 某个产品真正要注册的一条文件关联。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociationSpec {
    pub extension: &'static str,
    pub prog_id: String,
    pub description: &'static str,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistryWritePolicy {
    Always,
    IfNoDefaultAssociation { extension: &'static str },
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistryCommand {
    key: String,
    value_name: Option<String>,
    value_type: &'static str,
    data: String,
    policy: RegistryWritePolicy,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxRegistrationFiles {
    desktop_path: PathBuf,
    desktop_contents: String,
    mime_path: PathBuf,
    mime_contents: String,
}

/// 当前产品注册的扩展名（带点号，与注册表键一致）。
pub fn registered_extensions() -> Vec<&'static str> {
    crate::identity()
        .map(|identity| {
            associations_for(identity)
                .into_iter()
                .map(|spec| spec.extension)
                .collect()
        })
        .unwrap_or_default()
}

/// 派生产品要注册的文件关联；未声明扩展名的产品返回空集合。
pub fn associations_for(identity: &AppIdentity) -> Vec<AssociationSpec> {
    let prefix = prog_id_prefix(identity);
    let mut specs = Vec::new();
    for declared in identity.file_associations {
        let Some(kind) = association_kind(declared) else {
            tracing::warn!(extension = *declared, "unknown file association extension");
            continue;
        };
        specs.push(AssociationSpec {
            extension: kind.extension,
            prog_id: format!("{prefix}.{}", kind.type_name),
            description: kind.description,
        });
    }
    specs
}

/// 注册当前产品的文件关联与 URL 协议；两者都没声明的产品直接跳过。
pub fn schedule_registration_with_schemes(cx: &mut App, enabled_schemes: Vec<String>) {
    let Some(identity) = crate::identity() else {
        tracing::debug!("application identity is not configured; skipping file associations");
        return;
    };
    if !claims_any_registration(identity) {
        tracing::debug!(
            "product declares no file associations or URL schemes; skipping registration"
        );
        return;
    }
    cx.background_spawn(async move {
        if let Err(error) = ensure_registered(identity, &enabled_schemes) {
            tracing::warn!(error = %error, "file association migration failed");
        }
    })
    .detach();
}

/// 声明的 scheme 里用户启用的部分（大小写不敏感，保持声明顺序）。
fn enabled_url_schemes<'a>(identity: &AppIdentity, enabled: &[String]) -> Vec<&'a str> {
    identity
        .url_schemes
        .iter()
        .copied()
        .filter(|scheme| {
            enabled
                .iter()
                .any(|candidate| candidate.trim().eq_ignore_ascii_case(scheme))
        })
        .collect()
}

fn association_kind(declared: &str) -> Option<&'static AssociationKind> {
    let name = declared.trim().trim_start_matches('.');
    ASSOCIATION_CATALOG
        .iter()
        .find(|kind| kind.name == name || kind.extension == declared.trim())
}

/// ProgID 前缀：`navop` → `Navop`，`navop-shell` → `NavopShell`，
/// `com.navop.data` → `NavopData`。
fn prog_id_prefix(identity: &AppIdentity) -> String {
    let mut prefix = String::new();
    for segment in identity.app_id.split(['.', '-', '_']) {
        if segment.is_empty() || segment == REVERSE_DNS_ROOT {
            continue;
        }
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            prefix.extend(first.to_uppercase());
            prefix.push_str(chars.as_str());
        }
    }
    prefix
}

/// 产品是否声明了任何需要写进系统的关联（扩展名或 URL scheme）。
fn claims_any_registration(identity: &AppIdentity) -> bool {
    !associations_for(identity).is_empty() || !identity.url_schemes.is_empty()
}

fn ensure_registered(identity: &'static AppIdentity, enabled_schemes: &[String]) -> Result<()> {
    if !claims_any_registration(identity) {
        return Ok(());
    }
    let executable = std::env::current_exe().context("resolve current executable")?;
    let schemes = enabled_url_schemes(identity, enabled_schemes);
    remove_stale_url_schemes(identity, &schemes)?;

    #[cfg(target_os = "windows")]
    let result = ensure_windows_registered(&executable, identity, &schemes);

    #[cfg(target_os = "linux")]
    let result = ensure_linux_registered(&executable, identity, &schemes);

    #[cfg(target_os = "macos")]
    let result = ensure_macos_registered(&executable, identity, &schemes);

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    let result: Result<()> = Ok(());

    result?;
    save_registered_url_schemes(&schemes)
}

/// 之前注册过、现在被用户关掉的 scheme 需要从系统里摘掉，否则关掉开关后
/// 链接仍然会被系统交给 Navop。只删自己写过的键，不碰第三方应用的注册。
fn remove_stale_url_schemes(identity: &AppIdentity, current: &[&str]) -> Result<()> {
    let Some(previous) = read_registered_url_schemes()? else {
        return Ok(());
    };
    let stale = previous
        .iter()
        .filter(|scheme| !current.iter().any(|kept| kept.eq_ignore_ascii_case(scheme)))
        .filter(|scheme| {
            identity
                .url_schemes
                .iter()
                .any(|declared| declared.eq_ignore_ascii_case(scheme))
        })
        .cloned()
        .collect::<Vec<_>>();

    #[cfg(target_os = "windows")]
    for scheme in &stale {
        remove_windows_url_protocol(scheme)?;
    }

    #[cfg(target_os = "linux")]
    {
        // 桌面文件的 MimeType 会随重渲染去掉；Linux 没有「删除默认关联」的
        // 标准做法，未启用的 scheme 由应用层拒绝，不再回写默认值即可。
    }

    let _ = stale;
    Ok(())
}

fn registration_stamp(executable: &Path, identity: &AppIdentity, schemes: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(ASSOCIATION_SCHEMA_VERSION.to_le_bytes());
    hasher.update(std::env::consts::OS.as_bytes());
    hasher.update([0]);
    hasher.update(identity.app_id.as_bytes());
    hasher.update([0]);
    hasher.update(executable.to_string_lossy().as_bytes());
    hasher.update([0]);
    // 启用集合也是注册结果的一部分：开关变化必须重跑一次。
    hasher.update(schemes.join(",").as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 记录「上次真正写进系统的 scheme」。关闭某个 scheme 时据此精确摘除自己
/// 写过的注册表键，而不是把同名键都删掉。
fn registered_url_schemes_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join(REGISTERED_URL_SCHEMES_FILE))
}

fn read_registered_url_schemes() -> Result<Option<Vec<String>>> {
    match fs::read_to_string(registered_url_schemes_path()?) {
        Ok(contents) => Ok(Some(
            contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save_registered_url_schemes(schemes: &[&str]) -> Result<()> {
    let contents = if schemes.is_empty() {
        String::new()
    } else {
        format!("{}\n", schemes.join("\n"))
    };
    write_if_changed(&registered_url_schemes_path()?, &contents)?;
    Ok(())
}

fn registration_stamp_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join(REGISTRATION_STAMP_FILE))
}

fn stamp_matches(executable: &Path, identity: &AppIdentity, schemes: &[&str]) -> Result<bool> {
    stamp_matches_at(&registration_stamp_path()?, executable, identity, schemes)
}

fn stamp_matches_at(
    path: &Path,
    executable: &Path,
    identity: &AppIdentity,
    schemes: &[&str],
) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(current) => Ok(current.trim() == registration_stamp(executable, identity, schemes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn save_registration_stamp(
    executable: &Path,
    identity: &AppIdentity,
    schemes: &[&str],
) -> Result<()> {
    write_if_changed(
        &registration_stamp_path()?,
        &format!("{}\n", registration_stamp(executable, identity, schemes)),
    )?;
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
struct WindowsPlanContext {
    open_command: String,
    default_icon: String,
    application_key: String,
    url_protocol_description: String,
}

#[cfg(any(target_os = "windows", test))]
fn windows_plan_context(executable: &Path, identity: &AppIdentity) -> WindowsPlanContext {
    let executable = executable.to_string_lossy();
    WindowsPlanContext {
        open_command: format!(r#""{executable}" "%1""#),
        default_icon: format!(r#""{executable}",0"#),
        application_key: format!(
            r"HKCU\Software\Classes\Applications\{}{}",
            identity.binary_name, WINDOWS_EXECUTABLE_SUFFIX
        ),
        url_protocol_description: format!("URL:{} protocol", identity.display_name),
    }
}

#[cfg(any(target_os = "windows", test))]
const WINDOWS_EXECUTABLE_SUFFIX: &str = ".exe";

#[cfg(any(target_os = "windows", test))]
fn windows_registry_plan(
    executable: &Path,
    identity: &AppIdentity,
    schemes: &[&str],
) -> Vec<RegistryCommand> {
    let context = windows_plan_context(executable, identity);
    let specs = associations_for(identity);
    // 既没有扩展名也没有 URL scheme 的产品不注册任何东西：只写 `Applications`
    // 键既没人用，又会在注册表里留下一个永远无法打开文件的项。
    if specs.is_empty() && schemes.is_empty() {
        return Vec::new();
    }
    let mut commands = Vec::new();

    for spec in &specs {
        append_windows_association_commands(&mut commands, &context, spec);
    }
    if !specs.is_empty() {
        append_windows_application_commands(&mut commands, &context, &specs);
    }
    for scheme in schemes {
        append_windows_url_protocol_commands(&mut commands, &context, scheme);
    }
    commands
}

/// `jms://` 这类深链接依赖的 URL Protocol 注册：`Software\Classes\<scheme>`
/// 下的默认值、空的 `URL Protocol`、`DefaultIcon` 与 `shell\open\command`。
#[cfg(any(target_os = "windows", test))]
fn append_windows_url_protocol_commands(
    commands: &mut Vec<RegistryCommand>,
    context: &WindowsPlanContext,
    scheme: &str,
) {
    let base = format!(r"HKCU\Software\Classes\{scheme}");
    for (key, value_name, data) in [
        (base.clone(), None, context.url_protocol_description.clone()),
        (
            base.clone(),
            Some("URL Protocol".to_string()),
            String::new(),
        ),
        (
            format!(r"{base}\DefaultIcon"),
            None,
            context.default_icon.clone(),
        ),
        (
            format!(r"{base}\shell\open\command"),
            None,
            context.open_command.clone(),
        ),
    ] {
        commands.push(RegistryCommand {
            key,
            value_name,
            value_type: "REG_SZ",
            data,
            policy: RegistryWritePolicy::Always,
        });
    }
}

#[cfg(any(target_os = "windows", test))]
fn append_windows_association_commands(
    commands: &mut Vec<RegistryCommand>,
    context: &WindowsPlanContext,
    association: &AssociationSpec,
) {
    let prog_id_key = format!(r"HKCU\Software\Classes\{}", association.prog_id);
    for (key, data) in [
        (prog_id_key.clone(), association.description.to_string()),
        (
            format!(r"{prog_id_key}\DefaultIcon"),
            context.default_icon.clone(),
        ),
        (
            format!(r"{prog_id_key}\shell\open\command"),
            context.open_command.clone(),
        ),
    ] {
        commands.push(RegistryCommand {
            key,
            value_name: None,
            value_type: "REG_SZ",
            data,
            policy: RegistryWritePolicy::Always,
        });
    }
    commands.push(RegistryCommand {
        key: format!(r"HKCU\Software\Classes\{}", association.extension),
        value_name: None,
        value_type: "REG_SZ",
        data: association.prog_id.clone(),
        policy: RegistryWritePolicy::IfNoDefaultAssociation {
            extension: association.extension,
        },
    });
    commands.push(RegistryCommand {
        key: format!(
            r"HKCU\Software\Classes\{}\OpenWithProgids",
            association.extension
        ),
        value_name: Some(association.prog_id.clone()),
        value_type: "REG_NONE",
        data: String::new(),
        policy: RegistryWritePolicy::Always,
    });
}

#[cfg(any(target_os = "windows", test))]
fn append_windows_application_commands(
    commands: &mut Vec<RegistryCommand>,
    context: &WindowsPlanContext,
    specs: &[AssociationSpec],
) {
    commands.push(RegistryCommand {
        key: format!(r"{}\shell\open\command", context.application_key),
        value_name: None,
        value_type: "REG_SZ",
        data: context.open_command.clone(),
        policy: RegistryWritePolicy::Always,
    });
    for association in specs {
        commands.push(RegistryCommand {
            key: format!(r"{}\SupportedTypes", context.application_key),
            value_name: Some(association.extension.to_string()),
            value_type: "REG_SZ",
            data: String::new(),
            policy: RegistryWritePolicy::Always,
        });
    }
}

#[cfg(target_os = "windows")]
fn ensure_windows_registered(
    executable: &Path,
    identity: &AppIdentity,
    schemes: &[&str],
) -> Result<()> {
    if stamp_matches(executable, identity, schemes)? {
        return Ok(());
    }
    for registry_command in windows_registry_plan(executable, identity, schemes) {
        if let RegistryWritePolicy::IfNoDefaultAssociation { extension } = registry_command.policy
            && windows_default_association_exists(extension)?
        {
            continue;
        }
        run_registry_command(&registry_command)?;
    }
    notify_windows_association_changed();
    save_registration_stamp(executable, identity, schemes)
}

/// 把一个之前由 Navop 写过的 scheme 键从用户注册表里删掉（含子键）。
#[cfg(target_os = "windows")]
fn remove_windows_url_protocol(scheme: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let key = format!(r"HKCU\Software\Classes\{scheme}");
    let status = Command::new("reg.exe")
        .arg("delete")
        .arg(&key)
        .arg("/f")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .context("delete registered URL protocol")?;
    if !status.success() {
        // 键可能本来就不存在（例如从未真正写入过），删除失败不算致命错误。
        tracing::debug!(%scheme, "reg.exe delete did not remove the URL protocol key");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_default_association_exists(extension: &str) -> Result<bool> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("reg.exe")
        .arg("query")
        .arg(format!(r"HKCR\{extension}"))
        .arg("/ve")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("query current Windows file association")?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(windows_registry_output_has_default(
        &String::from_utf8_lossy(&output.stdout),
    ))
}

#[cfg(any(target_os = "windows", test))]
fn windows_registry_output_has_default(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        ["REG_SZ", "REG_EXPAND_SZ"]
            .into_iter()
            .find_map(|value_type| line.split_once(value_type))
            .is_some_and(|(_, value)| !value.trim().is_empty())
    })
}

#[cfg(target_os = "windows")]
fn run_registry_command(plan: &RegistryCommand) -> Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("reg.exe");
    command.arg("add").arg(&plan.key).arg("/f");
    match &plan.value_name {
        Some(value_name) => {
            command.arg("/v").arg(value_name);
        }
        None => {
            command.arg("/ve");
        }
    }
    command
        .arg("/t")
        .arg(plan.value_type)
        .arg("/d")
        .arg(&plan.data)
        .creation_flags(CREATE_NO_WINDOW);
    let status = command
        .status()
        .context("run reg.exe for file associations")?;
    if !status.success() {
        bail!("reg.exe failed for {} with status {status}", plan.key);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn notify_windows_association_changed() {
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

    // SAFETY: ASSOCCHANGED with IDLIST carries no item pointers by contract.
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

#[cfg(any(target_os = "linux", test))]
fn linux_registration_files(
    executable: &Path,
    data_dir: &Path,
    identity: &AppIdentity,
    schemes: &[&str],
) -> LinuxRegistrationFiles {
    let applications = data_dir.join("applications");
    let desktop_name = format!("{}{LINUX_DESKTOP_SUFFIX}", identity.binary_name);
    let mime_name = format!("{}{LINUX_MIME_SUFFIX}", identity.binary_name);
    LinuxRegistrationFiles {
        desktop_path: applications.join(&desktop_name),
        desktop_contents: render_linux_desktop(executable, identity, schemes),
        mime_path: data_dir.join("mime").join("packages").join(mime_name),
        mime_contents: LINUX_MIME_TEMPLATE.to_string(),
    }
}

/// 把模板里的产品名、图标与启动命令换成本产品的值。
#[cfg(any(target_os = "linux", test))]
fn render_linux_desktop(executable: &Path, identity: &AppIdentity, schemes: &[&str]) -> String {
    let quoted = desktop_exec_quote(executable);
    let name = identity.display_name;
    let binary = identity.binary_name;
    let mime_types = linux_default_mime_types(identity, schemes);
    LINUX_DESKTOP_TEMPLATE
        .lines()
        .filter_map(|line| {
            if line.starts_with("Exec=") {
                // `%U` 才能把 `jms://` 与 `file://` 这类 URL 传给应用；`%F` 只给路径。
                Some(format!("Exec={quoted} %U"))
            } else if line.starts_with("Name=") {
                Some(format!("Name={name}"))
            } else if line.starts_with("Icon=") {
                Some(format!("Icon={binary}"))
            } else if line.starts_with("StartupWMClass=") {
                Some(format!("StartupWMClass={binary}"))
            } else if line.starts_with("MimeType=") {
                if mime_types.is_empty() {
                    None
                } else {
                    Some(format!("MimeType={};", mime_types.join(";")))
                }
            } else {
                Some(line.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[cfg(any(target_os = "linux", test))]
fn desktop_exec_quote(executable: &Path) -> String {
    let mut escaped = String::new();
    for character in executable.to_string_lossy().chars() {
        match character {
            '\\' | '"' | '`' | '$' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '%' => escaped.push_str("%%"),
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

/// 产品需要设为默认的 MIME 类型：文件关联的 MIME 加上每个 URL scheme 对应的
/// `x-scheme-handler/<scheme>`。
#[cfg(any(target_os = "linux", test))]
fn linux_default_mime_types(identity: &AppIdentity, schemes: &[&str]) -> Vec<String> {
    let mut mime_types = identity
        .file_associations
        .iter()
        .filter_map(|declared| association_kind(declared))
        .flat_map(|kind| kind.mime_types.iter().copied())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for scheme in schemes {
        mime_types.push(format!("x-scheme-handler/{scheme}"));
    }
    mime_types
}

#[cfg(target_os = "linux")]
fn ensure_linux_registered(
    executable: &Path,
    identity: &AppIdentity,
    schemes: &[&str],
) -> Result<()> {
    let data_dir = dirs::data_local_dir().context("resolve user data directory")?;
    let files = linux_registration_files(executable, &data_dir, identity, schemes);
    let desktop_changed = write_if_changed(&files.desktop_path, &files.desktop_contents)?;
    let mime_changed = write_if_changed(&files.mime_path, &files.mime_contents)?;
    let stamp_current = stamp_matches(executable, identity, schemes)?;

    if mime_changed || !stamp_current {
        run_optional_refresh("update-mime-database", &data_dir.join("mime"))?;
    }
    if desktop_changed || !stamp_current {
        run_optional_refresh("update-desktop-database", &data_dir.join("applications"))?;
    }
    if !stamp_current {
        let desktop_name = format!("{}{LINUX_DESKTOP_SUFFIX}", identity.binary_name);
        for mime_type in linux_default_mime_types(identity, schemes) {
            ensure_linux_default_if_missing(&mime_type, &desktop_name)?;
        }
    }
    save_registration_stamp(executable, identity, schemes)
}

#[cfg(target_os = "linux")]
fn ensure_linux_default_if_missing(mime_type: &str, desktop_name: &str) -> Result<()> {
    let query = match Command::new("xdg-mime")
        .args(["query", "default", mime_type])
        .output()
    {
        Ok(query) => query,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!("xdg-mime is unavailable; skipping default association");
            return Ok(());
        }
        Err(error) => return Err(error).context("query default MIME application"),
    };
    if !query.status.success() {
        bail!("xdg-mime query failed for {mime_type}: {}", query.status);
    }
    if !linux_default_query_is_empty(&String::from_utf8_lossy(&query.stdout)) {
        return Ok(());
    }
    let status = Command::new("xdg-mime")
        .args(["default", desktop_name, mime_type])
        .status()
        .context("set default MIME application")?;
    if !status.success() {
        bail!("xdg-mime default failed for {mime_type}: {status}");
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn linux_default_query_is_empty(stdout: &str) -> bool {
    stdout.trim().is_empty()
}

#[cfg(target_os = "linux")]
fn run_optional_refresh(program: &str, directory: &Path) -> Result<()> {
    match Command::new(program).arg(directory).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!("{program} failed with status {status}"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(program, "association refresh command is unavailable");
            Ok(())
        }
        Err(error) => Err(error).with_context(|| format!("run {program}")),
    }
}

#[cfg(any(target_os = "macos", test))]
fn macos_app_bundle_from_executable(executable: &Path) -> Result<PathBuf> {
    let macos_dir = executable
        .parent()
        .context("current executable has no parent directory")?;
    if macos_dir.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        bail!("current executable is not inside an app MacOS directory");
    }
    let contents_dir = macos_dir
        .parent()
        .context("current executable has no Contents directory")?;
    if contents_dir.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        bail!("current executable is not inside an app Contents directory");
    }
    let app_bundle = contents_dir.parent().context("app bundle is missing")?;
    if app_bundle
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("app")
    {
        bail!("current executable is not inside an app bundle");
    }
    Ok(app_bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
fn ensure_macos_registered(
    executable: &Path,
    identity: &AppIdentity,
    schemes: &[&str],
) -> Result<()> {
    if stamp_matches(executable, identity, schemes)? {
        return Ok(());
    }
    let app_bundle = match macos_app_bundle_from_executable(executable) {
        Ok(app_bundle) => app_bundle,
        Err(error) => {
            tracing::debug!(%error, "skipping LaunchServices registration outside an app bundle");
            return Ok(());
        }
    };
    const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let status = Command::new(LSREGISTER)
        .arg("-f")
        .arg(&app_bundle)
        .status()
        .context("refresh LaunchServices file associations")?;
    if !status.success() {
        bail!("lsregister failed with status {status}");
    }
    save_registration_stamp(executable, identity, schemes)
}

fn write_if_changed(path: &Path, contents: &str) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(current) if current == contents => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create association directory {}", parent.display()))?;
    }
    fs::write(path, contents)
        .with_context(|| format!("write association file {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const FULL_PROG_IDS: [&str; 4] = [
        "Navop.SQLiteDatabase",
        "Navop.DuckDBDatabase",
        "Navop.MarkdownDocument",
        "Navop.TerminalRecording",
    ];

    #[test]
    fn full_product_claims_its_own_extensions() {
        let specs = associations_for(&AppIdentity::FULL);
        assert_eq!(
            vec![".db", ".duckdb", ".md", ".cast"],
            specs.iter().map(|spec| spec.extension).collect::<Vec<_>>()
        );
        for spec in &specs {
            assert!(spec.prog_id.starts_with("Navop."), "{}", spec.prog_id);
        }
        assert_eq!(
            FULL_PROG_IDS
                .iter()
                .map(|prog_id| prog_id.to_string())
                .collect::<Vec<_>>(),
            specs
                .iter()
                .map(|spec| spec.prog_id.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn standalone_products_claim_no_extensions() {
        assert!(associations_for(&AppIdentity::SHELL).is_empty());
        assert!(associations_for(&AppIdentity::DATA).is_empty());
        assert!(linux_default_mime_types(&AppIdentity::SHELL, &[]).is_empty());
        assert_eq!(["jms", "ssh", "sftp"], AppIdentity::FULL.url_schemes);
        assert!(AppIdentity::SHELL.url_schemes.is_empty());
        assert!(AppIdentity::DATA.url_schemes.is_empty());
    }

    #[test]
    fn prog_id_prefix_derives_from_the_app_id() {
        let shell = AppIdentity {
            app_id: "com.navop.shell",
            ..AppIdentity::SHELL
        };
        assert_eq!("NavopShell", prog_id_prefix(&shell));
        assert_eq!("Navop", prog_id_prefix(&AppIdentity::FULL));
        assert_eq!("NavopData", prog_id_prefix(&AppIdentity::DATA));
    }

    #[test]
    fn windows_registry_plan_registers_handlers_without_overwriting_user_choice() {
        let executable = Path::new(r"C:\Program Files\Navop\navop.exe");
        let commands = windows_registry_plan(executable, &AppIdentity::FULL, &["jms", "ssh"]);
        let rendered = commands
            .iter()
            .map(|command| format!("{} {:?} {}", command.key, command.value_name, command.data))
            .collect::<Vec<_>>()
            .join("\n");

        for (extension, prog_id) in [
            (".db", "Navop.SQLiteDatabase"),
            (".duckdb", "Navop.DuckDBDatabase"),
            (".md", "Navop.MarkdownDocument"),
            (".cast", "Navop.TerminalRecording"),
        ] {
            assert!(rendered.contains(extension));
            assert!(rendered.contains(prog_id));
        }
        assert!(rendered.contains(r#""C:\Program Files\Navop\navop.exe" "%1""#));
        assert!(rendered.contains(r"Applications\navop.exe"));
        assert!(rendered.contains(r"Software\Classes\jms"));
        assert!(rendered.contains("URL Protocol"));
        assert!(!rendered.contains("UserChoice"));
        assert!(!rendered.contains(".partial"));
        for (extension, prog_id) in [
            (".db", "Navop.SQLiteDatabase"),
            (".duckdb", "Navop.DuckDBDatabase"),
            (".md", "Navop.MarkdownDocument"),
            (".cast", "Navop.TerminalRecording"),
        ] {
            assert!(commands.iter().any(|command| {
                command.key.ends_with(extension)
                    && command.value_name.is_none()
                    && command.data == prog_id
                    && matches!(
                        command.policy,
                        RegistryWritePolicy::IfNoDefaultAssociation { extension: current }
                            if current == extension
                    )
            }));
        }
    }

    #[test]
    fn windows_plan_is_empty_for_products_without_extensions() {
        assert!(
            windows_registry_plan(
                Path::new(r"C:\Navop Shell\navop-shell.exe"),
                &AppIdentity::SHELL,
                &[]
            )
            .is_empty()
        );
    }

    #[test]
    fn linux_registration_uses_user_data_directories_and_absolute_executable() {
        let files = linux_registration_files(
            Path::new("/opt/Navop 100%/navop"),
            Path::new("/home/alice/.local/share"),
            &AppIdentity::FULL,
            &["jms", "ssh", "sftp"],
        );

        assert_eq!(
            PathBuf::from("/home/alice/.local/share/applications/navop.desktop"),
            files.desktop_path
        );
        assert_eq!(
            PathBuf::from("/home/alice/.local/share/mime/packages/navop.xml"),
            files.mime_path
        );
        assert!(
            files
                .desktop_contents
                .contains(r#"Exec="/opt/Navop 100%%/navop" %U"#)
        );
        assert!(files.desktop_contents.contains("Name=Navop"));
        assert!(
            files
                .desktop_contents
                .contains("MimeType=application/vnd.sqlite3")
        );
        assert!(files.desktop_contents.contains("application/x-asciicast"));
        assert!(files.desktop_contents.contains("x-scheme-handler/jms;"));
        assert!(files.mime_contents.contains("<glob pattern=\"*.duckdb\"/>"));
        assert!(files.mime_contents.contains("<glob pattern=\"*.md\"/>"));
        assert!(files.mime_contents.contains("<glob pattern=\"*.cast\"/>"));
        assert!(
            files
                .mime_contents
                .contains("<glob pattern=\"*.cast.partial\"/>")
        );
        assert!(
            !files
                .mime_contents
                .contains("<glob pattern=\"*.partial\"/>")
        );
        assert_eq!(
            vec![
                "application/vnd.sqlite3",
                "application/x-sqlite3",
                "application/x-duckdb",
                "application/vnd.duckdb",
                "text/markdown",
                "application/x-asciicast",
                "x-scheme-handler/jms",
            ],
            linux_default_mime_types(&AppIdentity::FULL, &["jms"])
        );
    }

    #[test]
    fn linux_registration_file_names_follow_the_binary_name() {
        let custom = AppIdentity {
            binary_name: "navop-data",
            file_associations: &["cast"],
            ..AppIdentity::DATA
        };
        let files = linux_registration_files(
            Path::new("/opt/Navop Data/navop-data"),
            Path::new("/home/alice/.local/share"),
            &custom,
            &[],
        );

        assert_eq!(
            PathBuf::from("/home/alice/.local/share/applications/navop-data.desktop"),
            files.desktop_path
        );
        assert_eq!(
            PathBuf::from("/home/alice/.local/share/mime/packages/navop-data.xml"),
            files.mime_path
        );
        assert!(files.desktop_contents.contains("Name=Navop Data"));
        assert!(files.desktop_contents.contains("Icon=navop-data"));
        assert_eq!(
            vec!["application/x-asciicast"],
            linux_default_mime_types(&custom, &[])
        );
        assert!(!files.desktop_contents.contains("x-scheme-handler/"));
    }

    #[test]
    fn registration_schema_covers_url_schemes() {
        assert_eq!(3, ASSOCIATION_SCHEMA_VERSION);
        assert_eq!("file-association-registration-v3", REGISTRATION_STAMP_FILE);
    }

    #[test]
    fn macos_bundle_is_derived_from_the_current_executable() {
        assert_eq!(
            PathBuf::from("/Applications/Navop.app"),
            macos_app_bundle_from_executable(Path::new(
                "/Applications/Navop.app/Contents/MacOS/navop"
            ))
            .unwrap()
        );
        assert!(macos_app_bundle_from_executable(Path::new("/tmp/target/debug/navop")).is_err());
    }

    #[test]
    fn registration_stamp_changes_when_the_executable_moves() {
        let identity = &AppIdentity::FULL;
        assert_eq!(
            registration_stamp(
                Path::new("/Applications/Navop.app/Contents/MacOS/navop"),
                identity,
                &["jms"]
            ),
            registration_stamp(
                Path::new("/Applications/Navop.app/Contents/MacOS/navop"),
                identity,
                &["jms"]
            )
        );
        assert_ne!(
            registration_stamp(
                Path::new("/Applications/Navop.app/Contents/MacOS/navop"),
                identity,
                &["jms"]
            ),
            registration_stamp(
                Path::new("/Users/alice/Applications/Navop.app/Contents/MacOS/navop"),
                identity,
                &["jms"]
            )
        );
        // 启用集合变化也必须触发一次重新注册。
        assert_ne!(
            registration_stamp(
                Path::new("/Applications/Navop.app/Contents/MacOS/navop"),
                identity,
                &["jms"]
            ),
            registration_stamp(
                Path::new("/Applications/Navop.app/Contents/MacOS/navop"),
                identity,
                &["jms", "ssh"]
            )
        );
    }

    #[test]
    fn registration_stamp_skips_repeated_migrations() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let stamp_path = temp.path().join("association-stamp");
        let executable = Path::new("/Applications/Navop.app/Contents/MacOS/navop");
        let identity = &AppIdentity::FULL;

        assert!(!stamp_matches_at(
            &stamp_path,
            executable,
            identity,
            &["jms"]
        )?);
        std::fs::write(
            &stamp_path,
            format!("{}\n", registration_stamp(executable, identity, &["jms"])),
        )?;
        assert!(stamp_matches_at(
            &stamp_path,
            executable,
            identity,
            &["jms"]
        )?);
        assert!(!stamp_matches_at(
            &stamp_path,
            Path::new("/Applications/Navop 2.app/Contents/MacOS/navop"),
            identity,
            &["jms"]
        )?);
        assert!(!stamp_matches_at(
            &stamp_path,
            executable,
            identity,
            &["jms", "ssh"]
        )?);
        Ok(())
    }

    #[test]
    fn default_association_detection_preserves_existing_choices() {
        assert!(windows_registry_output_has_default(
            "    (Default)    REG_SZ    VisualStudioCode.md\n"
        ));
        assert!(!windows_registry_output_has_default(
            "    (Default)    REG_SZ    \n"
        ));
        assert!(!linux_default_query_is_empty(
            "org.gnome.TextEditor.desktop\n"
        ));
        assert!(linux_default_query_is_empty("\n"));
    }

    #[test]
    fn write_if_changed_is_idempotent() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("nested/association.txt");

        assert!(write_if_changed(&path, "first")?);
        assert!(!write_if_changed(&path, "first")?);
        assert!(write_if_changed(&path, "second")?);
        assert_eq!("second", std::fs::read_to_string(path)?);
        Ok(())
    }
}
