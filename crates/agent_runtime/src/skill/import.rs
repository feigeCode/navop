use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

use super::SKILL_FILE;
use super::catalog::{SkillMetadata, parse_skill_file};

#[derive(Debug, Error)]
pub enum SkillImportError {
    #[error("skill directory is missing SKILL.md: {0}")]
    MissingSkillFile(PathBuf),
    #[error("destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("invalid skill {path}: {message}")]
    InvalidSkill { path: PathBuf, message: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub fn import_skill_dir(
    source: &Path,
    dest_root: &Path,
) -> Result<SkillMetadata, SkillImportError> {
    let source_skill = source.join(SKILL_FILE);
    if !source_skill.is_file() {
        return Err(SkillImportError::MissingSkillFile(source.to_path_buf()));
    }
    fs::create_dir_all(dest_root)?;
    let dest = dest_root.join(skill_dir_name(source)?);
    if dest.exists() {
        return Err(SkillImportError::DestinationExists(dest));
    }
    copy_dir_recursive(source, &dest)?;
    parse_skill_file(&dest.join(SKILL_FILE)).map_err(|err| SkillImportError::InvalidSkill {
        path: dest.join(SKILL_FILE),
        message: err.to_string(),
    })
}

fn skill_dir_name(source: &Path) -> Result<&std::ffi::OsStr, io::Error> {
    source.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "skill source must have a directory name",
        )
    })
}

fn copy_dir_recursive(source: &Path, dest: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &dest_path)?;
        } else {
            fs::copy(&source_path, &dest_path)?;
        }
    }
    Ok(())
}
