use db::GlobalDbState;
use db_view::extension_menu::DbTreeExtensionActionContext;
use extension_component::{ActionContext, PermissionSet, ViewActionEvent, ViewSpec};

use crate::extension_db_gateway::ExtensionDbGateway;

use super::catalog::ExtensionRuntimeCatalog;

impl ExtensionRuntimeCatalog {
    pub async fn run_db_tree_component_action(
        &self,
        context: DbTreeExtensionActionContext,
        db_state: GlobalDbState,
    ) -> extension_wasm::WasmResult<Vec<ViewSpec>> {
        let binding = self.component_binding_for_command(&context.command_id)?;
        if binding.extension_id != context.extension_id {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                context.command_id,
            ));
        }
        let permissions = PermissionSet::new(binding.permissions.iter());
        let db_host = ExtensionDbGateway::new(binding.extension_id.clone(), permissions, db_state);
        let state = extension_wasm::ComponentHostState::new(binding.extension_id.clone(), db_host);
        let runtime = extension_wasm::ComponentRuntime::from_file(
            binding.runtime_key.clone(),
            &binding.module_path,
            binding.config.clone(),
        )?;
        runtime
            .run_action_with_db(state, component_action_context(context))
            .await
    }

    pub async fn handle_db_tree_component_view_action(
        &self,
        context: DbTreeExtensionActionContext,
        db_state: GlobalDbState,
        event: ViewActionEvent,
    ) -> extension_wasm::WasmResult<()> {
        let binding = self.component_binding_for_command(&context.command_id)?;
        if binding.extension_id != context.extension_id {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                context.command_id,
            ));
        }
        let permissions = PermissionSet::new(binding.permissions.iter());
        let db_host = ExtensionDbGateway::new(binding.extension_id.clone(), permissions, db_state);
        let state = extension_wasm::ComponentHostState::new(binding.extension_id.clone(), db_host);
        let runtime = extension_wasm::ComponentRuntime::from_file(
            binding.runtime_key.clone(),
            &binding.module_path,
            binding.config.clone(),
        )?;
        runtime
            .handle_view_action_with_db(state, component_action_context(context), event)
            .await
    }
}

fn component_action_context(context: DbTreeExtensionActionContext) -> ActionContext {
    ActionContext {
        extension_id: context.extension_id,
        command_id: context.command_id,
        node_id: context.node_id,
        node_name: context.node_name,
        node_type: context.node_type.to_string(),
        database_type: context.database_type.as_str().to_string(),
        connection_id: context.connection_id,
    }
}
