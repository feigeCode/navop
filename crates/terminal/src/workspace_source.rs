//! 终端会话 → 侧边栏文件树来源的解析策略。
//!
//! 侧边栏文件树需要回答「当前这个终端会话，文件系统在哪里」。不同会话类型
//! 的映射方式不同:
//!
//! - 本机终端:工作目录(缺省回落到 home)。
//! - WSL 会话:`wsl.exe -d <发行版>`,文件系统在 Windows 侧是 `\\wsl$\<发行版>`。
//! - 容器 exec:`docker exec <容器> <shell>`,文件系统在容器内,不是本机路径,
//!   浏览需要经 `docker exec` 后端(见 `workspace_explorer` 的容器后端)。
//!
//! 每个会话类型是一个 [`WorkspaceSourceResolver`] 策略,按 [`default_workspace_resolvers`]
//! 的顺序尝试;新增会话类型只需加一个 impl,不改调用方。

use std::path::PathBuf;

use crate::terminal::resolve_local_working_dir;
use crate::types::LocalConfig;
#[cfg(target_os = "windows")]
use crate::wsl_distributions::{wsl_distribution_for_config, wsl_unc_root};

/// 容器浏览所需的 docker 调用前缀(`program` + `exec` 之前的全局参数)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerInvocation {
    /// `docker` 可执行文件名或路径;也可能是扩展自带的 `docker-provider`。
    pub program: String,
    /// `exec` 之前的全局参数,如 `--context prod` / `--host tcp://...`。
    pub global_args: Vec<String>,
    /// 启动终端时的环境变量(含 `DOCKER_HOST` / `DOCKER_TLS_VERIFY` /
    /// `DOCKER_CERT_PATH`),浏览容器文件时要一并传给子进程,才能命中同一个 daemon。
    pub env: Vec<(String, String)>,
}

impl DockerInvocation {
    /// 拼出 `docker <global...> exec <容器> <命令...>` 的完整参数列表。
    pub fn exec_args(&self, container: &str, command: &[String]) -> Vec<String> {
        let mut args = self.global_args.clone();
        args.push("exec".to_string());
        args.push(container.to_string());
        args.extend(command.iter().cloned());
        args
    }
}

/// 侧边栏文件树的来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalWorkspaceSource {
    /// 本机可读路径(含 WSL 的 UNC 根)。
    Host { root: PathBuf },
    /// 容器文件系统:路径为容器内路径,经 `docker exec` 浏览。
    Container {
        docker: DockerInvocation,
        container: String,
    },
}

impl LocalWorkspaceSource {
    /// 文件树的初始根路径(容器为容器内 `/`)。
    pub fn root(&self) -> PathBuf {
        match self {
            LocalWorkspaceSource::Host { root } => root.clone(),
            LocalWorkspaceSource::Container { .. } => PathBuf::from("/"),
        }
    }

    /// 是否为容器来源(决定是否启用本机 git / gitignore 等能力)。
    pub fn is_container(&self) -> bool {
        matches!(self, LocalWorkspaceSource::Container { .. })
    }
}

/// 单个会话类型的解析策略。
pub trait WorkspaceSourceResolver {
    /// 命中该会话类型时返回来源;不适用返回 `None` 交给下一个策略。
    fn resolve(&self, config: &LocalConfig) -> Option<LocalWorkspaceSource>;
}

/// 容器 exec 会话:命中 `docker exec` 或 Navop provider 的 `exec` / `exec-bridge`。
pub struct DockerExecResolver;

/// WSL 会话:命中 `wsl.exe -d <发行版>`。
pub struct WslResolver;

/// 本机终端:兜底,永远命中(只要有工作目录可回落)。
pub struct HostResolver;

impl WorkspaceSourceResolver for DockerExecResolver {
    fn resolve(&self, config: &LocalConfig) -> Option<LocalWorkspaceSource> {
        let (docker, container) = docker_exec_invocation(config)?;
        Some(LocalWorkspaceSource::Container { docker, container })
    }
}

impl WorkspaceSourceResolver for WslResolver {
    fn resolve(&self, config: &LocalConfig) -> Option<LocalWorkspaceSource> {
        #[cfg(target_os = "windows")]
        {
            let root = wsl_distribution_for_config(config).and_then(wsl_unc_root)?;
            return Some(LocalWorkspaceSource::Host { root });
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = config;
            None
        }
    }
}

impl WorkspaceSourceResolver for HostResolver {
    fn resolve(&self, config: &LocalConfig) -> Option<LocalWorkspaceSource> {
        resolve_local_working_dir(config.working_dir.clone())
            .map(|root| LocalWorkspaceSource::Host { root })
    }
}

/// 默认策略顺序:容器 → WSL → 本机。
pub fn default_workspace_resolvers() -> &'static [&'static dyn WorkspaceSourceResolver] {
    &[&DockerExecResolver, &WslResolver, &HostResolver]
}

/// 按默认策略解析会话的文件树来源。
pub fn resolve_local_workspace_source(config: &LocalConfig) -> Option<LocalWorkspaceSource> {
    default_workspace_resolvers()
        .iter()
        .find_map(|resolver| resolver.resolve(config))
}

/// 解析 `docker [全局参数] exec [选项] <容器> [命令...]`。
///
/// 同时识别 Navop Docker 扩展自带 provider 的两种写法:
/// `docker-provider exec [-i] <容器> <命令...>` 与
/// `docker-provider exec-bridge <容器> [命令...]`——两者都指向容器内,
/// 侧边栏应走容器文件后端(provider 会以同样的 `exec` 语义响应)。
///
/// 容器名要求是单个非空片段,且不是 `-` 开头的选项,避免把命令拼到错误目标上。
pub fn docker_exec_invocation(config: &LocalConfig) -> Option<(DockerInvocation, String)> {
    let program = config.shell.as_deref()?;
    let provider = is_docker_provider_program(program);
    if !provider && !is_docker_program(program) {
        return None;
    }
    // provider 的交互式入口是 `exec-bridge`,非交互文件操作用 docker 风格的 `exec`。
    let subcommands: &[&str] = if provider {
        &["exec", "exec-bridge"]
    } else {
        &["exec"]
    };
    let mut args = config.args.iter().peekable();
    let mut global_args = Vec::new();
    loop {
        let arg = args.next()?;
        if subcommands.contains(&arg.as_str()) {
            break;
        }
        global_args.push(arg.clone());
    }
    let container = loop {
        let arg = args.next()?;
        if arg == "--" {
            break args.next()?.clone();
        }
        if arg.starts_with('-') {
            if arg.contains('=') || !exec_option_takes_value(arg) {
                continue;
            }
            // 选项与取值分开写:吞掉下一个参数。
            args.next();
            continue;
        }
        break arg.clone();
    };
    if container.trim().is_empty() || container.starts_with('-') {
        return None;
    }
    Some((
        DockerInvocation {
            program: program.to_string(),
            global_args,
            env: config.env.clone(),
        },
        container,
    ))
}

/// `exec` 的选项是否需要紧跟一个取值。
///
/// 长选项:`--user/--workdir/--env/--env-file/--detach-keys`。
/// 短选项组:`-u/-w/-e` 带值,`-i/-t/-d` 是纯开关;`-it` 这类组合不含带值字符。
fn exec_option_takes_value(arg: &str) -> bool {
    if let Some(long) = arg.strip_prefix("--") {
        matches!(
            long,
            "user" | "workdir" | "env" | "env-file" | "detach-keys"
        )
    } else if let Some(short) = arg.strip_prefix('-') {
        short.chars().any(|ch| matches!(ch, 'u' | 'w' | 'e'))
    } else {
        false
    }
}

/// 只比较文件名,允许写成 `docker`、`docker.exe` 或完整路径。
fn is_docker_program(program: &str) -> bool {
    let file_name = program.rsplit(['\\', '/']).next().unwrap_or(program);
    file_name.eq_ignore_ascii_case("docker") || file_name.eq_ignore_ascii_case("docker.exe")
}

/// Navop Docker 扩展自带 provider 的可执行文件名(`docker-provider`)。
fn is_docker_provider_program(program: &str) -> bool {
    let file_name = program.rsplit(['\\', '/']).next().unwrap_or(program);
    file_name.eq_ignore_ascii_case("docker-provider")
        || file_name.eq_ignore_ascii_case("docker-provider.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(shell: &str, args: &[&str]) -> LocalConfig {
        LocalConfig {
            shell: Some(shell.to_string()),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            working_dir: None,
            env: Vec::new(),
        }
    }

    #[test]
    fn parses_plain_docker_exec() {
        let (docker, container) =
            docker_exec_invocation(&config("docker", &["exec", "-it", "abc123", "sh"]))
                .expect("docker exec should parse");
        assert_eq!("docker", docker.program);
        assert!(docker.global_args.is_empty());
        assert_eq!("abc123", container);
    }

    #[test]
    fn keeps_global_args_before_exec() {
        let (docker, container) = docker_exec_invocation(&config(
            "/usr/local/bin/docker",
            &[
                "--context",
                "prod",
                "--host",
                "tcp://1.2.3.4:2375",
                "exec",
                "web",
                "bash",
            ],
        ))
        .expect("global args should parse");
        assert_eq!(
            vec![
                "--context".to_string(),
                "prod".to_string(),
                "--host".to_string(),
                "tcp://1.2.3.4:2375".to_string()
            ],
            docker.global_args
        );
        assert_eq!("web", container);
    }

    #[test]
    fn skips_exec_options_with_separate_and_inline_values() {
        let (_, container) = docker_exec_invocation(&config(
            "docker",
            &[
                "exec",
                "-u",
                "root",
                "-w",
                "/app",
                "-e",
                "A=b",
                "--env=K=V",
                "-it",
                "svc",
                "sh",
            ],
        ))
        .expect("mixed options should parse");
        assert_eq!("svc", container);
    }

    #[test]
    fn honors_double_dash_separator() {
        let (_, container) =
            docker_exec_invocation(&config("docker", &["exec", "--", "weird-name", "sh"]))
                .expect("-- separator should parse");
        assert_eq!("weird-name", container);
    }

    #[test]
    fn rejects_non_docker_shells() {
        assert!(docker_exec_invocation(&config("bash", &["-l"])).is_none());
        assert!(docker_exec_invocation(&config("wsl.exe", &["-d", "Ubuntu"])).is_none());
    }

    #[test]
    fn rejects_exec_without_container() {
        assert!(docker_exec_invocation(&config("docker", &["exec", "-it"])).is_none());
    }

    #[test]
    fn container_source_wins_over_host() {
        let source =
            resolve_local_workspace_source(&config("docker", &["exec", "-it", "abc", "sh"]))
                .expect("container source");
        assert!(source.is_container());
        assert_eq!(PathBuf::from("/"), source.root());
    }

    #[test]
    fn host_source_is_the_fallback() {
        let source = resolve_local_workspace_source(&config("bash", &[]));
        assert!(matches!(source, Some(LocalWorkspaceSource::Host { .. })));
    }

    #[test]
    fn docker_exec_args_prefix_global_args_and_container() {
        let docker = DockerInvocation {
            program: "docker".into(),
            global_args: vec!["--context".into(), "prod".into()],
            env: Vec::new(),
        };
        assert_eq!(
            vec![
                "--context".to_string(),
                "prod".to_string(),
                "exec".to_string(),
                "web".to_string(),
                "ls".to_string(),
                "-la".to_string()
            ],
            docker.exec_args("web", &["ls".to_string(), "-la".to_string()])
        );
    }

    #[test]
    fn recognizes_provider_exec_bridge_as_container() {
        let mut config = config(
            "/Users/me/.config/navop/extensions/composite/com.navop.docker/bin/docker-provider",
            &["exec-bridge", "abc123", "sh"],
        );
        config.env = vec![
            ("DOCKER_HOST".into(), "tcp://10.0.0.3:2375".into()),
            ("DOCKER_TLS_VERIFY".into(), String::new()),
        ];
        let (docker, container) =
            docker_exec_invocation(&config).expect("provider exec-bridge should parse");
        assert_eq!("abc123", container);
        assert!(docker.global_args.is_empty());
        // 终端 env 必须原样带给文件树后端,才能命中同一个远端 daemon。
        assert_eq!(
            docker.env,
            vec![
                ("DOCKER_HOST".to_string(), "tcp://10.0.0.3:2375".to_string()),
                ("DOCKER_TLS_VERIFY".to_string(), String::new()),
            ]
        );
        assert!(
            resolve_local_workspace_source(&config)
                .expect("container source")
                .is_container()
        );
    }

    #[test]
    fn recognizes_provider_exec_with_flags() {
        let (_, container) = docker_exec_invocation(&config(
            "docker-provider",
            &["exec", "-i", "svc", "ls", "-Ap"],
        ))
        .expect("provider exec should parse");
        assert_eq!("svc", container);
    }

    #[test]
    fn rejects_provider_without_exec_subcommand() {
        assert!(docker_exec_invocation(&config("docker-provider", &[])).is_none());
    }
}
