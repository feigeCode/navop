//! 目录上传的受限并发调度。
//!
//! `upload_dir_with_progress` 里每个文件都要付固定的控制往返
//! （`stat` 探测属主 → `open` 暂存文件 → `flush`(含 fsync) → `fstat` 校验 →
//! `close` → `rename` 发布）。串行执行时，小文件吞吐由「单文件大小 ÷ 往返次数 × RTT」
//! 决定而不是带宽：30 ms 延迟链路上一个文件就要 180 ms，1000 个小文件跑到 3 分钟以上。
//!
//! 这个模块提供两件东西：
//!
//! 1. [`run_bounded`]：把多个「每文件」的未来重叠起来，让控制往返互相掩盖；
//! 2. [`ConcurrencyBudget`]：给大文件留出让步 —— 它们的写入段本来就会填满 SFTP 写窗口
//!    （`max_concurrent_writes` × `max_write_packet_len`），并发只会成倍放大缓冲字节数，
//!    换不到额外吞吐。

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

/// 允许同时上传的小文件数量。
pub(crate) const MAX_CONCURRENT_SMALL_FILE_UPLOADS: usize = 6;

/// 超过该大小的文件独占整个并发预算，行为与串行上传完全一致。
///
/// 与 `PIPELINE_THRESHOLD` 对齐（512 KiB）：这个尺寸以上的文件写入段已经会用满写窗口，
/// 控制往返不再是主要成本，没必要为它付出并发缓冲。
pub(crate) const SMALL_FILE_MAX_BYTES: u64 = 512 * 1024;

/// 单个任务占用的槽位数：小文件占 1 个，大文件独占全部槽位。
pub(crate) fn job_weight(file_size: u64, limit: usize) -> usize {
    let limit = limit.max(1);
    if file_size > SMALL_FILE_MAX_BYTES {
        limit
    } else {
        1
    }
}

/// 并发预算：按槽位计数，大文件一次吃掉全部槽位。
pub(crate) struct ConcurrencyBudget {
    limit: usize,
    in_use: usize,
}

impl ConcurrencyBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            in_use: 0,
        }
    }

    pub(crate) fn can_admit(&self, weight: usize) -> bool {
        self.in_use + weight <= self.limit
    }

    pub(crate) fn admit(&mut self, weight: usize) {
        self.in_use += weight;
    }

    pub(crate) fn release(&mut self, weight: usize) {
        self.in_use = self.in_use.saturating_sub(weight);
    }
}

/// 按 [`ConcurrencyBudget`] 并发执行任务，首个错误之后不再接纳新任务，但在飞的任务会跑完。
///
/// 「在飞的任务跑完再返回错误」是刻意保留的语义：每个文件在出错路径上要删除自己的
/// 暂存文件（`RemoteReplaceTemp::cleanup`），中途 drop 未来会让暂存文件残留。
pub(crate) async fn run_bounded<J, E, F, Fut>(
    jobs: Vec<J>,
    limit: usize,
    weight_of: impl Fn(&J) -> usize,
    run: F,
) -> Result<(), E>
where
    F: Fn(J) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let mut budget = ConcurrencyBudget::new(limit);
    let mut queued: VecDeque<J> = jobs.into();
    let mut running = FuturesUnordered::new();
    let mut first_error: Option<E> = None;

    loop {
        if first_error.is_none() {
            while let Some(next) = queued.front() {
                let weight = weight_of(next);
                if !budget.can_admit(weight) {
                    break;
                }
                let Some(job) = queued.pop_front() else {
                    break;
                };
                budget.admit(weight);
                let future = run(job);
                running.push(async move { (weight, future.await) });
            }
        }

        let Some((weight, result)) = running.next().await else {
            break;
        };
        budget.release(weight);

        if let Err(error) = result
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// 对外发布的进度高水位：并发任务各自上报，落地时只允许单调不减。
///
/// 没有它时，两个文件交错上报会让进度条回跳（后到的回调携带更小的累计值）。
pub(crate) struct ProgressHighWater {
    published: AtomicU64,
}

impl ProgressHighWater {
    pub(crate) fn new() -> Self {
        Self {
            published: AtomicU64::new(0),
        }
    }

    /// 记下 `value`，返回本次应当对外发布的值（不会低于此前发布过的值）。
    pub(crate) fn publish(&self, value: u64) -> u64 {
        let mut current = self.published.load(Ordering::Acquire);
        while value > current {
            match self.published.compare_exchange_weak(
                current,
                value,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return value,
                Err(observed) => current = observed,
            }
        }
        current
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConcurrencyBudget, MAX_CONCURRENT_SMALL_FILE_UPLOADS, ProgressHighWater,
        SMALL_FILE_MAX_BYTES, job_weight, run_bounded,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const TEST_LIMIT: usize = 4;

    #[test]
    fn small_file_weight_is_one_slot_and_large_file_takes_the_whole_budget() {
        assert_eq!(job_weight(SMALL_FILE_MAX_BYTES, TEST_LIMIT), 1);
        assert_eq!(job_weight(0, TEST_LIMIT), 1);
        assert_eq!(job_weight(SMALL_FILE_MAX_BYTES + 1, TEST_LIMIT), TEST_LIMIT);
        assert_eq!(job_weight(8 * 1024 * 1024, TEST_LIMIT), TEST_LIMIT);
    }

    #[test]
    fn budget_admits_small_jobs_up_to_the_limit() {
        let mut budget = ConcurrencyBudget::new(2);
        assert!(budget.can_admit(1));
        budget.admit(1);
        assert!(budget.can_admit(1));
        budget.admit(1);
        assert!(!budget.can_admit(1));

        budget.release(1);
        assert!(budget.can_admit(1));
    }

    #[test]
    fn budget_keeps_large_jobs_exclusive() {
        let mut budget = ConcurrencyBudget::new(TEST_LIMIT);
        budget.admit(1);
        assert!(
            !budget.can_admit(TEST_LIMIT),
            "小文件在飞时大文件不得进入预算"
        );

        budget.release(1);
        assert!(budget.can_admit(TEST_LIMIT));

        budget.admit(TEST_LIMIT);
        assert!(!budget.can_admit(1), "大文件在飞时小文件不得插队");
    }

    #[tokio::test]
    async fn run_bounded_overlaps_small_jobs() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let jobs: Vec<u64> = (0..TEST_LIMIT as u64).map(|index| index * 1024).collect();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_bounded(
                jobs,
                TEST_LIMIT,
                |_| 1,
                |_job| {
                    let in_flight = in_flight.clone();
                    let peak = peak.clone();
                    async move {
                        let now = in_flight.fetch_add(1, Ordering::AcqRel) + 1;
                        peak.fetch_max(now, Ordering::AcqRel);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        in_flight.fetch_sub(1, Ordering::AcqRel);
                        Ok::<(), anyhow::Error>(())
                    }
                },
            ),
        )
        .await
        .expect("并发调度不应超时");

        assert!(result.is_ok());
        assert_eq!(
            peak.load(Ordering::Acquire),
            TEST_LIMIT,
            "小文件必须真正重叠执行，而不是逐个 await"
        );
    }

    #[tokio::test]
    async fn run_bounded_keeps_large_jobs_serial() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let jobs: Vec<u64> = vec![8 * 1024 * 1024; 3];
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_bounded(
                jobs,
                TEST_LIMIT,
                |_| TEST_LIMIT,
                |_job| {
                    let in_flight = in_flight.clone();
                    let peak = peak.clone();
                    async move {
                        let now = in_flight.fetch_add(1, Ordering::AcqRel) + 1;
                        peak.fetch_max(now, Ordering::AcqRel);
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        in_flight.fetch_sub(1, Ordering::AcqRel);
                        Ok::<(), anyhow::Error>(())
                    }
                },
            ),
        )
        .await
        .expect("并发调度不应超时");

        assert!(result.is_ok());
        assert_eq!(
            peak.load(Ordering::Acquire),
            1,
            "大文件必须保持串行，避免成倍放大内存中的未确认字节"
        );
    }

    #[tokio::test]
    async fn run_bounded_drains_in_flight_jobs_after_the_first_error() {
        let slow_job_finished = Arc::new(AtomicBool::new(false));
        let queued_job_started = Arc::new(AtomicBool::new(false));

        let jobs: Vec<&'static str> = vec!["fails", "slow", "queued"];
        let result: Result<(), &'static str> = tokio::time::timeout(
            Duration::from_secs(5),
            run_bounded(
                jobs,
                2,
                |_| 1,
                |job| {
                    let slow_job_finished = slow_job_finished.clone();
                    let queued_job_started = queued_job_started.clone();
                    async move {
                        match job {
                            "fails" => Err("first failure"),
                            "slow" => {
                                tokio::time::sleep(Duration::from_millis(20)).await;
                                slow_job_finished.store(true, Ordering::Release);
                                Ok(())
                            }
                            _ => {
                                queued_job_started.store(true, Ordering::Release);
                                Ok(())
                            }
                        }
                    }
                },
            ),
        )
        .await
        .expect("并发调度不应超时");

        assert_eq!(result, Err("first failure"));
        assert!(
            slow_job_finished.load(Ordering::Acquire),
            "出错时在飞的任务必须跑完，否则它占用的暂存文件不会被清理"
        );
        assert!(
            !queued_job_started.load(Ordering::Acquire),
            "出错后不得再接纳排队中的任务"
        );
    }

    #[test]
    fn published_progress_never_regresses() {
        let high_water = ProgressHighWater::new();
        let published = [
            high_water.publish(10),
            high_water.publish(5),
            high_water.publish(30),
            high_water.publish(20),
        ];

        assert_eq!(published, [10, 10, 30, 30]);
    }

    #[test]
    fn default_concurrency_is_above_one_and_bounded() {
        assert!(MAX_CONCURRENT_SMALL_FILE_UPLOADS > 1);
        assert!(MAX_CONCURRENT_SMALL_FILE_UPLOADS <= 16);
    }
}
