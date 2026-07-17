//! Repairs per-user file associations after binary-only application updates.

use anyhow::{Context as _, Result, bail};
use gpui::{App, AppContext};
use one_core::storage::manager::get_config_dir;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ASSOCIATION_SCHEMA_VERSION: u32 = 1;
const REGISTRATION_STAMP_FILE: &str = "file-association-registration-v1";
#[cfg(any(target_os = "linux", test))]
const LINUX_DESKTOP_TEMPLATE: &str = include_str!("../../resources/linux/navop.desktop");
#[cfg(any(target_os = "linux", test))]
const LINUX_MIME_TEMPLATE: &str = include_str!("../../resources/linux/navop.xml");

#[cfg(any(target_os = "windows", test))]
const ASSOCIATIONS: &[AssociationSpec] = &[
    AssociationSpec {
        extension: ".db",
        prog_id: "Navop.SQLiteDatabase",
        description: "SQLite Database",
    },
    AssociationSpec {
        extension: ".duckdb",
        prog_id: "Navop.DuckDBDatabase",
        description: "DuckDB Database",
    },
    AssociationSpec {
        extension: ".md",
        prog_id: "Navop.MarkdownDocument",
        description: "Markdown Document",
    },
];

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy)]
struct AssociationSpec {
    extension: &'static str,
    prog_id: &'static str,
    description: &'static str,
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

pub(crate) fn schedule_registration(cx: &mut App) {
    cx.background_spawn(async move {
        if let Err(error) = ensure_registered() {
            tracing::warn!(error = %error, "file association migration failed");
        }
    })
    .detach();
}

fn ensure_registered() -> Result<()> {
    let executable = std::env::current_exe().context("resolve current executable")?;

    #[cfg(target_os = "windows")]
    return ensure_windows_registered(&executable);

    #[cfg(target_os = "linux")]
    return ensure_linux_registered(&executable);

    #[cfg(target_os = "macos")]
    return ensure_macos_registered(&executable);

    #[allow(unreachable_code)]
    Ok(())
}

fn registration_stamp(executable: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(ASSOCIATION_SCHEMA_VERSION.to_le_bytes());
    hasher.update(std::env::consts::OS.as_bytes());
    hasher.update([0]);
    hasher.update(executable.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn registration_stamp_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join(REGISTRATION_STAMP_FILE))
}

fn stamp_matches(executable: &Path) -> Result<bool> {
    stamp_matches_at(&registration_stamp_path()?, executable)
}

fn stamp_matches_at(path: &Path, executable: &Path) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(current) => Ok(current.trim() == registration_stamp(executable)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn save_registration_stamp(executable: &Path) -> Result<()> {
    write_if_changed(
        &registration_stamp_path()?,
        &format!("{}\n", registration_stamp(executable)),
    )?;
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn windows_registry_plan(executable: &Path) -> Vec<RegistryCommand> {
    let executable = executable.to_string_lossy();
    let open_command = format!(r#""{executable}" "%1""#);
    let default_icon = format!(r#""{executable}",0"#);
    let mut commands = Vec::new();

    for association in ASSOCIATIONS {
        append_windows_association_commands(
            &mut commands,
            association,
            &open_command,
            &default_icon,
        );
    }

    append_windows_application_commands(&mut commands, open_command);
    commands
}

#[cfg(any(target_os = "windows", test))]
fn append_windows_association_commands(
    commands: &mut Vec<RegistryCommand>,
    association: &AssociationSpec,
    open_command: &str,
    default_icon: &str,
) {
    let prog_id_key = format!(r"HKCU\Software\Classes\{}", association.prog_id);
    for (key, data) in [
        (prog_id_key.clone(), association.description.to_string()),
        (
            format!(r"{prog_id_key}\DefaultIcon"),
            default_icon.to_string(),
        ),
        (
            format!(r"{prog_id_key}\shell\open\command"),
            open_command.to_string(),
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
        data: association.prog_id.to_string(),
        policy: RegistryWritePolicy::IfNoDefaultAssociation {
            extension: association.extension,
        },
    });
    commands.push(RegistryCommand {
        key: format!(
            r"HKCU\Software\Classes\{}\OpenWithProgids",
            association.extension
        ),
        value_name: Some(association.prog_id.to_string()),
        value_type: "REG_NONE",
        data: String::new(),
        policy: RegistryWritePolicy::Always,
    });
}

#[cfg(any(target_os = "windows", test))]
fn append_windows_application_commands(commands: &mut Vec<RegistryCommand>, open_command: String) {
    let application_key = r"HKCU\Software\Classes\Applications\navop.exe";
    commands.push(RegistryCommand {
        key: format!(r"{application_key}\shell\open\command"),
        value_name: None,
        value_type: "REG_SZ",
        data: open_command,
        policy: RegistryWritePolicy::Always,
    });
    for association in ASSOCIATIONS {
        commands.push(RegistryCommand {
            key: format!(r"{application_key}\SupportedTypes"),
            value_name: Some(association.extension.to_string()),
            value_type: "REG_SZ",
            data: String::new(),
            policy: RegistryWritePolicy::Always,
        });
    }
}

#[cfg(target_os = "windows")]
fn ensure_windows_registered(executable: &Path) -> Result<()> {
    if stamp_matches(executable)? {
        return Ok(());
    }
    for registry_command in windows_registry_plan(executable) {
        if let RegistryWritePolicy::IfNoDefaultAssociation { extension } = registry_command.policy
            && windows_default_association_exists(extension)?
        {
            continue;
        }
        run_registry_command(&registry_command)?;
    }
    notify_windows_association_changed();
    save_registration_stamp(executable)
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
fn linux_registration_files(executable: &Path, data_dir: &Path) -> LinuxRegistrationFiles {
    let desktop_path = data_dir.join("applications").join("navop.desktop");
    let mime_path = data_dir.join("mime").join("packages").join("navop.xml");
    let executable = desktop_exec_quote(executable);
    let desktop_contents = LINUX_DESKTOP_TEMPLATE
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                format!("Exec={executable} %F")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    LinuxRegistrationFiles {
        desktop_path,
        desktop_contents,
        mime_path,
        mime_contents: LINUX_MIME_TEMPLATE.to_string(),
    }
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
fn linux_default_mime_types() -> Vec<&'static str> {
    vec![
        "application/vnd.sqlite3",
        "application/x-sqlite3",
        "application/x-duckdb",
        "application/vnd.duckdb",
        "text/markdown",
    ]
}

#[cfg(target_os = "linux")]
fn ensure_linux_registered(executable: &Path) -> Result<()> {
    let data_dir = dirs::data_local_dir().context("resolve user data directory")?;
    let files = linux_registration_files(executable, &data_dir);
    let desktop_changed = write_if_changed(&files.desktop_path, &files.desktop_contents)?;
    let mime_changed = write_if_changed(&files.mime_path, &files.mime_contents)?;
    let stamp_current = stamp_matches(executable)?;

    if mime_changed || !stamp_current {
        run_optional_refresh("update-mime-database", &data_dir.join("mime"))?;
    }
    if desktop_changed || !stamp_current {
        run_optional_refresh("update-desktop-database", &data_dir.join("applications"))?;
    }
    if !stamp_current {
        for mime_type in linux_default_mime_types() {
            ensure_linux_default_if_missing(mime_type)?;
        }
    }
    save_registration_stamp(executable)
}

#[cfg(target_os = "linux")]
fn ensure_linux_default_if_missing(mime_type: &str) -> Result<()> {
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
        .args(["default", "navop.desktop", mime_type])
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
fn ensure_macos_registered(executable: &Path) -> Result<()> {
    if stamp_matches(executable)? {
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
    save_registration_stamp(executable)
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

    #[test]
    fn windows_registry_plan_registers_handlers_without_overwriting_user_choice() {
        let executable = Path::new(r"C:\Program Files\Navop\navop.exe");
        let commands = windows_registry_plan(executable);
        let rendered = commands
            .iter()
            .map(|command| format!("{} {:?} {}", command.key, command.value_name, command.data))
            .collect::<Vec<_>>()
            .join("\n");

        for (extension, prog_id) in [
            (".db", "Navop.SQLiteDatabase"),
            (".duckdb", "Navop.DuckDBDatabase"),
            (".md", "Navop.MarkdownDocument"),
        ] {
            assert!(rendered.contains(extension));
            assert!(rendered.contains(prog_id));
        }
        assert!(rendered.contains(r#""C:\Program Files\Navop\navop.exe" "%1""#));
        assert!(!rendered.contains("UserChoice"));
        for (extension, prog_id) in [
            (".db", "Navop.SQLiteDatabase"),
            (".duckdb", "Navop.DuckDBDatabase"),
            (".md", "Navop.MarkdownDocument"),
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
    fn linux_registration_uses_user_data_directories_and_absolute_executable() {
        let files = linux_registration_files(
            Path::new("/opt/Navop 100%/navop"),
            Path::new("/home/alice/.local/share"),
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
        assert!(
            files
                .desktop_contents
                .contains("MimeType=application/vnd.sqlite3")
        );
        assert!(files.mime_contents.contains("<glob pattern=\"*.duckdb\"/>"));
        assert!(files.mime_contents.contains("<glob pattern=\"*.md\"/>"));
        assert_eq!(
            vec![
                "application/vnd.sqlite3",
                "application/x-sqlite3",
                "application/x-duckdb",
                "application/vnd.duckdb",
                "text/markdown",
            ],
            linux_default_mime_types()
        );
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
        assert_eq!(
            registration_stamp(Path::new("/Applications/Navop.app/Contents/MacOS/navop")),
            registration_stamp(Path::new("/Applications/Navop.app/Contents/MacOS/navop"))
        );
        assert_ne!(
            registration_stamp(Path::new("/Applications/Navop.app/Contents/MacOS/navop")),
            registration_stamp(Path::new(
                "/Users/alice/Applications/Navop.app/Contents/MacOS/navop"
            ))
        );
    }

    #[test]
    fn registration_stamp_skips_repeated_migrations() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let stamp_path = temp.path().join("association-stamp");
        let executable = Path::new("/Applications/Navop.app/Contents/MacOS/navop");

        assert!(!stamp_matches_at(&stamp_path, executable)?);
        std::fs::write(&stamp_path, format!("{}\n", registration_stamp(executable)))?;
        assert!(stamp_matches_at(&stamp_path, executable)?);
        assert!(!stamp_matches_at(
            &stamp_path,
            Path::new("/Applications/Navop 2.app/Contents/MacOS/navop")
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
