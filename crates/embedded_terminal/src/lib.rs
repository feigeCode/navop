//! Lightweight embedded terminal execution model.
//!
//! This crate does not depend on or modify the existing terminal UI. It maps a
//! target and a structured process request to an explicit execution plan. The
//! application owns PTY rendering, authentication, tunnels, host-key checks,
//! resize, input, and shutdown.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TerminalTarget {
    Local,
    Remote { connection_id: i64 },
}

impl TerminalTarget {
    pub const fn local() -> Self {
        Self::Local
    }

    pub fn remote(connection_id: i64) -> Result<Self, TerminalError> {
        if connection_id <= 0 {
            return Err(TerminalError::InvalidRemoteConnection);
        }
        Ok(Self::Remote { connection_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRequest {
    target: TerminalTarget,
    program: String,
    args: Vec<String>,
    environment: Vec<(String, String)>,
    working_directory: Option<String>,
    tty: bool,
}

impl TerminalRequest {
    pub fn new<I, S>(
        target: TerminalTarget,
        program: impl Into<String>,
        args: I,
    ) -> Result<Self, TerminalError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let program = program.into();
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        if !valid_argument(&program) || args.iter().any(|argument| !valid_argument(argument)) {
            return Err(TerminalError::InvalidCommand);
        }
        Ok(Self {
            target,
            program,
            args,
            environment: Vec::new(),
            working_directory: None,
            tty: true,
        })
    }

    pub fn shell(target: TerminalTarget, shell: impl Into<String>) -> Result<Self, TerminalError> {
        Self::new(target, shell, std::iter::empty::<String>())
    }

    pub fn with_environment(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, TerminalError> {
        let key = key.into();
        let value = value.into();
        if key.is_empty()
            || key.contains('=')
            || !key
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
            || value.contains(['\0', '\n', '\r'])
        {
            return Err(TerminalError::InvalidEnvironment);
        }
        self.environment.push((key, value));
        Ok(self)
    }

    pub fn tty(mut self, tty: bool) -> Self {
        self.tty = tty;
        self
    }

    pub fn with_working_directory(
        mut self,
        path: impl Into<String>,
    ) -> Result<Self, TerminalError> {
        let path = path.into();
        if path.trim().is_empty() || path.chars().any(char::is_control) {
            return Err(TerminalError::InvalidWorkingDirectory);
        }
        self.working_directory = Some(path);
        Ok(self)
    }

    pub fn target(&self) -> &TerminalTarget {
        &self.target
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn environment(&self) -> &[(String, String)] {
        &self.environment
    }

    pub fn working_directory(&self) -> Option<&str> {
        self.working_directory.as_deref()
    }

    pub fn has_tty(&self) -> bool {
        self.tty
    }

    pub fn plan(&self) -> TerminalExecutionPlan {
        match self.target {
            TerminalTarget::Local => TerminalExecutionPlan::Local {
                program: self.program.clone(),
                args: self.args.clone(),
                environment: self.environment.clone(),
                working_directory: self.working_directory.clone(),
                tty: self.tty,
            },
            TerminalTarget::Remote { connection_id } => TerminalExecutionPlan::Remote {
                connection_id,
                program: self.program.clone(),
                args: self.args.clone(),
                environment: self.environment.clone(),
                working_directory: self.working_directory.clone(),
                tty: self.tty,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalExecutionPlan {
    Local {
        program: String,
        args: Vec<String>,
        environment: Vec<(String, String)>,
        working_directory: Option<String>,
        tty: bool,
    },
    Remote {
        connection_id: i64,
        program: String,
        args: Vec<String>,
        environment: Vec<(String, String)>,
        working_directory: Option<String>,
        tty: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalSessionId(u64);

impl TerminalSessionId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    columns: u16,
    rows: u16,
}

impl TerminalSize {
    pub fn new(columns: u16, rows: u16) -> Result<Self, TerminalError> {
        if columns == 0 || rows == 0 {
            return Err(TerminalError::InvalidSize);
        }
        Ok(Self { columns, rows })
    }

    pub const fn columns(self) -> u16 {
        self.columns
    }

    pub const fn rows(self) -> u16 {
        self.rows
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalReadResult {
    bytes: Vec<u8>,
    closed: bool,
}

impl TerminalReadResult {
    pub fn new(bytes: Vec<u8>, closed: bool, max_bytes: usize) -> Result<Self, TerminalError> {
        if max_bytes == 0 || bytes.len() > max_bytes {
            return Err(TerminalError::InvalidReadLimit);
        }
        Ok(Self { bytes, closed })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn is_closed(&self) -> bool {
        self.closed
    }
}

/// Host-owned lifecycle for an embedded terminal. Implementations may map a
/// local plan to a PTY and a remote plan to an existing SSH connection, but the
/// UI does not need to know which transport is used.
pub trait TerminalHost {
    type Error;

    fn open(
        &mut self,
        plan: TerminalExecutionPlan,
        size: TerminalSize,
    ) -> Result<TerminalSessionId, Self::Error>;

    fn input(&mut self, session: TerminalSessionId, bytes: &[u8]) -> Result<(), Self::Error>;

    fn read(
        &mut self,
        session: TerminalSessionId,
        max_bytes: usize,
    ) -> Result<TerminalReadResult, Self::Error>;

    fn resize(&mut self, session: TerminalSessionId, size: TerminalSize)
    -> Result<(), Self::Error>;

    fn close(&mut self, session: TerminalSessionId) -> Result<(), Self::Error>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TerminalError {
    #[error("remote terminal target requires a persisted connection id")]
    InvalidRemoteConnection,
    #[error("terminal command is empty or contains a control character")]
    InvalidCommand,
    #[error("environment variable is invalid")]
    InvalidEnvironment,
    #[error("terminal size must have non-zero rows and columns")]
    InvalidSize,
    #[error("working directory is empty or contains a control character")]
    InvalidWorkingDirectory,
    #[error("terminal read limit must be non-zero and bound the returned bytes")]
    InvalidReadLimit,
}

fn valid_argument(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_plan_keeps_program_and_arguments_structured() {
        let plan = TerminalRequest::new(TerminalTarget::local(), "docker", ["ps", "--all"])
            .unwrap()
            .plan();
        let TerminalExecutionPlan::Local { program, args, .. } = plan else {
            panic!("expected local plan");
        };
        assert_eq!(program, "docker");
        assert_eq!(&args[..2], ["ps", "--all"]);
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn remote_plan_uses_host_managed_connection() {
        let plan = TerminalRequest::new(
            TerminalTarget::remote(42).unwrap(),
            "docker",
            ["exec", "web", "sh"],
        )
        .unwrap()
        .plan();
        let TerminalExecutionPlan::Remote {
            connection_id,
            program,
            args,
            ..
        } = plan
        else {
            panic!("expected remote plan");
        };
        assert_eq!(connection_id, 42);
        assert_eq!(program, "docker");
        assert_eq!(&args[..3], ["exec", "web", "sh"]);
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn invalid_values_fail_closed() {
        assert_eq!(
            TerminalTarget::remote(0).unwrap_err(),
            TerminalError::InvalidRemoteConnection
        );
        assert_eq!(
            TerminalRequest::new(TerminalTarget::local(), "", std::iter::empty::<String>())
                .unwrap_err(),
            TerminalError::InvalidCommand
        );
        assert_eq!(
            TerminalSize::new(0, 24).unwrap_err(),
            TerminalError::InvalidSize
        );
        assert_eq!(TerminalSessionId::new(0), None);
        assert_eq!(
            TerminalReadResult::new(vec![1, 2], false, 1).unwrap_err(),
            TerminalError::InvalidReadLimit
        );
    }

    #[test]
    fn arguments_are_not_shell_interpolated() {
        let plan = TerminalRequest::new(TerminalTarget::local(), "printf", ["%s", "hello world"])
            .unwrap()
            .tty(false)
            .plan();
        let TerminalExecutionPlan::Local { args, .. } = plan else {
            panic!("expected local plan");
        };
        assert_eq!(&args[..2], ["%s", "hello world"]);
    }

    #[test]
    fn working_directory_and_environment_are_preserved_in_the_plan() {
        let plan = TerminalRequest::new(TerminalTarget::local(), "tool", ["run"])
            .unwrap()
            .with_working_directory("/workspace")
            .unwrap()
            .with_environment("MODE", "interactive")
            .unwrap()
            .plan();
        let TerminalExecutionPlan::Local {
            environment,
            working_directory,
            tty,
            ..
        } = plan
        else {
            panic!("expected local plan");
        };
        assert_eq!(working_directory.as_deref(), Some("/workspace"));
        assert_eq!(environment, [("MODE".into(), "interactive".into())]);
        assert!(tty);
    }

    #[test]
    fn read_result_exposes_bounded_output_and_close_state() {
        let result = TerminalReadResult::new(b"ready\r\n".to_vec(), true, 64).unwrap();
        assert_eq!(result.bytes(), b"ready\r\n");
        assert!(result.is_closed());
    }
}
