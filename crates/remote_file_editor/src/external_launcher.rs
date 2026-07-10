use anyhow::{Result, bail};
use one_core::settings::RemoteFileEditorOverride;

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

pub fn launch_external_editor(program: &str, args: &[String]) -> Result<()> {
    validate_program(program)?;
    let mut command = std::process::Command::new(program);
    command.args(args);
    process_util::configure_background_child(&mut command);
    command.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{LaunchTemplateContext, render_args, resolve_program_with_env, validate_program};

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
}
