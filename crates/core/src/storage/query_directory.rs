use crate::storage::manager::{get_config_dir, get_queries_dir};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const QUERY_DIRECTORIES_FILE: &str = "query-directories.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryDirectoryScope {
    pub database_type: String,
    pub connection_id: String,
    pub database: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryDirectoryEntryKind {
    Directory,
    SqlFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryDirectoryItem {
    pub name: String,
    pub path: PathBuf,
    pub kind: QueryDirectoryEntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuerySqlImportFailure {
    pub source: PathBuf,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuerySqlImportReport {
    pub imported: Vec<PathBuf>,
    pub failures: Vec<QuerySqlImportFailure>,
}

impl QueryDirectoryScope {
    pub fn new(
        database_type: impl Into<String>,
        connection_id: impl Into<String>,
        database: impl Into<String>,
    ) -> Self {
        Self {
            database_type: database_type.into(),
            connection_id: connection_id.into(),
            database: database.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct QueryDirectorySettings {
    #[serde(default)]
    entries: Vec<QueryDirectoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct QueryDirectoryEntry {
    scope: QueryDirectoryScope,
    directory: PathBuf,
}

pub fn default_query_directory(scope: &QueryDirectoryScope) -> Result<PathBuf> {
    Ok(get_queries_dir()?
        .join(&scope.database_type)
        .join(&scope.connection_id)
        .join(&scope.database))
}

pub fn query_directory(scope: &QueryDirectoryScope) -> Result<PathBuf> {
    default_query_directory(scope)
}

pub fn added_query_directories(scope: &QueryDirectoryScope) -> Result<Vec<PathBuf>> {
    let settings = load_settings(&settings_path()?)?;
    let mut directories = settings.get_all(scope);
    directories.sort_by(|left, right| {
        query_directory_display_name(left)
            .to_lowercase()
            .cmp(&query_directory_display_name(right).to_lowercase())
            .then_with(|| left.cmp(right))
    });
    Ok(directories)
}

pub fn add_query_directory(scope: &QueryDirectoryScope, directory: &Path) -> Result<PathBuf> {
    if !directory.is_dir() {
        bail!(
            "query directory does not exist or is not a directory: {}",
            directory.display()
        );
    }

    let directory = directory
        .canonicalize()
        .with_context(|| format!("failed to resolve query directory {}", directory.display()))?;
    if default_query_directory(scope)?
        .canonicalize()
        .is_ok_and(|default| default == directory)
    {
        return Ok(directory);
    }

    let path = settings_path()?;
    let mut settings = load_settings(&path)?;
    if settings.add(scope.clone(), directory.clone()) {
        save_settings(&path, &settings)?;
    }
    Ok(directory)
}

pub fn query_directory_display_name(directory: &Path) -> String {
    directory
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| directory.display().to_string())
}

pub fn is_sql_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
}

pub fn list_query_directory(directory: &Path) -> Result<Vec<QueryDirectoryItem>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read query directory {}", directory.display()))?;
    let mut items = Vec::new();

    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read an entry from query directory {}",
                directory.display()
            )
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect query entry {}", path.display()))?;

        let (name, kind) = if file_type.is_dir() {
            (
                entry.file_name().to_string_lossy().into_owned(),
                QueryDirectoryEntryKind::Directory,
            )
        } else if file_type.is_file() && is_sql_file(&path) {
            (
                path.file_stem()
                    .and_then(OsStr::to_str)
                    .unwrap_or("unknown")
                    .to_string(),
                QueryDirectoryEntryKind::SqlFile,
            )
        } else {
            continue;
        };

        items.push(QueryDirectoryItem { name, path, kind });
    }

    items.sort_by(|left, right| {
        let left_rank = match left.kind {
            QueryDirectoryEntryKind::Directory => 0,
            QueryDirectoryEntryKind::SqlFile => 1,
        };
        let right_rank = match right.kind {
            QueryDirectoryEntryKind::Directory => 0,
            QueryDirectoryEntryKind::SqlFile => 1,
        };
        left_rank
            .cmp(&right_rank)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(items)
}

pub fn create_query_subdirectory(parent: &Path, name: &str) -> Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        bail!("query directory name cannot be empty");
    }

    let relative = Path::new(name);
    if relative.components().count() != 1
        || matches!(
            relative.components().next(),
            Some(std::path::Component::ParentDir | std::path::Component::CurDir)
        )
    {
        bail!("query directory name must be a single path component");
    }

    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create query parent directory {}",
            parent.display()
        )
    })?;
    let directory = parent.join(relative);
    fs::create_dir(&directory)
        .with_context(|| format!("failed to create query directory {}", directory.display()))?;
    Ok(directory)
}

pub fn unique_sql_destination(directory: &Path, source: &Path) -> Result<PathBuf> {
    if !is_sql_file(source) {
        bail!("only SQL files can be imported: {}", source.display());
    }

    let file_name = source
        .file_name()
        .filter(|name| !name.is_empty())
        .context("SQL source path has no file name")?;
    let direct = directory.join(file_name);
    if !direct.exists() {
        return Ok(direct);
    }

    let stem = source
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .context("SQL source path has no file stem")?
        .to_string_lossy();
    let extension = source
        .extension()
        .context("SQL source path has no extension")?
        .to_string_lossy();

    for suffix in 1_u32.. {
        let candidate = directory.join(format!("{stem} ({suffix}).{extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    unreachable!("the import suffix range is unbounded")
}

pub fn import_query_sql_files<I, P>(directory: &Path, sources: I) -> Result<QuerySqlImportReport>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    fs::create_dir_all(directory).with_context(|| {
        format!(
            "failed to create query import directory {}",
            directory.display()
        )
    })?;

    let mut report = QuerySqlImportReport::default();
    for source in sources {
        let source = source.as_ref();
        let result = unique_sql_destination(directory, source).and_then(|destination| {
            fs::copy(source, &destination)
                .with_context(|| {
                    format!(
                        "failed to copy SQL file {} to {}",
                        source.display(),
                        destination.display()
                    )
                })
                .map(|_| destination)
        });

        match result {
            Ok(destination) => report.imported.push(destination),
            Err(error) => report.failures.push(QuerySqlImportFailure {
                source: source.to_path_buf(),
                error: error.to_string(),
            }),
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn scope() -> QueryDirectoryScope {
        QueryDirectoryScope::new("mysql", "42", "reporting")
    }

    #[test]
    fn settings_keep_multiple_added_directories_for_the_same_scope() {
        let first = PathBuf::from("/workspace/sql");
        let second = PathBuf::from("/archive/sql");
        let settings = QueryDirectorySettings {
            entries: vec![
                QueryDirectoryEntry {
                    scope: scope(),
                    directory: first.clone(),
                },
                QueryDirectoryEntry {
                    scope: scope(),
                    directory: second.clone(),
                },
            ],
        };

        assert_eq!(vec![first, second], settings.get_all(&scope()));
    }

    #[test]
    fn added_directories_are_isolated_by_scope() {
        let settings = QueryDirectorySettings {
            entries: vec![QueryDirectoryEntry {
                scope: scope(),
                directory: PathBuf::from("/workspace/sql"),
            }],
        };
        let other_scope = QueryDirectoryScope::new("mysql", "43", "reporting");

        assert!(settings.get_all(&other_scope).is_empty());
    }

    #[test]
    fn settings_round_trip_preserves_all_added_directories() {
        let temp = tempdir().unwrap();
        let settings_path = temp.path().join("query-directories.json");
        let first = temp.path().join("queries");
        let second = temp.path().join("archive");
        let mut settings = QueryDirectorySettings::default();
        settings.add(scope(), first.clone());
        settings.add(scope(), second.clone());

        save_settings(&settings_path, &settings).unwrap();
        let loaded = load_settings(&settings_path).unwrap();

        assert_eq!(vec![first, second], loaded.get_all(&scope()));
    }

    #[test]
    fn adding_the_same_directory_twice_does_not_duplicate_it() {
        let directory = PathBuf::from("/workspace/sql");
        let mut settings = QueryDirectorySettings::default();

        assert!(settings.add(scope(), directory.clone()));
        assert!(!settings.add(scope(), directory.clone()));

        assert_eq!(vec![directory], settings.get_all(&scope()));
    }

    #[test]
    fn legacy_single_directory_json_remains_compatible() {
        let content = r#"{
            "entries": [
                {
                    "scope": {
                        "database_type": "mysql",
                        "connection_id": "42",
                        "database": "reporting"
                    },
                    "directory": "/workspace/sql"
                }
            ]
        }"#;

        let settings: QueryDirectorySettings = serde_json::from_str(content).unwrap();

        assert_eq!(
            vec![PathBuf::from("/workspace/sql")],
            settings.get_all(&scope())
        );
    }

    #[test]
    fn sql_extension_matching_is_case_insensitive() {
        assert!(is_sql_file(Path::new("query.sql")));
        assert!(is_sql_file(Path::new("query.SQL")));
        assert!(!is_sql_file(Path::new("query.txt")));
        assert!(!is_sql_file(Path::new("sql")));
    }

    #[test]
    fn import_destination_uses_original_name_when_available() {
        let temp = tempdir().unwrap();
        let source = Path::new("/outside/report.sql");

        assert_eq!(
            temp.path().join("report.sql"),
            unique_sql_destination(temp.path(), source).unwrap()
        );
    }

    #[test]
    fn import_destination_avoids_overwriting_existing_files() {
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("report.sql"), "").unwrap();
        std::fs::write(temp.path().join("report (1).sql"), "").unwrap();

        assert_eq!(
            temp.path().join("report (2).sql"),
            unique_sql_destination(temp.path(), Path::new("/outside/report.sql")).unwrap()
        );
    }

    #[test]
    fn import_destination_rejects_non_sql_sources() {
        let temp = tempdir().unwrap();

        assert!(unique_sql_destination(temp.path(), Path::new("/outside/report.txt")).is_err());
    }

    #[test]
    fn importing_sql_copies_the_source_and_preserves_existing_files() {
        let source_dir = tempdir().unwrap();
        let target_dir = tempdir().unwrap();
        let source = source_dir.path().join("report.sql");
        std::fs::write(&source, "select 1;").unwrap();
        std::fs::write(target_dir.path().join("report.sql"), "select 0;").unwrap();

        let report = import_query_sql_files(target_dir.path(), vec![source.clone()]).unwrap();

        assert_eq!(
            vec![target_dir.path().join("report (1).sql")],
            report.imported
        );
        assert!(report.failures.is_empty());
        assert_eq!("select 1;", std::fs::read_to_string(&source).unwrap());
        assert_eq!(
            "select 0;",
            std::fs::read_to_string(target_dir.path().join("report.sql")).unwrap()
        );
        assert_eq!(
            "select 1;",
            std::fs::read_to_string(target_dir.path().join("report (1).sql")).unwrap()
        );
    }

    #[test]
    fn importing_sql_continues_after_an_invalid_source() {
        let source_dir = tempdir().unwrap();
        let target_dir = tempdir().unwrap();
        let invalid = source_dir.path().join("notes.txt");
        let valid = source_dir.path().join("report.sql");
        std::fs::write(&invalid, "not sql").unwrap();
        std::fs::write(&valid, "select 1;").unwrap();

        let report =
            import_query_sql_files(target_dir.path(), vec![invalid.clone(), valid]).unwrap();

        assert_eq!(vec![target_dir.path().join("report.sql")], report.imported);
        assert_eq!(1, report.failures.len());
        assert_eq!(invalid, report.failures[0].source);
        assert!(report.failures[0].error.contains("only SQL files"));
    }

    #[test]
    fn query_directory_entries_include_folders_and_sql_files_in_tree_order() {
        let temp = tempdir().unwrap();
        std::fs::create_dir(temp.path().join("reports")).unwrap();
        std::fs::create_dir(temp.path().join("archive")).unwrap();
        std::fs::write(temp.path().join("z.sql"), "").unwrap();
        std::fs::write(temp.path().join("A.SQL"), "").unwrap();
        std::fs::write(temp.path().join("ignored.txt"), "").unwrap();

        let entries = list_query_directory(temp.path()).unwrap();
        let summary = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect::<Vec<_>>();

        assert_eq!(
            vec![
                ("archive", QueryDirectoryEntryKind::Directory),
                ("reports", QueryDirectoryEntryKind::Directory),
                ("A", QueryDirectoryEntryKind::SqlFile),
                ("z", QueryDirectoryEntryKind::SqlFile),
            ],
            summary
        );
    }

    #[test]
    fn create_query_subdirectory_rejects_escaping_and_duplicate_names() {
        let temp = tempdir().unwrap();

        assert!(create_query_subdirectory(temp.path(), "../outside").is_err());
        assert!(create_query_subdirectory(temp.path(), "nested/folder").is_err());
        assert!(create_query_subdirectory(temp.path(), "").is_err());

        let created = create_query_subdirectory(temp.path(), "reports").unwrap();
        assert_eq!(temp.path().join("reports"), created);
        assert!(created.is_dir());
        assert!(create_query_subdirectory(temp.path(), "reports").is_err());
    }
}

impl QueryDirectorySettings {
    fn get_all(&self, scope: &QueryDirectoryScope) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|entry| &entry.scope == scope)
            .map(|entry| entry.directory.clone())
            .collect()
    }

    fn add(&mut self, scope: QueryDirectoryScope, directory: PathBuf) -> bool {
        if self
            .entries
            .iter()
            .any(|entry| entry.scope == scope && entry.directory == directory)
        {
            return false;
        }

        self.entries.push(QueryDirectoryEntry { scope, directory });
        true
    }
}

fn load_settings(path: &Path) -> Result<QueryDirectorySettings> {
    if !path.exists() {
        return Ok(QueryDirectorySettings::default());
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read query directory settings {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| {
        format!(
            "failed to parse query directory settings {}",
            path.display()
        )
    })
}

fn save_settings(path: &Path, settings: &QueryDirectorySettings) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create query directory settings parent {}",
                parent.display()
            )
        })?;
    }

    let content =
        serde_json::to_string_pretty(settings).context("failed to serialize query directories")?;
    std::fs::write(path, content)
        .with_context(|| format!("failed to save query directory settings {}", path.display()))
}

fn settings_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join(QUERY_DIRECTORIES_FILE))
}
