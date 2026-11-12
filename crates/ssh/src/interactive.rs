//! 交互流量让路：记录进程内最近一次用户按键的时刻。
//!
//! 侧边栏文件管理器与终端共用同一条 SSH 传输，批量上传会持续灌满这条 TCP
//! 连接的发送缓冲，用户按键只能排在缓冲区末尾，表现为"上传时终端打字反应
//! 半天"。SFTP 没有实时性要求，人打字有，所以批量写入在用户输入期间主动
//! 让路：先让发送缓冲排空，按键才能立刻发出去。
//!
//! 这里用进程级时钟而不是按会话记录，是为了让所有上传路径（侧边栏、历史
//! 查询等）不必新增参数就能共享同一个判断；代价是往 A 机上传时会因为 B 机
//! 的输入停顿几十毫秒，这种保守偏差对上传吞吐没有实质影响。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// 用户停止输入多久之后恢复批量写入。
pub const INTERACTIVE_QUIET: Duration = Duration::from_millis(100);
/// 一次让路的最长停顿。用户持续输入时上传不能无限期停住，停顿到期后仍然要
/// 写下一块；按 256 KiB 一块计算，这个上限把"边打字边传"的速率保底在
/// 1.5 MB/s 以上，而一旦停手（[`INTERACTIVE_QUIET`] 内没有新按键）立刻恢复
/// 全速。
pub const INTERACTIVE_MAX_PAUSE: Duration = Duration::from_millis(150);

/// 最近一次用户按键时刻（毫秒，相对 [`CLOCK_EPOCH`]），0 表示还没有记录。
static LAST_USER_INPUT_MS: AtomicU64 = AtomicU64::new(0);
static CLOCK_EPOCH: OnceLock<Instant> = OnceLock::new();

fn clock_epoch() -> Instant {
    *CLOCK_EPOCH.get_or_init(Instant::now)
}

fn now_ms() -> u64 {
    clock_epoch().elapsed().as_millis() as u64
}

/// 记录一次用户按键。终端把按键写进 SSH 通道时调用。
pub fn note_user_input() {
    LAST_USER_INPUT_MS.store(now_ms().max(1), Ordering::Relaxed);
}

/// 距离最近一次用户按键过去了多久；从未记录过时返回 `None`。
pub fn user_input_idle() -> Option<Duration> {
    let last = LAST_USER_INPUT_MS.load(Ordering::Relaxed);
    (last > 0).then(|| Duration::from_millis(now_ms().saturating_sub(last)))
}

/// 批量传输写下一块数据前的让路点。
///
/// 只要用户最近 [`INTERACTIVE_QUIET`] 内敲过键盘就等一会儿，让共享连接上的
/// 发送缓冲先排空，按键不再排在几 MB 数据后面；单次让路最多阻塞
/// [`INTERACTIVE_MAX_PAUSE`]，上传不会因此停住。
pub async fn yield_to_user_input() {
    let Some(idle) = user_input_idle() else {
        return;
    };
    if idle >= INTERACTIVE_QUIET {
        return;
    }

    let started = Instant::now();
    loop {
        let Some(idle) = user_input_idle() else {
            return;
        };
        if idle >= INTERACTIVE_QUIET {
            break;
        }
        let Some(remaining) = INTERACTIVE_MAX_PAUSE.checked_sub(started.elapsed()) else {
            break;
        };
        let wait = (INTERACTIVE_QUIET - idle).min(remaining);
        if wait.is_zero() {
            break;
        }
        tokio::time::sleep(wait).await;
    }

    let paused = started.elapsed();
    if paused >= Duration::from_millis(20) {
        tracing::debug!(
            target: "ssh.interactive",
            paused_ms = paused.as_millis() as u64,
            "批量传输为用户按键让路"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两个场景共用进程级时钟，必须放在同一个用例里顺序执行，否则并行用例会
    /// 互相重置状态。
    #[tokio::test]
    async fn yield_to_user_input_only_waits_while_the_user_types() {
        LAST_USER_INPUT_MS.store(0, Ordering::Relaxed);
        assert_eq!(user_input_idle(), None, "没有按键记录时不应伪造空闲时间");

        let started = Instant::now();
        yield_to_user_input().await;
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "没有按键时让路点必须立即返回，实际耗时 {:?}",
            started.elapsed()
        );

        note_user_input();
        assert!(
            user_input_idle().is_some_and(|idle| idle < Duration::from_millis(50)),
            "刚记录过的按键必须表现为最近输入"
        );

        let started = Instant::now();
        yield_to_user_input().await;
        let waited = started.elapsed();
        assert!(
            waited >= INTERACTIVE_QUIET - Duration::from_millis(20),
            "按键之后应先等输入静默，实际只等了 {waited:?}"
        );
        assert!(
            waited <= INTERACTIVE_MAX_PAUSE + Duration::from_millis(100),
            "让路不能超过上限，实际等了 {waited:?}"
        );

        let started = Instant::now();
        yield_to_user_input().await;
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "输入已静默后必须立即放行，实际耗时 {:?}",
            started.elapsed()
        );
    }
}
