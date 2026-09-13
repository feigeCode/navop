use super::{WslDistribution, decode_wsl_output, local_config_for_wsl_distro_with, parse_wsl_list_output};

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
        WslDistribution::new("openEuler-24.03".into(), Some("Stopped".into()), Some(2), false),
    ]
}

#[test]
fn parses_verbose_utf16_output_with_default_marker() {
    assert_eq!(expected_distributions(), parse_wsl_list_output(&utf16le(&verbose_sample())));
}

#[test]
fn parses_verbose_utf16_output_with_bom() {
    let mut raw = vec![0xFF, 0xFE];
    raw.extend(utf16le(&verbose_sample()));
    assert_eq!(expected_distributions(), parse_wsl_list_output(&raw));
}

#[test]
fn parses_utf8_output_without_bom() {
    assert_eq!(expected_distributions(), parse_wsl_list_output(verbose_sample().as_bytes()));
}

#[test]
fn parses_rows_missing_the_version_column_when_state_is_known() {
    let distributions = parse_wsl_list_output(b"Ubuntu-22.04  Running\r\n".as_slice());
    assert_eq!(
        vec![WslDistribution::new("Ubuntu-22.04".into(), Some("Running".into()), None, false)],
        distributions
    );
}

#[test]
fn localized_header_and_error_text_yield_no_distributions() {
    // 本地化表头的版本列不是数字，被稳定过滤；数据行不受表头本地化影响。
    let localized = "\r\n  名称            状态            版本\r\n* Ubuntu-22.04    正在运行         2\r\n";
    assert_eq!(
        vec![WslDistribution::new("Ubuntu-22.04".into(), Some("正在运行".into()), Some(2), true)],
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
    assert!(config
        .env
        .contains(&("TERM".to_string(), "xterm-256color".to_string())));
}

#[test]
fn distro_launch_config_trims_names_and_rejects_blank_names() {
    let config = local_config_for_wsl_distro_with("wsl.exe".into(), "  Debian-12  ").unwrap();
    assert_eq!(vec!["--distribution".to_string(), "Debian-12".to_string()], config.args);
    assert!(local_config_for_wsl_distro_with("wsl.exe".into(), "   ").is_err());
}

#[test]
fn decode_prefers_utf16_when_zero_bytes_dominate() {
    assert_eq!("abc".to_string(), decode_wsl_output(&utf16le("abc")));
    assert_eq!("abc".to_string(), decode_wsl_output(b"abc"));
}