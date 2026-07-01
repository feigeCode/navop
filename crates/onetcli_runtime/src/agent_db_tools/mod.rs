mod context;
mod execution;
mod schema;

#[cfg(test)]
mod tests;

use self::context::current_database_context;
use self::schema::{
    connection_schema, execute_schema, query_schema, sample_schema, scoped_schema, table_schema,
};
use agent_runtime::{
    ResourceContext, RiskLevel, ToolError, ToolName, ToolObservation, ToolRegistry, ToolSpec,
    tools::{Tool, ToolInvocation},
};
use async_trait::async_trait;
use one_core::storage::ConnectionRepository;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum AgentDbToolKind {
    Query,
    ExecuteSql,
    ListDatabases,
    ListTables,
    DescribeTable,
    SampleRows,
}

#[derive(Clone)]
struct AgentDbTool {
    repo: Arc<ConnectionRepository>,
    kind: AgentDbToolKind,
}

pub fn register_agent_db_tools(repo: Arc<ConnectionRepository>, registry: &mut ToolRegistry) {
    register_agent_db_tool_handlers(registry, repo);
}

fn register_agent_db_tool_handlers(registry: &mut ToolRegistry, repo: Arc<ConnectionRepository>) {
    for kind in [
        AgentDbToolKind::Query,
        AgentDbToolKind::ExecuteSql,
        AgentDbToolKind::ListDatabases,
        AgentDbToolKind::ListTables,
        AgentDbToolKind::DescribeTable,
        AgentDbToolKind::SampleRows,
    ] {
        registry.register(Arc::new(AgentDbTool {
            repo: repo.clone(),
            kind,
        }));
    }
}

#[async_trait]
impl Tool for AgentDbTool {
    fn name(&self) -> ToolName {
        match self.kind {
            AgentDbToolKind::Query => ToolName::new("db_query"),
            AgentDbToolKind::ExecuteSql => ToolName::new("db_execute_sql"),
            AgentDbToolKind::ListDatabases => ToolName::new("db_list_databases"),
            AgentDbToolKind::ListTables => ToolName::new("db_list_tables"),
            AgentDbToolKind::DescribeTable => ToolName::new("db_describe_table"),
            AgentDbToolKind::SampleRows => ToolName::new("db_sample_rows"),
        }
    }

    fn spec(&self, resources: &ResourceContext) -> ToolSpec {
        let suffix = current_context_suffix(resources);
        let (description, schema, risk) = match self.kind {
            AgentDbToolKind::Query => (
                format!(
                    "Run a read-only SQL query against the current Agent database context.{suffix}"
                ),
                query_schema(),
                RiskLevel::Read,
            ),
            AgentDbToolKind::ExecuteSql => (
                format!(
                    "Execute mutating or dangerous SQL against the current Agent database context. This always requires user approval before execution.{suffix}"
                ),
                execute_schema(),
                RiskLevel::High,
            ),
            AgentDbToolKind::ListDatabases => (
                format!(
                    "List databases/catalogs for the current Agent database connection.{suffix}"
                ),
                connection_schema(),
                RiskLevel::Read,
            ),
            AgentDbToolKind::ListTables => (
                format!("List tables in the current Agent database/schema context.{suffix}"),
                scoped_schema(),
                RiskLevel::Read,
            ),
            AgentDbToolKind::DescribeTable => (
                format!(
                    "Describe columns, indexes, and foreign keys for a table in the current Agent database context.{suffix}"
                ),
                table_schema(),
                RiskLevel::Read,
            ),
            AgentDbToolKind::SampleRows => (
                format!(
                    "Read a small limited sample of rows from a table in the current Agent database context.{suffix}"
                ),
                sample_schema(),
                RiskLevel::Read,
            ),
        };
        ToolSpec::new(self.name(), description, schema).with_risk(risk)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        match self.kind {
            AgentDbToolKind::Query => self.query(invocation).await,
            AgentDbToolKind::ExecuteSql => self.execute_sql(invocation).await,
            AgentDbToolKind::ListDatabases => self.list_databases(invocation).await,
            AgentDbToolKind::ListTables => self.list_tables(invocation).await,
            AgentDbToolKind::DescribeTable => self.describe_table(invocation).await,
            AgentDbToolKind::SampleRows => self.sample_rows(invocation).await,
        }
    }
}

fn current_context_suffix(resources: &ResourceContext) -> String {
    current_database_context(resources)
        .map(|ctx| {
            format!(
                " Defaults: connection={}, database={}, schema={}.",
                ctx.connection_id,
                ctx.database.as_deref().unwrap_or("<none>"),
                ctx.schema.as_deref().unwrap_or("<none>")
            )
        })
        .unwrap_or_default()
}
