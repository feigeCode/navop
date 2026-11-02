use super::*;

fn context(temp: &tempfile::TempDir) -> AppPathResolutionContext {
    AppPathResolutionContext {
        executable_path: temp.path().join("navop.exe"),
        current_dir: temp.path().to_path_buf(),
        portable_environment: None,
        data_dir_environment: None,
        development_data_dir: None,
    }
}

fn development_context(temp: &tempfile::TempDir) -> AppPathResolutionContext {
    AppPathResolutionContext {
        development_data_dir: Some(temp.path().join("navop-dev")),
        ..context(temp)
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
fn portable_master_key_storage_is_kept_under_the_data_state_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };
    let paths = resolve_app_paths(&overrides, &context(&temp)).expect("resolve paths");
    let data_dir = paths.data_dir().expect("portable data dir");

    assert_eq!(
        temp.path().join("data/state/key_storage"),
        crate::key_storage::key_storage_path_for_data_dir(data_dir)
    );
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
        development_data_dir: None,
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
fn portable_mode_allows_opt_in_persistent_master_key_support() {
    let temp = tempfile::tempdir().expect("tempdir");
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };

    let paths = resolve_app_paths(&overrides, &context(&temp)).expect("resolve paths");

    assert!(paths.allows_persistent_master_key());
}

#[test]
fn portable_mode_requires_startup_unlock_until_persistence_is_enabled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };

    let paths = resolve_app_paths(&overrides, &context(&temp)).expect("resolve paths");

    assert!(paths.requires_master_key_on_startup(false, false));
    assert!(!paths.requires_master_key_on_startup(false, true));
}

#[test]
fn installed_mode_uses_the_existing_startup_lock_setting() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths =
        resolve_app_paths(&AppPathOverrides::default(), &context(&temp)).expect("resolve paths");

    assert!(!paths.requires_master_key_on_startup(false, false));
    assert!(paths.requires_master_key_on_startup(true, false));
}

#[test]
fn debug_build_uses_an_isolated_development_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = resolve_app_paths(&AppPathOverrides::default(), &development_context(&temp))
        .expect("resolve paths");

    let root = temp.path().join("navop-dev");
    assert_eq!(
        &AppRunMode::Development { root: root.clone() },
        paths.mode()
    );
    assert_eq!(root.join("config"), *paths.config_dir());
    assert_eq!(Some(&root.join("state")), paths.data_dir());
    assert_eq!(Some(&root.join("cache")), paths.cache_dir());
    assert!(paths.is_development());
    assert!(!paths.is_portable());
    assert!(!paths.uses_installed_directories());
}

#[test]
fn explicit_data_dir_overrides_the_development_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let overrides = AppPathOverrides {
        portable: false,
        data_dir: Some(PathBuf::from("explicit-data")),
    };

    let paths = resolve_app_paths(&overrides, &development_context(&temp)).expect("resolve paths");

    assert!(paths.is_portable());
    assert_eq!(
        temp.path().join("explicit-data/config"),
        *paths.config_dir()
    );
}

#[test]
fn environment_data_dir_overrides_the_development_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut context = development_context(&temp);
    context.data_dir_environment = Some(OsString::from("environment-data"));

    let paths = resolve_app_paths(&AppPathOverrides::default(), &context).expect("resolve paths");

    assert!(paths.is_portable());
    assert_eq!(
        temp.path().join("environment-data/config"),
        *paths.config_dir()
    );
}

#[test]
fn portable_flag_overrides_the_development_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let overrides = AppPathOverrides {
        portable: true,
        data_dir: None,
    };

    let paths = resolve_app_paths(&overrides, &development_context(&temp)).expect("resolve paths");

    assert!(paths.is_portable());
    assert_eq!(temp.path().join("data/config"), *paths.config_dir());
}

#[test]
fn development_mode_keeps_installed_master_key_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = resolve_app_paths(&AppPathOverrides::default(), &development_context(&temp))
        .expect("resolve paths");

    assert!(!paths.requires_master_key_on_startup(false, false));
    assert!(paths.requires_master_key_on_startup(true, false));
}

#[test]
fn development_paths_are_created_and_checked_for_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("navop-dev");
    let paths = development_paths(root.clone());

    prepare_paths(&paths).expect("prepare development paths");

    assert!(root.join("config").is_dir());
    assert!(root.join("state").is_dir());
    assert!(root.join("cache").is_dir());
}

#[test]
fn development_directory_is_derived_from_the_build_profile() {
    let directory = development_data_dir_for(Path::new("/usr/local/bin/navop"));

    if cfg!(debug_assertions) {
        assert_eq!(
            Some(OsStr::new(DEVELOPMENT_DATA_DIR_NAME)),
            directory.as_deref().and_then(Path::file_name)
        );
    } else {
        assert!(directory.is_none());
    }
}

#[test]
fn source_builds_are_isolated_even_in_release_profile() {
    // release 自测同样必须隔离：这些路径过去都被当成「正式安装态」，
    // 一次运行就会写生产配置与生产 keychain。
    for executable in [
        "/repo/target/release/navop",
        "/repo/target/debug/navop",
        "/repo/targets/build-123/release/navop",
        "/repo/target/release/bundle/macos/Navop.app/Contents/MacOS/navop",
    ] {
        assert!(
            runs_from_source_build(Path::new(executable)),
            "{executable} 应当识别为源码构建"
        );
    }

    // cargo install 的产物落在 ~/.cargo/bin，同样不是安装态。
    let home = dirs::home_dir().expect("home");
    assert!(
        runs_from_source_build(&home.join(".cargo/bin/navop")),
        "cargo install 产物应当识别为源码构建"
    );

    // 真正的安装位置不隔离。
    for executable in [
        "/Applications/Navop.app/Contents/MacOS/navop",
        "/usr/bin/navop",
        "/opt/Navop/navop",
    ] {
        assert!(
            !runs_from_source_build(Path::new(executable)),
            "{executable} 应当按安装态处理"
        );
        if !cfg!(debug_assertions) {
            assert!(
                development_data_dir_for(Path::new(executable)).is_none(),
                "{executable} 在 release 下应当使用安装态目录"
            );
        }
    }
}
