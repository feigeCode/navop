const DEFAULT_PORT: u16 = 3389;
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct Config {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: Option<String>,
    pub(crate) domain: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) timeout_seconds: u64,
}

pub(crate) enum ParseOutcome {
    Run(Config),
    Help,
}

struct ParseState {
    host: Option<String>,
    port: u16,
    username: Option<String>,
    domain: Option<String>,
    width: u32,
    height: u32,
    timeout_seconds: u64,
}

impl Default for ParseState {
    fn default() -> Self {
        Self {
            host: None,
            port: DEFAULT_PORT,
            username: None,
            domain: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

pub(crate) fn parse_args<I, S>(args: I, password: Option<String>) -> Result<ParseOutcome, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut state = ParseState::default();
    let mut args = args.into_iter().map(Into::into);
    while let Some(argument) = args.next() {
        if matches!(argument.as_str(), "-h" | "--help") {
            return Ok(ParseOutcome::Help);
        }
        parse_argument(&argument, &mut args, &mut state)?;
    }
    finish_parse(state, password).map(ParseOutcome::Run)
}

fn parse_argument<I>(argument: &str, args: &mut I, state: &mut ParseState) -> Result<(), String>
where
    I: Iterator<Item = String>,
{
    match argument {
        "--host" => state.host = Some(next_value(args, "--host")?),
        "--port" => state.port = parse_number(next_value(args, "--port")?, "--port")?,
        "--username" => state.username = Some(next_value(args, "--username")?),
        "--domain" => state.domain = Some(next_value(args, "--domain")?),
        "--width" => {
            state.width = parse_positive_dimension(next_value(args, "--width")?, "--width")?
        }
        "--height" => {
            state.height = parse_positive_dimension(next_value(args, "--height")?, "--height")?
        }
        "--timeout-seconds" => {
            state.timeout_seconds =
                parse_positive_number(next_value(args, "--timeout-seconds")?, "--timeout-seconds")?
        }
        _ => return Err(format!("unknown argument `{argument}`")),
    }
    Ok(())
}

fn finish_parse(state: ParseState, password: Option<String>) -> Result<Config, String> {
    let host = state
        .host
        .ok_or_else(|| "missing required argument `--host <host>`".to_owned())?;
    if host.is_empty() {
        return Err("`--host` must not be empty".to_owned());
    }
    Ok(Config {
        host,
        port: state.port,
        username: state.username,
        domain: state.domain,
        password,
        width: state.width,
        height: state.height,
        timeout_seconds: state.timeout_seconds,
    })
}

fn next_value<I>(args: &mut I, option: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| format!("missing value for `{option}`"))
}

fn parse_number<T>(value: String, option: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for `{option}`"))
}

fn parse_positive_number<T>(value: String, option: &str) -> Result<T, String>
where
    T: std::str::FromStr + Default + PartialOrd,
{
    let parsed = parse_number(value, option)?;
    if parsed <= T::default() {
        return Err(format!("`{option}` must be greater than zero"));
    }
    Ok(parsed)
}

fn parse_positive_dimension(value: String, option: &str) -> Result<u32, String> {
    let dimension = parse_positive_number(value, option)?;
    if dimension > i32::MAX as u32 {
        return Err(format!("`{option}` must not exceed {}", i32::MAX));
    }
    Ok(dimension)
}

pub(crate) fn usage() -> &'static str {
    "Minimal GPUI Windows native RDP smoke client

Usage:
  gpui-rdp-smoke --host <host> [options]

Required:
  --host <host>                 RDP server name or IP address

Options:
  --port <port>                 RDP port (default: 3389)
  --username <username>         User name
  --domain <domain>             Windows domain
  --width <pixels>              Remote desktop width (default: 1280)
  --height <pixels>             Remote desktop height (default: 720)
  --timeout-seconds <seconds>   Login diagnostic timeout (default: 60)
  -h, --help                    Print this help

Password:
  Read only from NAVOP_RDP_PASSWORD. Do not put a password on the command line."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ParseOutcome, String> {
        parse_args(args.iter().copied(), None)
    }

    fn parse_error(args: &[&str]) -> String {
        match parse(args) {
            Ok(_) => panic!("expected argument parsing to fail"),
            Err(error) => error,
        }
    }

    #[test]
    fn parses_required_host_and_defaults() {
        let ParseOutcome::Run(config) = parse(&["--host", "rdp.example"]).unwrap() else {
            panic!("expected run configuration");
        };
        assert_eq!(config.host, "rdp.example");
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.width, DEFAULT_WIDTH);
        assert_eq!(config.height, DEFAULT_HEIGHT);
        assert_eq!(config.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
        assert!(config.username.is_none());
        assert!(config.domain.is_none());
        assert!(config.password.is_none());
    }

    #[test]
    fn parses_all_options_and_password_source() {
        let ParseOutcome::Run(config) = parse_args(
            [
                "--host",
                "10.0.0.5",
                "--port",
                "3390",
                "--username",
                "alice",
                "--domain",
                "EXAMPLE",
                "--width",
                "1600",
                "--height",
                "900",
                "--timeout-seconds",
                "90",
            ],
            Some("secret".to_owned()),
        )
        .unwrap() else {
            panic!("expected run configuration");
        };
        assert_eq!(config.port, 3390);
        assert_eq!(config.username.as_deref(), Some("alice"));
        assert_eq!(config.domain.as_deref(), Some("EXAMPLE"));
        assert_eq!(config.width, 1600);
        assert_eq!(config.height, 900);
        assert_eq!(config.timeout_seconds, 90);
        assert_eq!(config.password.as_deref(), Some("secret"));
    }

    #[test]
    fn rejects_missing_host_unknown_arguments_and_invalid_numbers() {
        assert!(parse_error(&[]).contains("--host"));
        assert!(parse_error(&["--wat"]).contains("unknown argument"));
        assert!(parse_error(&["--host", "rdp.example", "--port", "not-a-port"]).contains("--port"));
        assert!(
            parse_error(&["--host", "rdp.example", "--width", "0"]).contains("greater than zero")
        );
        assert!(
            parse_error(&["--host", "rdp.example", "--height", "2147483648"])
                .contains("must not exceed")
        );
    }

    #[test]
    fn help_does_not_require_host() {
        assert!(matches!(parse(&["--help"]).unwrap(), ParseOutcome::Help));
    }
}
