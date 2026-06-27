use agent_runtime::{Plan, RuntimeEvent, StepStatus, ToolObservation};
use uuid::Uuid;

use super::ChatEngine;
use crate::ai_chat::types::{ChatMessageUIGeneric, MessageExtension};

pub(super) fn apply_runtime_event<E: MessageExtension + Default>(
    engine: &mut ChatEngine<E>,
    event: RuntimeEvent,
) {
    match event {
        RuntimeEvent::PlanUpdated { plan, .. } => upsert_plan_message(engine, &plan),
        RuntimeEvent::AssistantMessageDelta { delta, .. } => {
            append_runtime_assistant_delta(engine, delta);
        }
        RuntimeEvent::AssistantMessage { text, .. } => {
            finalize_runtime_assistant_message(engine, text);
        }
        RuntimeEvent::Status { title, is_done, .. } => {
            engine.push_status(title, is_done);
        }
        RuntimeEvent::TurnFailed { reason, .. } => {
            engine.push_assistant(format!("Error: {reason}"));
            finish_loading(engine);
        }
        RuntimeEvent::TurnCompleted { .. } => finish_loading(engine),
        RuntimeEvent::ToolCallStarted { tool_name, .. } => {
            engine.push_status(format!("Calling tool `{tool_name}`"), false);
        }
        RuntimeEvent::ToolCallFinished { success, .. } => {
            engine.push_status("Tool call finished", success);
        }
        RuntimeEvent::ObservationAdded { observation, .. } => {
            engine.push_assistant(format_tool_observation(&observation));
        }
        RuntimeEvent::NeedUserInput { question, .. } => {
            engine.push_assistant(format!("Input required: {question}"));
            finish_loading(engine);
        }
        RuntimeEvent::TurnStarted { .. } => {}
    }
}

fn upsert_plan_message<E: MessageExtension + Default>(engine: &mut ChatEngine<E>, plan: &Plan) {
    let content = format_plan_markdown(plan);
    if let Some(id) = engine.plan_message_id.as_deref()
        && let Some(message) = engine.messages.iter_mut().find(|message| message.id == id)
    {
        message.content = content;
        return;
    }

    let id = Uuid::new_v4().to_string();
    engine
        .messages
        .push(ChatMessageUIGeneric::assistant(content).with_id(id.clone()));
    engine.plan_message_id = Some(id);
}

fn append_runtime_assistant_delta<E: MessageExtension + Default>(
    engine: &mut ChatEngine<E>,
    delta: String,
) {
    let id = engine
        .runtime_assistant_message_id
        .clone()
        .unwrap_or_else(|| {
            let id = engine.push_streaming_assistant();
            engine.runtime_assistant_message_id = Some(id.clone());
            id
        });
    if let Some(message) = engine.messages.iter_mut().find(|message| message.id == id) {
        message.content.push_str(&delta);
    }
}

fn finalize_runtime_assistant_message<E: MessageExtension + Default>(
    engine: &mut ChatEngine<E>,
    text: String,
) {
    if let Some(id) = engine.runtime_assistant_message_id.take() {
        engine.finalize_streaming(&id, text);
    } else if !text.is_empty() {
        engine.push_assistant(text);
    }
    finish_loading(engine);
}

fn finish_loading<E: MessageExtension + Default>(engine: &mut ChatEngine<E>) {
    engine.is_loading = false;
    engine.cancel_token = None;
}

fn format_plan_markdown(plan: &Plan) -> String {
    let mut output = format!("### Plan\n\n**Goal:** {}\n\n", plan.goal);
    if plan.steps.is_empty() {
        output.push_str("_No steps yet._");
        return output;
    }

    for step in &plan.steps {
        output.push_str(&format!(
            "- [{}] {}\n",
            step_status_marker(step.status),
            step.title
        ));
        if !step.description.trim().is_empty() {
            output.push_str(&format!("  - {}\n", step.description));
        }
    }
    output
}

fn step_status_marker(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => " ",
        StepStatus::Running => "~",
        StepStatus::Observed => "-",
        StepStatus::Skipped => "s",
        StepStatus::Failed => "!",
        StepStatus::Completed => "x",
    }
}

fn format_tool_observation(observation: &ToolObservation) -> String {
    let status = if observation.success {
        "Tool succeeded"
    } else {
        "Tool failed"
    };
    format!(
        "**{status}: `{}`**\n\n{}",
        observation.tool_name, observation.summary
    )
}
