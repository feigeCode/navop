use std::fs::File;
use std::path::{Path, PathBuf};

use fs2::FileExt as _;

/// 组件根目录的跨进程排他锁；文件描述符随守卫释放。
pub struct ComponentLock {
    _file: File,
}

impl ComponentLock {
    pub fn acquire(root: &Path) -> std::io::Result<Self> {
        let file = lock_file(root)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }

    pub async fn acquire_async(root: PathBuf) -> std::io::Result<Self> {
        tokio::task::spawn_blocking(move || Self::acquire(&root))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn try_acquire(root: &Path) -> std::io::Result<Option<Self>> {
        let file = lock_file(root)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn lock_file(root: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(root)?;
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".component.lock"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use super::ComponentLock;

    const ROOT_ENV: &str = "NAVOP_X11_LOCK_TEST_ROOT";
    const ID_ENV: &str = "NAVOP_X11_LOCK_TEST_ID";

    #[test]
    fn cross_process_worker() {
        let Some(root) = std::env::var_os(ROOT_ENV).map(PathBuf::from) else {
            return;
        };
        let id = std::env::var(ID_ENV).unwrap();
        std::fs::write(root.join(format!("{id}.started")), b"").unwrap();
        let _lock = ComponentLock::acquire(&root).unwrap();
        std::fs::write(root.join(format!("{id}.locked")), b"").unwrap();
    }

    #[test]
    fn component_lock_excludes_another_process() {
        let root = test_root();
        let first = ComponentLock::acquire(&root).unwrap();
        let second = spawn_worker(&root, "second");
        wait_for(&root.join("second.started"), Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !root.join("second.locked").exists(),
            "第一个进程持锁时第二个进程不得进入临界区"
        );
        drop(first);
        wait_child(second);
        assert!(root.join("second.locked").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn spawn_worker(root: &Path, id: &str) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lock::tests::cross_process_worker",
                "--nocapture",
            ])
            .env(ROOT_ENV, root)
            .env(ID_ENV, id)
            .spawn()
            .unwrap()
    }

    fn wait_for(path: &Path, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(path.exists(), "等待 {} 超时", path.display());
    }

    fn wait_child(mut child: Child) {
        let status = child.wait().unwrap();
        assert!(status.success(), "锁测试子进程失败：{status}");
    }

    fn test_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("x11-lock-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
