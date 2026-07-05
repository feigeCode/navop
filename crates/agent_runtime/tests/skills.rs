use agent_runtime::{SkillCatalog, SkillContext, SkillImportError, SkillRef, import_skill_dir};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn catalog_loads_codex_style_skill_metadata() {
    let root = tempdir().unwrap();
    write_skill(
        root.path().join("ops"),
        r#"---
name: ops
description: Run operational playbooks
metadata:
  short-description: Ops
---

# Ops

Use this skill for production checks.
"#,
    );

    let catalog = SkillCatalog::load_from_roots([root.path().to_path_buf()]);

    assert!(catalog.errors.is_empty());
    assert_eq!(1, catalog.skills.len());
    assert_eq!("ops", catalog.skills[0].name);
    assert_eq!("Run operational playbooks", catalog.skills[0].description);
    assert_eq!(Some("Ops"), catalog.skills[0].short_description.as_deref());
    assert!(catalog.skills[0].path.ends_with("ops/SKILL.md"));
}

#[test]
fn context_describes_and_wraps_selected_skill_metadata() {
    let root = tempdir().unwrap();
    let skill_path = root.path().join("ops").join("SKILL.md");
    write_skill(
        root.path().join("ops"),
        r#"---
name: ops
description: Run operational playbooks
---

Follow the deployment checklist before changing production.
"#,
    );
    let skill = SkillRef::new("ops", "Run operational playbooks", skill_path);

    let context = SkillContext::new().with_skill(skill);
    let description = context.describe();
    let wrapped = context.wrap_user_prompt("Deploy the service");

    assert!(description.contains("ops"));
    assert!(description.contains("Run operational playbooks"));
    assert!(wrapped.contains("Selected skills for this turn"));
    assert!(wrapped.contains("ops"));
    assert!(!wrapped.contains("Follow the deployment checklist"));
    assert!(wrapped.contains("Deploy the service"));
}

#[test]
fn empty_context_leaves_user_prompt_unchanged() {
    let context = SkillContext::new();

    assert_eq!("hello", context.wrap_user_prompt("hello"));
    assert!(context.describe().is_empty());
}

#[test]
fn import_skill_dir_copies_valid_skill_directory_into_destination_root() {
    let source_parent = tempdir().unwrap();
    let dest = tempdir().unwrap();
    write_skill(
        source_parent.path().join("custom"),
        r#"---
name: custom
description: Custom imported skill
---

Use the custom workflow.
"#,
    );

    let imported = import_skill_dir(&source_parent.path().join("custom"), dest.path()).unwrap();

    assert_eq!("custom", imported.name);
    assert_eq!("Custom imported skill", imported.description);
    assert!(dest.path().join("custom/SKILL.md").is_file());
}

#[test]
fn import_skill_dir_rejects_directories_without_skill_md() {
    let source_parent = tempdir().unwrap();
    let dest = tempdir().unwrap();
    fs::create_dir_all(source_parent.path().join("bad")).unwrap();

    let error = import_skill_dir(&source_parent.path().join("bad"), dest.path()).unwrap_err();

    assert!(matches!(error, SkillImportError::MissingSkillFile(_)));
}

fn write_skill(dir: impl AsRef<Path>, contents: &str) {
    fs::create_dir_all(dir.as_ref()).unwrap();
    fs::write(dir.as_ref().join("SKILL.md"), contents).unwrap();
}
