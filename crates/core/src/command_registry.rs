use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub id: String,
    pub title: String,
    pub handler: CommandHandler,
    pub extension_id: Option<String>,
    pub category: Option<String>,
    pub icon: Option<String>,
    pub enablement_when: Option<String>,
}

impl CommandDescriptor {
    pub fn wasm(id: impl Into<String>, title: impl Into<String>, handler: CommandHandler) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            handler,
            extension_id: None,
            category: None,
            icon: None,
            enablement_when: None,
        }
    }

    pub fn with_extension(mut self, extension_id: impl Into<String>) -> Self {
        self.extension_id = Some(extension_id.into());
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_enablement(mut self, when: impl Into<String>) -> Self {
        self.enablement_when = Some(when.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandHandler {
    Builtin {
        command: String,
    },
    Wasm {
        runtime_id: String,
        function: String,
    },
}

impl CommandHandler {
    pub fn builtin(command: impl Into<String>) -> Self {
        Self::Builtin {
            command: command.into(),
        }
    }

    pub fn wasm(runtime_id: impl Into<String>, function: impl Into<String>) -> Self {
        Self::Wasm {
            runtime_id: runtime_id.into(),
            function: function.into(),
        }
    }
}

#[derive(Debug, Default)]
pub struct CommandRegistry {
    commands: BTreeMap<String, CommandDescriptor>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, command: CommandDescriptor) -> Result<(), CommandRegistryError> {
        if self.commands.contains_key(&command.id) {
            return Err(CommandRegistryError::DuplicateCommand {
                id: command.id.clone(),
            });
        }
        self.commands.insert(command.id.clone(), command);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&CommandDescriptor> {
        self.commands.get(id)
    }

    pub fn list(&self) -> impl Iterator<Item = &CommandDescriptor> {
        self.commands.values()
    }

    pub fn unregister_extension(&mut self, extension_id: &str) {
        self.commands
            .retain(|_, command| command.extension_id.as_deref() != Some(extension_id));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandRegistryError {
    #[error("duplicate command id: {id}")]
    DuplicateCommand { id: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_registers_and_finds_command() {
        let mut registry = CommandRegistry::new();
        let command = CommandDescriptor::wasm(
            "example.echo",
            "Echo",
            CommandHandler::wasm("main", "invoke"),
        );
        registry.register(command.clone()).unwrap();
        assert_eq!(Some(&command), registry.get("example.echo"));
    }

    #[test]
    fn registry_rejects_duplicate_command_ids() {
        let mut registry = CommandRegistry::new();
        registry
            .register(CommandDescriptor::wasm(
                "example.echo",
                "Echo",
                CommandHandler::wasm("main", "invoke"),
            ))
            .unwrap();
        let err = registry
            .register(CommandDescriptor::wasm(
                "example.echo",
                "Echo 2",
                CommandHandler::wasm("main", "invoke"),
            ))
            .unwrap_err();
        assert!(err.to_string().contains("duplicate command"));
    }

    #[test]
    fn registry_unregisters_commands_by_extension() {
        let mut registry = CommandRegistry::new();
        registry
            .register(
                CommandDescriptor::wasm("ext.a", "A", CommandHandler::wasm("ext::main", "invoke"))
                    .with_extension("ext.one"),
            )
            .unwrap();
        registry
            .register(
                CommandDescriptor::wasm("ext.b", "B", CommandHandler::wasm("ext::main", "invoke"))
                    .with_extension("ext.two"),
            )
            .unwrap();

        registry.unregister_extension("ext.one");

        assert!(registry.get("ext.a").is_none());
        assert!(registry.get("ext.b").is_some());
    }

    #[test]
    fn command_descriptor_tracks_enablement_when_clause() {
        let command = CommandDescriptor::wasm(
            "ext.refresh",
            "Refresh",
            CommandHandler::wasm("ext::main", "invoke"),
        )
        .with_extension("ext.one")
        .with_enablement("connection.kind == 'duckdb'");

        assert_eq!(Some("ext.one"), command.extension_id.as_deref());
        assert_eq!(
            Some("connection.kind == 'duckdb'"),
            command.enablement_when.as_deref()
        );
    }
}
