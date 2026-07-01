mod config;
mod execution;
mod output;
mod schema;

use self::schema::{path_schema, read_schema, write_schema};
use agent_runtime::{
    ResourceContext, ResourceKind, RiskLevel, ToolError, ToolName, ToolObservation, ToolRegistry,
    ToolSpec,
    tools::{Tool, ToolInvocation},
};
use async_trait::async_trait;
use one_core::storage::ConnectionRepository;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum AgentSshToolKind {
    ListDir,
    ReadFile,
    FileStat,
    WriteFile,
}

#[derive(Clone)]
struct AgentSshTool {
    repo: Arc<ConnectionRepository>,
    kind: AgentSshToolKind,
}

pub fn register_agent_ssh_tools(repo: Arc<ConnectionRepository>, registry: &mut ToolRegistry) {
    for kind in [
        AgentSshToolKind::ListDir,
        AgentSshToolKind::ReadFile,
        AgentSshToolKind::FileStat,
        AgentSshToolKind::WriteFile,
    ] {
        registry.register(Arc::new(AgentSshTool {
            repo: repo.clone(),
            kind,
        }));
    }
}

#[async_trait]
impl Tool for AgentSshTool {
    fn name(&self) -> ToolName {
        match self.kind {
            AgentSshToolKind::ListDir => ToolName::new("ssh_list_dir"),
            AgentSshToolKind::ReadFile => ToolName::new("ssh_read_file"),
            AgentSshToolKind::FileStat => ToolName::new("ssh_file_stat"),
            AgentSshToolKind::WriteFile => ToolName::new("ssh_write_file"),
        }
    }

    fn spec(&self, resources: &ResourceContext) -> ToolSpec {
        let suffix = current_context_suffix(resources);
        let (description, schema, risk) = match self.kind {
            AgentSshToolKind::ListDir => (
                format!(
                    "List a remote directory through the current Agent SSH/SFTP context.{suffix}"
                ),
                path_schema(),
                RiskLevel::Read,
            ),
            AgentSshToolKind::ReadFile => (
                format!(
                    "Read a bounded remote file through the current Agent SSH/SFTP context.{suffix}"
                ),
                read_schema(),
                RiskLevel::Read,
            ),
            AgentSshToolKind::FileStat => (
                format!(
                    "Inspect a remote path through the current Agent SSH/SFTP context.{suffix}"
                ),
                path_schema(),
                RiskLevel::Read,
            ),
            AgentSshToolKind::WriteFile => (
                format!(
                    "Write a remote file through the current Agent SSH/SFTP context. This always requires user approval before execution.{suffix}"
                ),
                write_schema(),
                RiskLevel::High,
            ),
        };
        ToolSpec::new(self.name(), description, schema).with_risk(risk)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        self.execute_sftp(invocation).await
    }
}

fn current_context_suffix(resources: &ResourceContext) -> String {
    let Some(resource) = resources.current() else {
        return String::new();
    };
    if resource.kind != ResourceKind::Ssh {
        return String::new();
    }
    format!(" Defaults: connection={}.", resource.id)
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Execution(error.to_string())
}
