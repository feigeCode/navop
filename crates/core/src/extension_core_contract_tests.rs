use serde_json::json;

use crate::{
    command_registry::{CommandDescriptor, CommandHandler, CommandRegistry},
    contributions::{ContributionProvenance, SlotItem, SlotRegistry},
    when_clause::{WhenContext, evaluate},
};

#[test]
fn command_registry_rejects_duplicate_ids() {
    let mut registry = CommandRegistry::new();
    let command = CommandDescriptor::wasm(
        "example.echo",
        "Echo",
        CommandHandler::wasm("example::main", "invoke"),
    );

    registry.register(command).unwrap();
    let error = registry
        .register(CommandDescriptor::wasm(
            "example.echo",
            "Echo Again",
            CommandHandler::wasm("example::main", "invoke"),
        ))
        .unwrap_err();

    assert!(error.to_string().contains("duplicate command"));
}

#[test]
fn slot_registry_returns_items_sorted_by_group_order() {
    let mut registry = SlotRegistry::default();
    registry.add("db.tree.table", slot_item("late", "extension@20"));
    registry.add("db.tree.table", slot_item("early", "extension@10"));

    let items = registry.items("db.tree.table");

    assert_eq!(
        vec!["early", "late"],
        items
            .iter()
            .map(|item| item.command.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn when_clause_evaluates_context_paths_and_boolean_operators() {
    let context = WhenContext::from_json(json!({
        "connection": { "kind": "postgresql" },
        "node": { "type": "table" },
        "selection": { "empty": false }
    }));

    assert!(evaluate("connection.kind == 'postgresql'", &context).unwrap());
    assert!(
        evaluate(
            "node.type in ['table', 'view'] && !selection.empty",
            &context
        )
        .unwrap()
    );
    assert!(!evaluate("node.type == 'schema' || selection.empty", &context).unwrap());
}

fn slot_item(command: &str, group: &str) -> SlotItem {
    SlotItem {
        command: command.to_string(),
        label: Some(command.to_string()),
        icon: None,
        group: Some(group.to_string()),
        when: None,
        args: serde_json::Value::Null,
        provenance: ContributionProvenance {
            extension_id: "ext.test".to_string(),
        },
    }
}
