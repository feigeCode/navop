use gpui_shell::{HostModule, HostObject, HostValue};

#[derive(Clone)]
pub(crate) struct ShellConnectionContext {
    pub connection_id: i64,
    pub name: String,
    pub contribution_id: String,
    pub resource_type: String,
    pub resource: HostValue,
}

pub(super) fn context_module(
    contribution: &extension_runtime::RegisteredShellViewContribution,
    connection: Option<ShellConnectionContext>,
) -> HostModule {
    let extension_id = contribution.extension_id.clone();
    let view_id = contribution.id.clone();
    let backends = contribution.backends.keys().cloned().collect::<Vec<_>>();
    let connection = connection
        .map(|connection| {
            HostObject::new()
                .field("id", connection.connection_id)
                .field("name", connection.name)
                .field("contributionId", connection.contribution_id)
                .field("resourceType", connection.resource_type)
                .field("resource", connection.resource)
                .into()
        })
        .unwrap_or(HostValue::Null);
    HostModule::new("navop.context")
        .declarations(
            "export function current(): { extensionId: string; viewId: string; backends: string[]; connection: { id: number; name: string; contributionId: string; resourceType: string; resource: { handle: string; capabilities: string[]; metadata: unknown } } | null };",
        )
        .function("current", move |_| {
            Ok(HostObject::new()
                .field("extensionId", extension_id.clone())
                .field("viewId", view_id.clone())
                .field("backends", backends.clone())
                .field("connection", connection.clone())
                .into())
        })
}
