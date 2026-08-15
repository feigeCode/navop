use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const FAVORITES_FILE_NAME: &str = "favorites.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLogFavorites {
    #[serde(default)]
    recording_ids: BTreeSet<String>,
}

impl SessionLogFavorites {
    pub fn contains(&self, recording_id: &str) -> bool {
        self.recording_ids.contains(recording_id)
    }

    pub fn set(&mut self, recording_id: impl Into<String>, favorite: bool) -> bool {
        let recording_id = recording_id.into();
        if favorite {
            self.recording_ids.insert(recording_id)
        } else {
            self.recording_ids.remove(&recording_id)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.recording_ids.is_empty()
    }
}

pub fn load_session_log_favorites(
    session_logs_directory: impl AsRef<Path>,
) -> io::Result<SessionLogFavorites> {
    let path = favorites_path(session_logs_directory.as_ref());
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(invalid_data),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SessionLogFavorites::default()),
        Err(error) => Err(error),
    }
}

pub fn save_session_log_favorites(
    session_logs_directory: impl AsRef<Path>,
    favorites: &SessionLogFavorites,
) -> io::Result<()> {
    let directory = session_logs_directory.as_ref();
    fs::create_dir_all(directory)?;
    let destination = favorites_path(directory);
    let temporary = temporary_path(directory);
    let bytes = serde_json::to_vec_pretty(favorites).map_err(invalid_data)?;
    write_and_publish(&temporary, &destination, &bytes)
}

fn favorites_path(directory: &Path) -> PathBuf {
    directory.join(FAVORITES_FILE_NAME)
}

fn temporary_path(directory: &Path) -> PathBuf {
    directory.join(format!(".favorites-{}.tmp", Uuid::new_v4()))
}

fn write_and_publish(temporary: &Path, destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(temporary, destination)?;
        sync_parent_directory(destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let flags = MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0);
    unsafe {
        MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(destination.as_ptr()), flags)
            .map_err(io::Error::other)
    }
}

#[cfg(unix)]
fn sync_parent_directory(destination: &Path) -> io::Result<()> {
    let Some(directory) = destination.parent() else {
        return Ok(());
    };
    fs::File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_destination: &Path) -> io::Result<()> {
    Ok(())
}

fn invalid_data(error: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
