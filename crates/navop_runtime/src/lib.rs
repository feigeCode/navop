pub mod connections;
#[cfg(feature = "database")]
pub use runtime_database::database_tools;
#[cfg(feature = "redis")]
pub use runtime_redis::redis_tools;
#[cfg(feature = "sftp")]
pub use runtime_sftp::sftp_tools;

pub use runtime_core::workspaces;
pub use runtime_core::builtin_tool_registry_with_version;

pub fn builtin_tool_registry() -> tool_runtime::ToolRegistry {
    builtin_tool_registry_with_version(env!("CARGO_PKG_VERSION"))
}

pub fn tool_registry(
    repo: std::sync::Arc<one_core::storage::ConnectionRepository>,
) -> Result<tool_runtime::ToolRegistry, tool_runtime::ToolRegistryError> {
    tool_registry_with_version(repo, env!("CARGO_PKG_VERSION"))
}

pub fn tool_registry_with_version(
    repo: std::sync::Arc<one_core::storage::ConnectionRepository>,
    version: &'static str,
) -> Result<tool_runtime::ToolRegistry, tool_runtime::ToolRegistryError> {
    #[allow(unused_mut)]
    let mut registries = vec![
        builtin_tool_registry_with_version(version),
        connections::connection_tool_registry(repo.clone()),
    ];
    #[cfg(feature = "database")]
    registries.push(database_tools::database_tool_registry(repo.clone()));
    #[cfg(feature = "redis")]
    registries.push(redis_tools::redis_tool_registry(repo));
    tool_runtime::ToolRegistry::merge(registries)
}

#[cfg(test)]
mod embedded_cli_removal_tests {
    #[test]
    fn runtime_has_no_embedded_command_module() {
        let source = include_str!("lib.rs");
        let manifest = include_str!("../Cargo.toml");
        let module_declaration = ["pub mod cli", "host;"].join("_");
        let cli_dependency = ["onetcli", "cli"].join("_");

        assert!(!source.contains(&module_declaration));
        assert!(!manifest.contains(&format!("{cli_dependency} =")));
    }
}
