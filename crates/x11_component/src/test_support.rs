//! 所有读写进程级状态的单元测试共用同一异步互斥锁。
use tokio::sync::{Mutex, MutexGuard};

static GLOBAL_STATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

pub fn global_state_lock() -> MutexGuard<'static, ()> {
    GLOBAL_STATE_TEST_LOCK.blocking_lock()
}

pub async fn global_state_lock_async() -> MutexGuard<'static, ()> {
    GLOBAL_STATE_TEST_LOCK.lock().await
}
