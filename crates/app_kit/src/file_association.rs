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

const ASSOCIATION_SCHEMA_VERSION: u32 = 2;
const REGISTRATION_STAMP_FILE: &str = "file-association-registration-v2";
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

/// 注册当前产品的文件关联；没有扩展名的产品直接跳过。
pub fn schedule_registration(cx: &mut App) {
    let Some(identity) = crate::identity() else {
        tracing::debug!("application identity is not configured; skipping file associations");
        return;
    };
    if associations_for(identity).is_empty() {
        tracing::debug!("product declares no file associations; skipping registration");
        return;
    }
    cx.background_spawn(async move {
        if let Err(error) = ensure_registered(identity) {
            tracing::warn!(error = %error, "file association migration failed");
        }
    })
    .detach();
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

fn ensure_registered(identity: &'static AppIdentity) -> Result<()> {
    if associations_for(identity).is_empty() {
        return Ok(());
    }
    let executable = std::env::current_exe().context("resolve current executable")?;

    #[cfg(target_os = "windows")]
    return ensure_windows_registered(&executable, identity);

    #[cfg(target_os = "linux")]
    return ensure_linux_registered(&executable, identity);

    #[cfg(target_os = "macos")]
    return ensure_macos_registered(&executable, identity);

    #[allow(unreachable_code)]
    Ok(())
}

fn registration_stamp(executable: &Path, identity: &AppIdentity) -> String {
    let mut hasher = Sha256::new();
    hasher.update(ASSOCIATION_SCHEMA_VERSION.to_le_bytes());
    hasher.update(std::env::consts::OS.as_bytes());
    hasher.update([0]);
    hasher.update(identity.app_id.as_bytes());
    hasher.update([0]);
    hasher.update(executable.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn registration_stamp_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join(REGISTRATION_STAMP_FILE))
}

fn stamp_matches(executable: &Path, identity: &AppIdentity) -> Result<bool> {
    stamp_matches_at(&registration_stamp_path()?, executable, identity)
}

fn stamp_matches_at(path: &Path, executable: &Path, identity: &AppIdentity) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(current) => Ok(current.trim() == registration_stamp(executable, identity)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn save_registration_stamp(executable: &Path, identity: &AppIdentity) -> Result<()> {
    write_if_changed(
        &registration_stamp_path()?,
        &format!("{}\n", registration_stamp(executable, identity)),
    )?;
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
struct WindowsPlanContext {
    open_command: String,
    default_icon: String,
    application_key: String,
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
    }
}

#[cfg(any(target_os = "windows", test))]
const WINDOWS_EXECUTABLE_SUFFIX: &str = ".exe";

#[cfg(any(target_os = "windows", test))]
fn windows_registry_plan(executable: &Path, identity: &AppIdentity) -> Vec<RegistryCommand> {
    let context = windows_plan_context(executable, identity);
    let specs = associations_for(identity);
    // 没有声明扩展名的产品不注册任何东西：只写 `Applications` 键既没人用，
    // 又会在注册表里留下一个永远无法打开文件的项。
    if specs.is_empty() {
        return Vec::new();
    }
    let mut commands = Vec::new();

    for spec in &specs {
        append_windows_association_commands(&mut commands, &context, spec);
    }
    append_windows_application_commands(&mut commands, &context, &specs);
    commands
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
fn ensure_windows_registered(executable: &Path, identity: &AppIdentity) -> Result<()> {
    if stamp_matches(executable, identity)? {
        return Ok(());
    }
    for registry_command in windows_registry_plan(executable, identity) {
        if let RegistryWritePolicy::IfNoDefaultAssociation { extension } = registry_command.policy
            && windows_default_association_exists(extension)?
        {
            continue;
        }
        run_registry_command(&registry_command)?;
    }
    notify_windows_association_changed();
    save_registration_stamp(executable, identity)
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
) -> LinuxRegistrationFiles {
    let applications = data_dir.join("applications");
    let desktop_name = format!("{}{LINUX_DESKTOP_SUFFIX}", identity.binary_name);
    let mime_name = format!("{}{LINUX_MIME_SUFFIX}", identity.binary_name);
    LinuxRegistrationFiles {
        desktop_path: applications.join(&desktop_name),
        desktop_contents: render_linux_desktop(executable, identity),
        mime_path: data_dir.join("mime").join("packages").join(mime_name),
        mime_contents: LINUX_MIME_TEMPLATE.to_string(),
    }
}

/// 把模板里的产品名、图标与启动命令换成本产品的值。
#[cfg(any(target_os = "linux", test))]
fn render_linux_desktop(executable: &Path, identity: &AppIdentity) -> String {
    let quoted = desktop_exec_quote(executable);
    let name = identity.display_name;
    let binary = identity.binary_name;
    LINUX_DESKTOP_TEMPLATE
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                format!("Exec={quoted} %F")
            } else if line.starts_with("Name=") {
                format!("Name={name}")
            } else if line.starts_with("Icon=") {
                format!("Icon={binary}")
            } else if line.starts_with("StartupWMClass=") {
                format!("StartupWMClass={binary}")
            } else {
                line.to_string()
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

#[cfg(any(target_os = "linux", test))]
fn linux_default_mime_types(identity: &AppIdentity) -> Vec<&'static str> {
    identity
        .file_associations
        .iter()
        .filter_map(|declared| association_kind(declared))
        .flat_map(|kind| kind.mime_types.iter().copied())
        .collect()
}

#[cfg(target_os = "linux")]
fn ensure_linux_registered(executable: &Path, identity: &AppIdentity) -> Result<()> {
    let data_dir = dirs::data_local_dir().context("resolve user data directory")?;
    let files = linux_registration_files(executable, &data_dir, identity);
    let desktop_changed = write_if_changed(&files.desktop_path, &files.desktop_contents)?;
    let mime_changed = write_if_changed(&files.mime_path, &files.mime_contents)?;
    let stamp_current = stamp_matches(executable, identity)?;

    if mime_changed || !stamp_current {
        run_optional_refresh("update-mime-database", &data_dir.join("mime"))?;
    }
    if desktop_changed || !stamp_current {
        run_optional_refresh("update-desktop-database", &data_dir.join("applications"))?;
    }
    if !stamp_current {
        let desktop_name = format!("{}{LINUX_DESKTOP_SUFFIX}", identity.binary_name);
        for mime_type in linux_default_mime_types(identity) {
            ensure_linux_default_if_missing(mime_type, &desktop_name)?;
        }
    }
    save_registration_stamp(executable, identity)
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
fn ensure_macos_registered(executable: &Path, identity: &AppIdentity) -> Result<()> {
    if stamp_matches(executable, identity)? {
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
    save_registration_stamp(executable, identity)
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
        assert!(linux_default_mime_types(&AppIdentity::SHELL).is_empty());
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
        let commands = windows_registry_plan(executable, &AppIdentity::FULL);
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
                &AppIdentity::SHELL
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
                .contains(r#"Exec="/opt/Navop 100%%/navop" %F"#)
        );
        assert!(files.desktop_contents.contains("Name=Navop"));
        assert!(
            files
                .desktop_contents
                .contains("MimeType=application/vnd.sqlite3")
        );
        assert!(files.desktop_contents.contains("application/x-asciicast"));
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
            ],
            linux_default_mime_types(&AppIdentity::FULL)
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
            linux_default_mime_types(&custom)
        );
    }

    #[test]
    fn terminal_recording_associations_bump_the_registration_schema() {
        assert_eq!(2, ASSOCIATION_SCHEMA_VERSION);
        assert_eq!("file-association-registration-v2", REGISTRATION_STAMP_FILE);
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
                identity
            ),
            registration_stamp(
                Path::new("/Applications/Navop.app/Contents/MacOS/navop"),
                identity
            )
        );
        assert_ne!(
            registration_stamp(
                Path::new("/Applications/Navop.app/Contents/MacOS/navop"),
                identity
            ),
            registration_stamp(
                Path::new("/Users/alice/Applications/Navop.app/Contents/MacOS/navop"),
                identity
            )
        );
    }

    #[test]
    fn registration_stamp_skips_repeated_migrations() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let stamp_path = temp.path().join("association-stamp");
        let executable = Path::new("/Applications/Navop.app/Contents/MacOS/navop");
        let identity = &AppIdentity::FULL;

        assert!(!stamp_matches_at(&stamp_path, executable, identity)?);
        std::fs::write(
            &stamp_path,
            format!("{}\n", registration_stamp(executable, identity)),
        )?;
        assert!(stamp_matches_at(&stamp_path, executable, identity)?);
        assert!(!stamp_matches_at(
            &stamp_path,
            Path::new("/Applications/Navop 2.app/Contents/MacOS/navop"),
            identity
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
