use super::{
    WslDistribution, decode_wsl_output, local_config_for_wsl_distro_with, parse_wsl_list_output,
    resolve_reported_working_dir, wsl_distribution_for_config, wsl_unc_path, wsl_unc_root,
};
#[cfg(target_os = "windows")]
use super::wsl_distribution_from_root;
use crate::LocalConfig;
use std::path::{Path, PathBuf};

fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn verbose_sample() -> String {
    [
        "\r\n",
        "  NAME            STATE           VERSION\r\n",
        "* Ubuntu-22.04    Running         2\r\n",
        "  Debian          Stopped         1\r\n",
        "  openEuler-24.03  Stopped         2\r\n",
    ]
    .concat()
}

fn expected_distributions() -> Vec<WslDistribution> {
    vec![
        WslDistribution::new("Ubuntu-22.04".into(), Some("Running".into()), Some(2), true),
        WslDistribution::new("Debian".into(), Some("Stopped".into()), Some(1), false),
        WslDistribution::new(
            "openEuler-24.03".into(),
            Some("Stopped".into()),
            Some(2),
            false,
        ),
    ]
}

#[test]
fn parses_verbose_utf16_output_with_default_marker() {
    assert_eq!(
        expected_distributions(),
        parse_wsl_list_output(&utf16le(&verbose_sample()))
    );
}

#[test]
fn parses_verbose_utf16_output_with_bom() {
    let mut raw = vec![0xFF, 0xFE];
    raw.extend(utf16le(&verbose_sample()));
    assert_eq!(expected_distributions(), parse_wsl_list_output(&raw));
}

#[test]
fn parses_utf8_output_without_bom() {
    assert_eq!(
        expected_distributions(),
        parse_wsl_list_output(verbose_sample().as_bytes())
    );
}

#[test]
fn parses_rows_missing_the_version_column_when_state_is_known() {
    let distributions = parse_wsl_list_output(b"Ubuntu-22.04  Running\r\n".as_slice());
    assert_eq!(
        vec![WslDistribution::new(
            "Ubuntu-22.04".into(),
            Some("Running".into()),
            None,
            false
        )],
        distributions
    );
}

#[test]
fn localized_header_and_error_text_yield_no_distributions() {
    // 本地化表头的版本列不是数字，被稳定过滤；数据行不受表头本地化影响。
    let localized =
        "\r\n  名称            状态            版本\r\n* Ubuntu-22.04    正在运行         2\r\n";
    assert_eq!(
        vec![WslDistribution::new(
            "Ubuntu-22.04".into(),
            Some("正在运行".into()),
            Some(2),
            true
        )],
        parse_wsl_list_output(localized.as_bytes())
    );
    let error = "适用于 Linux 的 Windows 子系统没有已安装的分发版。\r\n";
    assert!(parse_wsl_list_output(&utf16le(error)).is_empty());
}

#[test]
fn header_only_output_yields_no_distributions() {
    let header_only = "\r\n  NAME            STATE           VERSION\r\n";
    assert!(parse_wsl_list_output(&utf16le(header_only)).is_empty());
}

#[test]
fn empty_and_blank_output_yield_no_distributions() {
    assert!(parse_wsl_list_output(&[]).is_empty());
    assert!(parse_wsl_list_output(b"\r\n").is_empty());
}

#[test]
fn distro_launch_config_uses_the_distribution_argument_form() {
    let config =
        local_config_for_wsl_distro_with("C:\\Windows\\System32\\wsl.exe".into(), "Ubuntu-22.04")
            .unwrap();
    assert_eq!(
        Some("C:\\Windows\\System32\\wsl.exe".to_string()),
        config.shell
    );
    assert_eq!(
        vec!["--distribution".to_string(), "Ubuntu-22.04".to_string()],
        config.args
    );
    assert!(config.working_dir.is_none());
    // 继承 LocalConfig::default() 的基础终端环境变量
    assert!(
        config
            .env
            .contains(&("TERM".to_string(), "xterm-256color".to_string()))
    );
}

#[test]
fn distro_launch_config_trims_names_and_rejects_blank_names() {
    let config = local_config_for_wsl_distro_with("wsl.exe".into(), "  Debian-12  ").unwrap();
    assert_eq!(
        vec!["--distribution".to_string(), "Debian-12".to_string()],
        config.args
    );
    assert!(local_config_for_wsl_distro_with("wsl.exe".into(), "   ").is_err());
}

#[test]
fn decode_prefers_utf16_when_zero_bytes_dominate() {
    assert_eq!("abc".to_string(), decode_wsl_output(&utf16le("abc")));
    assert_eq!("abc".to_string(), decode_wsl_output(b"abc"));
}

// ---------------------------------------------------------------------------
// WSL 会话的文件树根目录映射
//
// WSL 终端本身只是一个跑 `wsl.exe -d <distro>` 的本地终端，它没有工作目录，
// 文件树如果按普通本地会话回落到 Windows 主目录，就会显示本机磁盘而不是
// 发行版里的文件。这里把「会话 → 发行版 → UNC 路径」的映射固化成纯函数。
// ---------------------------------------------------------------------------

fn wsl_config(args: &[&str]) -> LocalConfig {
    LocalConfig {
        shell: Some("wsl.exe".into()),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        ..LocalConfig::default()
    }
}

#[test]
fn distro_launch_config_round_trips_its_distribution_name() {
    let config =
        local_config_for_wsl_distro_with("C:\\Windows\\System32\\wsl.exe".into(), "Ubuntu-24.04")
            .unwrap();

    assert_eq!(Some("Ubuntu-24.04"), wsl_distribution_for_config(&config));
}

#[test]
fn only_wsl_programs_with_a_distribution_argument_count_as_wsl_sessions() {
    // 别的 shell 带同名参数不算 WSL 会话
    let powershell = LocalConfig {
        shell: Some("powershell.exe".into()),
        args: vec!["--distribution".into(), "Ubuntu-24.04".into()],
        ..LocalConfig::default()
    };
    assert_eq!(None, wsl_distribution_for_config(&powershell));

    // wsl.exe 会走进默认发行版，但不是「某个发行版会话」，无从确定根目录
    assert_eq!(None, wsl_distribution_for_config(&wsl_config(&[])));
    assert_eq!(
        None,
        wsl_distribution_for_config(&wsl_config(&["--distribution"]))
    );
    assert_eq!(
        None,
        wsl_distribution_for_config(&wsl_config(&["-d", "   "]))
    );

    let without_shell = LocalConfig {
        shell: None,
        ..LocalConfig::default()
    };
    assert_eq!(None, wsl_distribution_for_config(&without_shell));
}

#[test]
fn distribution_argument_accepts_both_flags_and_ignores_other_arguments() {
    assert_eq!(
        Some("Debian"),
        wsl_distribution_for_config(&wsl_config(&["-d", "Debian"]))
    );
    assert_eq!(
        Some("Debian"),
        wsl_distribution_for_config(&wsl_config(&[
            "--cd",
            "/home/navop",
            "--distribution",
            "Debian"
        ]))
    );
}

#[test]
fn wsl_program_matching_only_compares_the_file_name() {
    for program in [
        r"C:\Windows\System32\wsl.exe",
        r"C:\Windows\System32\WSL.EXE",
        "wsl",
        "/usr/bin/wsl",
    ] {
        let config = LocalConfig {
            shell: Some(program.into()),
            args: vec!["--distribution".into(), "Debian".into()],
            ..LocalConfig::default()
        };
        assert_eq!(
            Some("Debian"),
            wsl_distribution_for_config(&config),
            "program={program}"
        );
    }

    let lookalike = LocalConfig {
        shell: Some("wslconfig.exe".into()),
        args: vec!["--distribution".into(), "Debian".into()],
        ..LocalConfig::default()
    };
    assert_eq!(None, wsl_distribution_for_config(&lookalike));
}

#[test]
fn wsl_unc_root_points_at_the_distribution_share() {
    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Ubuntu-24.04")),
        wsl_unc_root("Ubuntu-24.04")
    );
    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Debian")),
        wsl_unc_root("  Debian  ")
    );

    // 发行版名必须是单个路径片段，否则会拼出指向别处的路径
    for invalid in ["", "   ", ".", "..", "a/b", r"a\b", r"..\..\x"] {
        assert_eq!(None, wsl_unc_root(invalid), "invalid={invalid:?}");
    }
}

#[test]
fn wsl_unc_path_maps_absolute_linux_paths_into_the_share() {
    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Ubuntu-24.04")),
        wsl_unc_path("Ubuntu-24.04", "/")
    );
    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Ubuntu-24.04\home\navop")),
        wsl_unc_path("Ubuntu-24.04", "/home/navop")
    );
    // 重复与结尾分隔符不影响结果
    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Ubuntu-24.04\etc")),
        wsl_unc_path("Ubuntu-24.04", "/etc/")
    );
    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Ubuntu-24.04\etc")),
        wsl_unc_path("Ubuntu-24.04", "//etc")
    );

    // 相对路径与向上越界都无法确定落在发行版内，拒绝映射
    for invalid in [
        "",
        "etc",
        "home/navop",
        "/home/../etc",
        "/home/..",
        "/..",
        "/home/navop/../../etc",
    ] {
        assert_eq!(
            None,
            wsl_unc_path("Ubuntu-24.04", invalid),
            "invalid={invalid:?}"
        );
    }
}

#[cfg(target_os = "windows")]
#[test]
fn wsl_distribution_from_root_recognizes_windows_shares() {
    for root in [
        r"\\wsl$\Ubuntu-24.04",
        r"\\wsl$\Ubuntu-24.04\home\navop",
        r"\\wsl.localhost\Ubuntu-24.04",
        // canonicalize() 的 verbatim 形式
        r"\\?\UNC\wsl$\Ubuntu-24.04\etc",
    ] {
        assert_eq!(
            Some("Ubuntu-24.04"),
            wsl_distribution_from_root(Path::new(root)),
            "root={root}"
        );
    }
}

#[cfg(target_os = "windows")]
#[test]
fn wsl_distribution_from_root_ignores_other_roots() {
    for root in [
        r"C:\Users\navop",
        r"D:\work",
        r"\\server\share",
        r"\\wsl$",
        r"\\wslx\Ubuntu",
    ] {
        assert_eq!(
            None,
            wsl_distribution_from_root(Path::new(root)),
            "root={root}"
        );
    }
}

#[test]
fn reported_non_wsl_directory_is_used_as_is() {
    let root = Path::new(r"D:\work");

    assert_eq!(
        Some(PathBuf::from(r"D:\work\src")),
        resolve_reported_working_dir(root, r"D:\work\src")
    );
    assert_eq!(
        Some(PathBuf::from("relative")),
        resolve_reported_working_dir(root, "relative")
    );
}

#[cfg(target_os = "windows")]
#[test]
fn reported_linux_directory_is_mapped_back_into_the_wsl_share() {
    let root = Path::new(r"\\wsl$\Ubuntu-24.04");

    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Ubuntu-24.04\etc")),
        resolve_reported_working_dir(root, "/etc")
    );
    assert_eq!(
        Some(PathBuf::from(r"\\wsl$\Ubuntu-24.04")),
        resolve_reported_working_dir(root, "/")
    );
    assert_eq!(None, resolve_reported_working_dir(root, "etc"));
}

/// 结构契约：WSL 识别必须走后台子进程约定隐藏控制台窗口。
///
/// `wsl.exe` 是控制台程序，无控制台的 GUI 进程直接 spawn 会让 Windows 新建一个
/// 控制台窗口（应用启动时表现为闪一下黑框）。
#[test]
fn wsl_detection_hides_the_background_console() {
    let source = include_str!("wsl_distributions.rs");

    assert!(
        source.contains("process_util::configure_background_child(&mut command)"),
        "WSL 识别未隐藏控制台：Windows 启动时会闪出控制台窗口"
    );
}
