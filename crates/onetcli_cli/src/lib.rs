use clap::{Parser, ValueEnum, builder::PossibleValue};
use std::ffi::OsString;

#[derive(Debug, PartialEq, Eq)]
pub enum OnetCliCommand {
    Tool(ToolCommand),
    Connection(ConnectionCommand),
    Db(DbCommand),
    Ssh(SshCommand),
    Sftp(SftpCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
}

impl ValueEnum for OutputFormat {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Json]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Self::Json => Some(PossibleValue::new("json")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum ToolCommand {
    /// List CLI-callable tools.
    List {
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Print a tool descriptor with input and output schemas.
    Schema {
        /// Tool id to inspect.
        tool_id: String,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Call a tool with optional JSON input.
    Call {
        /// Tool id to call.
        tool_id: String,
        /// JSON object passed to the tool. Defaults to {}.
        #[arg(long, conflicts_with = "positional_input")]
        input: Option<String>,
        /// Positional JSON input kept for compatibility. Prefer --input.
        #[arg(value_name = "JSON_INPUT")]
        positional_input: Option<String>,
        /// Allow mutating or destructive tools to run.
        #[arg(long)]
        allow_write: bool,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum ConnectionCommand {
    /// List saved connections.
    List {
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Show a saved connection by id or exact name.
    Show {
        /// Connection id or exact name.
        connection: String,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum DbCommand {
    /// Show database schema metadata.
    Schema {
        /// Saved connection id or exact name.
        connection: String,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Run a read-oriented SQL query.
    Query {
        /// Saved connection id or exact name.
        connection: String,
        /// SQL text to run.
        #[arg(long)]
        sql: String,
        /// Require read-only execution.
        #[arg(long)]
        readonly: bool,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Execute a write-capable SQL file.
    Exec {
        /// Saved connection id or exact name.
        connection: String,
        /// SQL file to run.
        #[arg(long)]
        file: String,
        /// Explicitly allow write execution.
        #[arg(long)]
        write: bool,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum SshCommand {
    /// Execute a remote command over SSH.
    Exec {
        /// Saved connection id or exact name.
        connection: String,
        /// Remote command to execute.
        #[arg(long)]
        command: String,
        /// Optional timeout, for example 10s.
        #[arg(long)]
        timeout: Option<String>,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Start an interactive SSH shell.
    Shell {
        /// Saved connection id or exact name.
        connection: String,
        /// Remote working directory.
        #[arg(long)]
        workdir: Option<String>,
        /// Initialization script.
        #[arg(long)]
        init: Option<String>,
        /// Transcript output path.
        #[arg(long)]
        transcript: Option<String>,
    },
    /// Open a local port forward.
    Tunnel {
        /// Saved connection id or exact name.
        connection: String,
        /// Local bind port.
        #[arg(long)]
        local: u16,
        /// Remote host:port target.
        #[arg(long)]
        remote: String,
    },
    /// Open a SOCKS proxy.
    Socks {
        /// Saved connection id or exact name.
        connection: String,
        /// Local bind port.
        #[arg(long)]
        local: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum SftpCommand {
    /// List a remote directory.
    List {
        /// Saved connection id or exact name.
        connection: String,
        /// Remote path.
        path: String,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Read a remote file.
    Read {
        /// Saved connection id or exact name.
        connection: String,
        /// Remote path.
        path: String,
        /// Maximum bytes to read.
        #[arg(long)]
        max_bytes: Option<u64>,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Check whether a remote path exists.
    Stat {
        /// Saved connection id or exact name.
        connection: String,
        /// Remote path.
        path: String,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Upload a local file or directory.
    Upload {
        /// Saved connection id or exact name.
        connection: String,
        /// Local file or directory path.
        local_path: String,
        /// Remote destination path.
        remote_path: String,
        /// Existing target policy: fail, overwrite, or skip.
        #[arg(long, default_value = "fail")]
        on_exists: String,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Download a remote file or directory.
    Download {
        /// Saved connection id or exact name.
        connection: String,
        /// Remote file or directory path.
        remote_path: String,
        /// Local destination path.
        local_path: String,
        /// Existing target policy: fail, overwrite, or skip.
        #[arg(long, default_value = "fail")]
        on_exists: String,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
}

#[derive(Debug, Parser)]
#[command(name = "onetcli")]
#[command(about = "OnetCli desktop app and automation commands")]
struct CliArgs {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, clap::Subcommand)]
enum CliCommand {
    /// Inspect and call OnetCli tools.
    Tool {
        #[command(subcommand)]
        command: ToolCommand,
    },
    /// Inspect saved OnetCli connections.
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    /// Database automation commands.
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
    /// SSH automation commands.
    Ssh {
        #[command(subcommand)]
        command: SshCommand,
    },
    /// SFTP automation commands.
    Sftp {
        #[command(subcommand)]
        command: SftpCommand,
    },
}

pub fn parse_from<I, T>(args: I) -> Result<Option<OnetCliCommand>, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = CliArgs::try_parse_from(args)?;
    Ok(args.command.map(|command| match command {
        CliCommand::Tool { command } => OnetCliCommand::Tool(command),
        CliCommand::Connection { command } => OnetCliCommand::Connection(command),
        CliCommand::Db { command } => OnetCliCommand::Db(command),
        CliCommand::Ssh { command } => OnetCliCommand::Ssh(command),
        CliCommand::Sftp { command } => OnetCliCommand::Sftp(command),
    }))
}

pub fn print_error(error: clap::Error) -> i32 {
    let exit_code = error.exit_code();
    let _ = error.print();
    exit_code
}

#[cfg(test)]
mod tests;
