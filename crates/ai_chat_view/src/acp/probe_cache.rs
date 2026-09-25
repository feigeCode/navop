//! ACP agent 探测结果的落盘缓存。
//!
//! 探测会真的拉起一个 CLI 子进程，代价不低，因此结果要能跨会话复用：
//! - 记录按 agent id 存一份，附带**启动指纹**（命令 + 参数 + 环境变量的稳定哈希）。
//!   用户改了命令/参数/环境变量后指纹变化，旧结论立即视为失效。
//! - 记录里带上 `checked_at`，界面据此显示「检查于 X 前」，用户也可以手动刷新。
//!
//! 设置页与聊天切换器共用同一份缓存，避免同一个 agent 被反复拉起。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

use super::config::{AcpAgentConfig, AcpTransport};
use super::probe::AcpAgentProbe;

const CACHE_VERSION: u32 = 1;
const CACHE_FILE: &str = "acp-agent-probes.json";

/// 一条探测记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpProbeRecord {
    /// 启动指纹；与当前配置不一致时该记录不再可用。
    pub fingerprint: String,
    /// 检测时间（Unix 秒）。
    pub checked_at: i64,
    /// 探测结论。
    pub probe: AcpAgentProbe,
    /// 解析到的可执行文件绝对路径（命令是绝对路径时才有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_path: Option<String>,
}

impl AcpProbeRecord {
    pub fn new(fingerprint: impl Into<String>, probe: AcpAgentProbe) -> Self {
        Self {
            fingerprint: fingerprint.into(),
            checked_at: now_unix(),
            probe,
            command_path: None,
        }
    }

    pub fn with_command_path(mut self, command_path: Option<String>) -> Self {
        self.command_path = command_path;
        self
    }

    /// 检测至今经过的秒数。
    pub fn age_secs(&self) -> u64 {
        (now_unix() - self.checked_at).max(0) as u64
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AcpProbeCacheFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    probes: HashMap<String, AcpProbeRecord>,
}

#[derive(Default)]
struct AcpProbeCacheState {
    probes: HashMap<String, AcpProbeRecord>,
}

/// 进程内共享的探测缓存（写穿到磁盘）。
pub struct AcpProbeCache {
    state: RefCell<AcpProbeCacheState>,
    path: Option<PathBuf>,
}

impl Global for AcpProbeCache {}

impl AcpProbeCache {
    /// 读取缓存；磁盘上没有或解析失败时返回空缓存（探测总能重建）。
    pub fn load() -> Self {
        let path = cache_path();
        let probes = path
            .as_deref()
            .and_then(read_cache_file)
            .filter(|file| file.version == CACHE_VERSION)
            .map(|file| file.probes)
            .unwrap_or_default();
        Self {
            state: RefCell::new(AcpProbeCacheState { probes }),
            path,
        }
    }

    /// 取某个 agent 的有效记录；指纹不匹配视为没有缓存。
    pub fn get(&self, agent_id: &str, fingerprint: &str) -> Option<AcpProbeRecord> {
        self.state
            .borrow()
            .probes
            .get(agent_id)
            .filter(|record| record.fingerprint == fingerprint)
            .cloned()
    }

    /// 不做指纹校验地取记录（设置页列「已停用/历史结论」用）。
    pub fn get_any(&self, agent_id: &str) -> Option<AcpProbeRecord> {
        self.state.borrow().probes.get(agent_id).cloned()
    }

    /// 写入一条记录并落盘。
    pub fn store(&self, agent_id: &str, record: AcpProbeRecord) {
        self.state
            .borrow_mut()
            .probes
            .insert(agent_id.to_string(), record);
        self.persist();
    }

    /// 丢弃某些 agent 的记录（配置变了或用户要求重检测）。
    pub fn invalidate(&self, agent_ids: &[String]) {
        {
            let mut state = self.state.borrow_mut();
            for id in agent_ids {
                state.probes.remove(id);
            }
        }
        self.persist();
    }

    fn persist(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let file = AcpProbeCacheFile {
            version: CACHE_VERSION,
            probes: self.state.borrow().probes.clone(),
        };
        if let Err(error) = write_cache_file(path, &file) {
            tracing::warn!(%error, path = %path.display(), "failed to persist ACP probe cache");
        }
    }
}

/// 当前 agent 配置对应的启动指纹。
///
/// 只覆盖会影响探测结论的字段：可执行命令、参数、环境变量。
pub fn probe_fingerprint(config: &AcpAgentConfig) -> String {
    let AcpTransport::Stdio { command, args, env } = &config.transport;
    let mut spec = String::new();
    spec.push_str(command);
    for arg in args {
        spec.push('\u{1f}');
        spec.push_str(arg);
    }
    // 环境变量顺序不影响语义，排序后再入指纹。
    let mut env = env.clone();
    env.sort();
    for (name, value) in env {
        spec.push('\u{1e}');
        spec.push_str(&name);
        spec.push('=');
        spec.push_str(&value);
    }
    format!("{:016x}", fnv1a64(spec.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn cache_path() -> Option<PathBuf> {
    one_core::storage::manager::get_config_dir()
        .ok()
        .map(|dir| dir.join(CACHE_FILE))
}

fn read_cache_file(path: &std::path::Path) -> Option<AcpProbeCacheFile> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_cache_file(path: &std::path::Path, file: &AcpProbeCacheFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(file)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, content)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// 取全局缓存；没有就按磁盘内容初始化一次。
pub fn acp_probe_cache(cx: &mut App) -> &AcpProbeCache {
    if cx.try_global::<AcpProbeCache>().is_none() {
        let cache = AcpProbeCache::load();
        cx.set_global(cache);
    }
    cx.global::<AcpProbeCache>()
}

/// 测试用：构造只存在于内存的缓存（不碰磁盘）。
#[cfg(test)]
pub(crate) fn in_memory_probe_cache() -> AcpProbeCache {
    AcpProbeCache {
        state: RefCell::new(AcpProbeCacheState::default()),
        path: None,
    }
}

/// 测试用：直接构造记录。
#[cfg(test)]
pub(crate) fn test_record(
    fingerprint: &str,
    checked_at: i64,
    probe: AcpAgentProbe,
) -> AcpProbeRecord {
    AcpProbeRecord {
        fingerprint: fingerprint.to_string(),
        checked_at,
        probe,
        command_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(id: &str, command: &str, args: &[&str], env: &[(&str, &str)]) -> AcpAgentConfig {
        AcpAgentConfig::new(id, "Agent", command)
            .with_args(args.iter().map(|arg| arg.to_string()).collect())
            .with_env(
                env.iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
            )
    }

    #[test]
    fn fingerprint_ignores_identity_but_tracks_launch_spec() {
        let base = config("a", "codex-acp", &["--stdio"], &[("K", "1")]);
        let renamed = AcpAgentConfig::new("b", "Other name", "codex-acp")
            .with_args(vec!["--stdio".to_string()])
            .with_env(vec![("K".to_string(), "1".to_string())]);
        let other_arg = config("a", "codex-acp", &["--verbose"], &[("K", "1")]);
        let other_env = config("a", "codex-acp", &["--stdio"], &[("K", "2")]);
        let other_command = config("a", "claude-agent-acp", &["--stdio"], &[("K", "1")]);

        assert_eq!(probe_fingerprint(&base), probe_fingerprint(&renamed));
        assert_ne!(probe_fingerprint(&base), probe_fingerprint(&other_arg));
        assert_ne!(probe_fingerprint(&base), probe_fingerprint(&other_env));
        assert_ne!(probe_fingerprint(&base), probe_fingerprint(&other_command));
    }

    #[test]
    fn fingerprint_ignores_environment_ordering() {
        let first = config("a", "codex-acp", &[], &[("A", "1"), ("B", "2")]);
        let second = config("a", "codex-acp", &[], &[("B", "2"), ("A", "1")]);

        assert_eq!(probe_fingerprint(&first), probe_fingerprint(&second));
    }

    #[test]
    fn records_with_a_stale_fingerprint_are_not_returned() {
        let cache = in_memory_probe_cache();
        let probe = AcpAgentProbe {
            name: Some("Codex".to_string()),
            ..Default::default()
        };
        cache.store("codex", AcpProbeRecord::new("old", probe.clone()));

        assert!(cache.get("codex", "old").is_some());
        assert!(cache.get("codex", "new").is_none());
        // 无条件读取仍然能看到历史结论。
        assert!(cache.get_any("codex").is_some());
    }

    #[test]
    fn invalidate_drops_the_record() {
        let cache = in_memory_probe_cache();
        cache.store("codex", AcpProbeRecord::new("fp", AcpAgentProbe::default()));

        cache.invalidate(&["codex".to_string()]);

        assert!(cache.get_any("codex").is_none());
    }

    #[test]
    fn age_is_measured_from_the_check_time() {
        let record = test_record("fp", now_unix() - 90, AcpAgentProbe::default());

        assert_eq!(90, record.age_secs());
    }
}
