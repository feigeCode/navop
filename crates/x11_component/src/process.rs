use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::X11ComponentError;
use crate::lock::ComponentLock;
use crate::paths::atomic_write;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PidFile {
    pub pid: u32,
    pub started_at: u64,
    pub executable: PathBuf,
    #[serde(default = "legacy_record_version")]
    pub version: String,
    pub owner_pid: u32,
    pub owner_started_at: u64,
}

impl PidFile {
    pub fn read(path: &Path) -> Result<Self, X11ComponentError> {
        let bytes = std::fs::read(path).map_err(process_error)?;
        serde_json::from_slice(&bytes).map_err(|error| X11ComponentError::Config(error.to_string()))
    }

    pub fn write(&self, path: &Path) -> Result<(), X11ComponentError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| X11ComponentError::Process(error.to_string()))?;
        atomic_write(path, &bytes)
    }

    pub fn capture(
        pid: u32,
        _expected_executable: &Path,
        version: &str,
    ) -> Result<Self, X11ComponentError> {
        let identity = process_identity(pid)
            .ok_or_else(|| X11ComponentError::Process("无法读取 yserver 进程身份".into()))?;
        let owner = process_identity(std::process::id())
            .ok_or_else(|| X11ComponentError::Process("无法读取 Navop 进程身份".into()))?;
        Ok(Self {
            pid,
            started_at: identity.0,
            executable: identity.1,
            version: version.to_string(),
            owner_pid: std::process::id(),
            owner_started_at: owner.0,
        })
    }
}

pub fn instance_path(root: &Path, record: &PidFile) -> PathBuf {
    root.join("instances").join(format!(
        "{}-{}.json",
        record.owner_pid, record.owner_started_at
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrphanState {
    Active,
    Orphan,
    Stale,
}

pub fn same_process(record: &PidFile, current: Option<(u64, PathBuf)>) -> bool {
    current.is_some_and(|(started_at, executable)| {
        started_at == record.started_at && executable == record.executable
    })
}

pub fn orphan_state(
    record: &PidFile,
    child: Option<(u64, PathBuf)>,
    owner: Option<(u64, ())>,
) -> OrphanState {
    if !same_process(record, child) {
        return OrphanState::Stale;
    }
    if owner == Some((record.owner_started_at, ())) {
        OrphanState::Active
    } else {
        OrphanState::Orphan
    }
}

/// 启动前同时检查共享记录和各实例记录，覆盖只写完实例记录的崩溃窗口。
pub fn cleanup_orphan(path: &Path) -> Result<(), X11ComponentError> {
    cleanup_record(path)?;
    let Some(root) = path.parent() else {
        return Ok(());
    };
    let instances = root.join("instances");
    if !instances.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(instances).map_err(process_error)? {
        cleanup_record(&entry.map_err(process_error)?.path())?;
    }
    Ok(())
}

/// 活跃记录可能属于另一 Navop 实例；只有确认 stale 后才删除文件，绝不杀它。
fn cleanup_record(path: &Path) -> Result<(), X11ComponentError> {
    if !path.exists() {
        return Ok(());
    }
    let record = match PidFile::read(path) {
        Ok(record) => record,
        Err(X11ComponentError::Config(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    let owner = process_identity(record.owner_pid).map(|identity| (identity.0, ()));
    match orphan_state(&record, process_identity(record.pid), owner) {
        OrphanState::Active => Err(X11ComponentError::ActiveOtherInstance(
            "pidfile 属于另一活跃 Navop 实例；拒绝清理".into(),
        )),
        OrphanState::Orphan => {
            // 身份已匹配的子进程可能恰好退出；无论 kill 结果都移除该实例记录。
            let _ = kill_owned(&record);
            std::fs::remove_file(path).map_err(process_error)
        }
        OrphanState::Stale => std::fs::remove_file(path).map_err(process_error),
    }
}

/// 只有共享 pidfile 仍记录同一子进程时才允许删除。
pub fn remove_pidfile_if_owned(root: &Path, owned: &PidFile) -> Result<(), X11ComponentError> {
    let Some(_lock) = ComponentLock::try_acquire(root).map_err(process_error)? else {
        return Ok(());
    };
    remove_owned_files(root, owned)
}

pub(crate) fn remove_owned_files(root: &Path, owned: &PidFile) -> Result<(), X11ComponentError> {
    let path = root.join("yserver.pid");
    if path.exists() && PidFile::read(&path).is_ok_and(|current| current == *owned) {
        std::fs::remove_file(path).map_err(process_error)?;
    }
    remove_if_matching(&instance_path(root, owned), owned)
}

fn remove_if_matching(path: &Path, owned: &PidFile) -> Result<(), X11ComponentError> {
    if path.exists() && PidFile::read(path).is_ok_and(|current| current == *owned) {
        std::fs::remove_file(path).map_err(process_error)?;
    }
    Ok(())
}

/// 扫描每实例记录；owner 已退出但 yserver 尚存时同样保护其版本。
pub fn active_versions(root: &Path) -> Result<BTreeSet<String>, X11ComponentError> {
    let mut versions = BTreeSet::new();
    let directory = root.join("instances");
    if !directory.exists() {
        return Ok(versions);
    }
    for entry in std::fs::read_dir(directory).map_err(process_error)? {
        let path = entry.map_err(process_error)?.path();
        let Ok(record) = PidFile::read(&path) else {
            continue;
        };
        if same_process(&record, process_identity(record.pid)) {
            versions.insert(record.version);
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(versions)
}

/// 仅用于本进程子进程或 owner 已确认死亡的孤儿。
pub fn kill_owned(record: &PidFile) -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(record.pid)]));
    let current = system.process(Pid::from_u32(record.pid)).map(|process| {
        (
            process.start_time(),
            process.exe().map_or_else(PathBuf::new, Path::to_path_buf),
        )
    });
    if same_process(record, current) {
        if let Some(process) = system.process(Pid::from_u32(record.pid)) {
            return process.kill();
        }
    }
    false
}

static EXIT_RECORD: Mutex<Option<PidFile>> = Mutex::new(None);

pub fn register_exit_cleanup(record: PidFile) {
    *EXIT_RECORD
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(record);
}

extern "C" fn cleanup_at_exit() {
    if let Some(record) = EXIT_RECORD
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
    {
        kill_owned(&record);
    }
}

pub fn install_exit_handler() -> Result<(), X11ComponentError> {
    // SAFETY：注册只执行一次，回调在进程正常退出时清理自己的 yserver。
    if unsafe { libc::atexit(cleanup_at_exit) } != 0 {
        return Err(X11ComponentError::Process("注册退出清理失败".into()));
    }
    Ok(())
}

fn legacy_record_version() -> String {
    "legacy".into()
}

fn process_identity(pid: u32) -> Option<(u64, PathBuf)> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]));
    let process = system.process(Pid::from_u32(pid))?;
    Some((
        process.start_time(),
        process.exe().map_or_else(PathBuf::new, Path::to_path_buf),
    ))
}

fn process_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Process(error.to_string())
}

#[cfg(test)]
mod tests;
