use super::*;

fn context(temp: &tempfile::TempDir) -> AppPathResolutionContext {
    AppPathResolutionContext {
        executable_path: temp.path().join("navop.exe"),
        current_dir: temp.path().to_path_buf(),
        portable_environment: None,
        data_dir_environment: None,
    }
}

#[test]
fn portable_flag_is_consumed_without_treating_files_as_options() {
    let parsed = parse_startup_arguments(["--portable", "query.sql"].map(OsString::from))
        .expect("parse startup arguments");

    assert!(parsed.path_overrides.portable);
    assert_eq!(vec![OsString::from("query.sql")], parsed.remaining);
}

#[test]
fn explicit_data_dir_is_consumed_and_implies_portable_mode() {
    let parsed =
        parse_startup_arguments(["--data-dir", "portable-data", "query.sql"].map(OsString::from))
            .expect("parse startup arguments");

    assert_eq!(
        Some(PathBuf::from("portable-data")),
        parsed.path_overrides.data_dir
    );
    assert_eq!(vec![OsString::from("query.sql")], parsed.remaining);
}

#[test]
fn argument_terminator_preserves_option_like_file_names() {
    let parsed = parse_startup_arguments(["--", "--portable"].map(OsString::from))
        .expect("parse startup arguments");

    assert!(!parsed.path_overrides.portable);
    assert_eq!(vec![OsString::from("--portable")], parsed.remaining);
}

#[test]
fn data_dir_requires_a_non_empty_path() {
    assert!(parse_startup_arguments([OsString::from("--data-dir")]).is_err());
    assert!(parse_startup_arguments([OsString::from("--data-dir=")]).is_err());
}

#[test]
fn portable_flag_uses_sibling_data_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };

    let paths = resolve_app_paths(&overrides, &context(&temp)).expect("resolve paths");

    assert_eq!(
        &AppRunMode::Portable {
            root: temp.path().to_path_buf()
        },
        paths.mode()
    );
    assert_eq!(temp.path().join("data/config"), *paths.config_dir());
    assert_eq!(Some(&temp.path().join("data/state")), paths.data_dir());
    assert_eq!(Some(&temp.path().join("data/cache")), paths.cache_dir());
}

#[test]
fn marker_file_enables_portable_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join(PORTABLE_MARKER_FILE), "").expect("write marker");

    let paths =
        resolve_app_paths(&AppPathOverrides::default(), &context(&temp)).expect("resolve paths");

    assert!(paths.is_portable());
}

#[test]
fn macos_bundle_uses_the_directory_next_to_the_app() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bundle = temp.path().join("Navop.app/Contents/MacOS/navop");
    let context = AppPathResolutionContext {
        executable_path: bundle,
        current_dir: temp.path().to_path_buf(),
        portable_environment: None,
        data_dir_environment: None,
    };
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };

    let paths = resolve_app_paths(&overrides, &context).expect("resolve paths");

    assert_eq!(temp.path().join("data/config"), *paths.config_dir());
}

#[test]
fn environment_data_dir_is_resolved_against_current_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut context = context(&temp);
    context.data_dir_environment = Some(OsString::from("custom-data"));

    let paths = resolve_app_paths(&AppPathOverrides::default(), &context).expect("resolve paths");

    assert!(paths.is_portable());
    assert_eq!(temp.path().join("custom-data/config"), *paths.config_dir());
}

#[test]
fn explicit_data_dir_overrides_environment_data_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut context = context(&temp);
    context.data_dir_environment = Some(OsString::from("environment-data"));
    let overrides = AppPathOverrides {
        portable: false,
        data_dir: Some(PathBuf::from("explicit-data")),
    };

    let paths = resolve_app_paths(&overrides, &context).expect("resolve paths");

    assert_eq!(
        temp.path().join("explicit-data/config"),
        *paths.config_dir()
    );
}

#[test]
fn portable_flag_overrides_environment_data_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut context = context(&temp);
    context.data_dir_environment = Some(OsString::from("environment-data"));
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };

    let paths = resolve_app_paths(&overrides, &context).expect("resolve paths");

    assert_eq!(temp.path().join("data/config"), *paths.config_dir());
}

#[test]
fn truthy_portable_environment_enables_portable_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut context = context(&temp);
    context.portable_environment = Some(OsString::from("true"));

    let paths = resolve_app_paths(&AppPathOverrides::default(), &context).expect("resolve paths");

    assert!(paths.is_portable());
}

#[test]
fn portable_paths_are_created_and_checked_for_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = portable_paths(temp.path().to_path_buf(), temp.path().join("data"));

    prepare_paths(&paths).expect("prepare portable paths");

    assert!(temp.path().join("data/config").is_dir());
    assert!(temp.path().join("data/state").is_dir());
    assert!(temp.path().join("data/cache").is_dir());
}

#[test]
fn installed_mode_keeps_persistent_master_key_support() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths =
        resolve_app_paths(&AppPathOverrides::default(), &context(&temp)).expect("resolve paths");

    assert!(!paths.is_portable());
    assert_eq!(Some(OsStr::new("navop")), paths.config_dir().file_name());
    assert!(paths.allows_persistent_master_key());
}

#[test]
fn portable_mode_disables_persistent_master_key_support() {
    let temp = tempfile::tempdir().expect("tempdir");
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };

    let paths = resolve_app_paths(&overrides, &context(&temp)).expect("resolve paths");

    assert!(!paths.allows_persistent_master_key());
    assert!(paths.requires_master_key_on_startup(false));
}
