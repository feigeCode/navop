use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use extension_runtime::extension::manifest::RemoteFileEditorLaunchMode;
use one_core::settings::RemoteFileEditorOverride;

const MACOS_OPEN_PROGRAM: &str = "/usr/bin/open";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTemplateContext {
    pub file: String,
    pub remote_path: String,
    pub name: String,
}

pub fn render_args(args: &[String], context: &LaunchTemplateContext) -> Vec<String> {
    let templates = if args.is_empty() {
        &["{file}".to_string()][..]
    } else {
        args
    };
    templates
        .iter()
        .map(|arg| {
            arg.replace("{file}", &context.file)
                .replace("{remote_path}", &context.remote_path)
                .replace("{name}", &context.name)
        })
        .collect()
}

pub fn validate_program(program: &str) -> Result<()> {
    if program.trim().is_empty() {
        bail!("external editor program is empty");
    }
    Ok(())
}

pub fn resolve_program_with_env(
    candidates: &[String],
    program_override: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some(program) = program_override.filter(|value| !value.trim().is_empty()) {
        return Some(program.to_string());
    }
    candidates
        .iter()
        .map(|candidate| expand_env_variables(candidate, &env))
        .find(|candidate| !candidate.trim().is_empty())
}

pub fn resolve_editor_program(
    candidates: &[String],
    editor_override: Option<&RemoteFileEditorOverride>,
) -> Option<String> {
    let program_override = editor_override.map(|value| value.program.as_str());
    let expanded = resolve_program_with_env(candidates, program_override, |name| {
        std::env::var(name).ok()
    })?;
    if program_override.is_some()
        || is_bare_program(&expanded)
        || std::path::Path::new(&expanded).is_file()
    {
        Some(expanded)
    } else {
        candidates.iter().find_map(|candidate| {
            let expanded = expand_env_variables(candidate, &|name| std::env::var(name).ok());
            std::path::Path::new(&expanded)
                .is_file()
                .then_some(expanded)
        })
    }
}

fn expand_env_variables(candidate: &str, env: &impl Fn(&str) -> Option<String>) -> String {
    let mut expanded = candidate.to_string();
    for name in ["ProgramFiles", "ProgramFiles(x86)"] {
        let placeholder = format!("${{env:{name}}}");
        if expanded.contains(&placeholder) {
            expanded = expanded.replace(&placeholder, env(name).as_deref().unwrap_or(""));
        }
    }
    expanded
}

fn is_bare_program(program: &str) -> bool {
    !program.contains('/') && !program.contains('\\')
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalLaunchCommand {
    program: String,
    args: Vec<String>,
}

fn macos_app_bundle(program: &str) -> Result<PathBuf> {
    Path::new(program)
        .ancestors()
        .find(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("macOS editor executable is not inside an .app bundle"))
}

fn plan_external_launch_for_os(
    os: &str,
    program: &str,
    args: &[String],
    launch_mode: RemoteFileEditorLaunchMode,
) -> Result<ExternalLaunchCommand> {
    validate_program(program)?;
    match launch_mode {
        RemoteFileEditorLaunchMode::Direct => Ok(ExternalLaunchCommand {
            program: program.to_string(),
            args: args.to_vec(),
        }),
        RemoteFileEditorLaunchMode::MacosOpen if os == "macos" => {
            let bundle = macos_app_bundle(program)?;
            let mut launch_args = vec!["-a".to_string(), bundle.to_string_lossy().into_owned()];
            launch_args.extend_from_slice(args);
            Ok(ExternalLaunchCommand {
                program: MACOS_OPEN_PROGRAM.to_string(),
                args: launch_args,
            })
        }
        RemoteFileEditorLaunchMode::MacosOpen => {
            bail!("macOS open launch mode is only supported on macOS")
        }
    }
}

pub fn launch_external_editor(
    program: &str,
    args: &[String],
    launch_mode: RemoteFileEditorLaunchMode,
) -> Result<()> {
    let plan = plan_external_launch_for_os(std::env::consts::OS, program, args, launch_mode)?;
    let mut command = std::process::Command::new(&plan.program);
    command.args(&plan.args);
    process_util::configure_background_child(&mut command);
    command.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use extension_runtime::extension::manifest::RemoteFileEditorLaunchMode;

    use super::{
        LaunchTemplateContext, macos_app_bundle, plan_external_launch_for_os, render_args,
        resolve_program_with_env, validate_program,
    };

    #[test]
    fn renders_supported_argument_templates() {
        let args = render_args(
            &[
                "--reuse-window".to_string(),
                "{file}".to_string(),
                "{remote_path}".to_string(),
                "{name}".to_string(),
            ],
            &LaunchTemplateContext {
                file: "/tmp/edit/app.conf".to_string(),
                remote_path: "/etc/app.conf".to_string(),
                name: "app.conf".to_string(),
            },
        );

        assert_eq!(
            vec![
                "--reuse-window",
                "/tmp/edit/app.conf",
                "/etc/app.conf",
                "app.conf"
            ],
            args
        );
    }

    #[test]
    fn rejects_empty_program() {
        let error = validate_program("  ").expect_err("empty program should fail");

        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn program_override_wins_over_manifest_candidates() {
        let program = resolve_program_with_env(
            &["manifest-editor".to_string()],
            Some("/custom/editor"),
            |_| None,
        );

        assert_eq!(Some("/custom/editor".to_string()), program);
    }

    #[test]
    fn program_candidates_expand_supported_environment_variables() {
        let program = resolve_program_with_env(
            &["${env:ProgramFiles}/Editor/editor.exe".to_string()],
            None,
            |name| (name == "ProgramFiles").then(|| "C:/Program Files".to_string()),
        );

        assert_eq!(
            Some("C:/Program Files/Editor/editor.exe".to_string()),
            program
        );
    }

    #[test]
    fn direct_launch_plan_preserves_program_and_args() {
        let plan = plan_external_launch_for_os(
            "linux",
            "/usr/bin/zed",
            &["/tmp/app.conf".to_string()],
            RemoteFileEditorLaunchMode::Direct,
        )
        .unwrap();

        assert_eq!("/usr/bin/zed", plan.program);
        assert_eq!(vec!["/tmp/app.conf"], plan.args);
    }

    #[test]
    fn macos_app_bundle_is_derived_from_editor_executable() {
        assert_eq!(
            std::path::PathBuf::from("/Applications/Notepad--.app"),
            macos_app_bundle("/Applications/Notepad--.app/Contents/MacOS/Notepad--").unwrap()
        );
        assert_eq!(
            std::path::PathBuf::from("/Applications/Zed.app"),
            macos_app_bundle("/Applications/Zed.app/Contents/MacOS/zed").unwrap()
        );
    }

    #[test]
    fn macos_app_bundle_rejects_non_bundle_executable() {
        let error = macos_app_bundle("/usr/local/bin/editor").unwrap_err();

        assert!(error.to_string().contains(".app"));
    }

    #[test]
    fn macos_open_launch_plan_uses_launch_services_without_shell() {
        let plan = plan_external_launch_for_os(
            "macos",
            "/Applications/Notepad--.app/Contents/MacOS/Notepad--",
            &["/tmp/remote file.html".to_string()],
            RemoteFileEditorLaunchMode::MacosOpen,
        )
        .unwrap();

        assert_eq!("/usr/bin/open", plan.program);
        assert_eq!(
            vec!["-a", "/Applications/Notepad--.app", "/tmp/remote file.html"],
            plan.args
        );
    }

    #[test]
    fn macos_open_launch_mode_rejects_other_platforms() {
        let error = plan_external_launch_for_os(
            "linux",
            "/Applications/Zed.app/Contents/MacOS/zed",
            &["/tmp/app.rs".to_string()],
            RemoteFileEditorLaunchMode::MacosOpen,
        )
        .unwrap_err();

        assert!(error.to_string().contains("macOS"));
    }
}
