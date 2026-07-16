use anyhow::Result;
use cditor_app::{
    AiCancellationToken, AiModelDescriptor, AiProvider, AiProviderError,
    AiRequest as AiProviderRequest, AiStreamEvent, AiStreamSender, AiTaskKind,
};
use futures::StreamExt;
use gpui::App;
use one_core::gpui_tokio::Tokio;
use one_core::llm::{
    ChatRequest, GlobalProviderState, Message, ProviderManager, Role, extract_stream_text_parts,
};
use one_core::settings::GlobalCurrentUser;
use one_core::storage::GlobalStorageState;
use one_core::storage::traits::Repository;
use rust_i18n::t;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;
use tokio::runtime::Handle;

use crate::ai_model_catalog::{ModelCatalog, ModelRoute, build_catalog};

const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(50);

struct StreamTaskInput {
    manager: Arc<ProviderManager>,
    route: ModelRoute,
    request: ChatRequest,
    request_id: u64,
    sender: AiStreamSender,
    cancellation: AiCancellationToken,
}

pub(crate) struct NavopAiProvider {
    manager: Arc<ProviderManager>,
    runtime: Handle,
    catalog: Arc<ModelCatalog>,
}

pub(crate) fn build_provider(cx: &App) -> Result<Option<Arc<dyn AiProvider>>> {
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        return Ok(None);
    };
    let Some(repository) = storage
        .storage
        .get::<one_core::llm::storage::ProviderRepository>()
    else {
        return Ok(None);
    };
    let logged_in = GlobalCurrentUser::get_user(cx).is_some();
    if logged_in {
        if let Err(error) = repository.ensure_onetcli_provider() {
            tracing::warn!(%error, "failed to ensure built-in Notes AI provider");
        }
    }
    let mut configs = repository.list()?;
    if !logged_in {
        configs.retain(|config| !config.is_builtin());
    }
    let catalog = build_catalog(configs);
    if catalog.descriptors.is_empty() {
        return Ok(None);
    }
    let manager = cx
        .try_global::<GlobalProviderState>()
        .map(GlobalProviderState::manager)
        .ok_or_else(|| anyhow::anyhow!(t!("Notes.ai_provider_unavailable").to_string()))?;
    Ok(Some(Arc::new(NavopAiProvider {
        manager,
        runtime: Tokio::handle(cx),
        catalog: Arc::new(catalog),
    })))
}

impl AiProvider for NavopAiProvider {
    fn id(&self) -> &str {
        "navop"
    }

    fn models(&self) -> Vec<AiModelDescriptor> {
        self.catalog.descriptors.clone()
    }

    fn default_model_id(&self) -> Option<String> {
        self.catalog.default_model_id.clone()
    }

    fn stream(
        &self,
        request: AiProviderRequest,
        sender: AiStreamSender,
        cancellation: AiCancellationToken,
    ) -> Result<(), AiProviderError> {
        let model_id = request
            .model_id
            .clone()
            .or_else(|| self.default_model_id())
            .ok_or_else(|| {
                AiProviderError::Request(t!("Notes.ai_model_not_selected").to_string())
            })?;
        let route = self.catalog.routes.get(&model_id).cloned().ok_or_else(|| {
            AiProviderError::Request(t!("Notes.ai_model_unavailable").to_string())
        })?;
        let chat_request = chat_request(&route, &request);
        let (completion_sender, completion_receiver) = mpsc::sync_channel(1);
        let task = spawn_stream_task(
            self.runtime.clone(),
            StreamTaskInput {
                manager: self.manager.clone(),
                route,
                request: chat_request,
                request_id: request.request_id,
                sender: sender.clone(),
                cancellation: cancellation.clone(),
            },
            completion_sender,
        );
        wait_for_completion(task, completion_receiver, cancellation)
    }
}

fn spawn_stream_task(
    runtime: Handle,
    input: StreamTaskInput,
    completion_sender: SyncSender<Result<(), AiProviderError>>,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        let result = stream_from_provider(input).await;
        let _ = completion_sender.send(result);
    })
}

async fn stream_from_provider(input: StreamTaskInput) -> Result<(), AiProviderError> {
    let provider = input
        .manager
        .get_provider(&input.route.config)
        .await
        .map_err(|error| AiProviderError::Request(error.to_string()))?;
    let mut stream = provider
        .chat_stream(&input.request)
        .await
        .map_err(|error| AiProviderError::Request(error.to_string()))?;
    while let Some(response) = stream.next().await {
        if input.cancellation.is_cancelled() {
            return Err(AiProviderError::Cancelled);
        }
        let response = response.map_err(|error| AiProviderError::Protocol(error.to_string()))?;
        if let Some(content) = extract_stream_text_parts(&response).content {
            input
                .sender
                .send(AiStreamEvent::Delta {
                    request_id: input.request_id,
                    text: content.to_owned(),
                })
                .await
                .map_err(|_| AiProviderError::ChannelClosed)?;
        }
    }
    if input.cancellation.is_cancelled() {
        return Err(AiProviderError::Cancelled);
    }
    input
        .sender
        .send(AiStreamEvent::Done {
            request_id: input.request_id,
        })
        .await
        .map_err(|_| AiProviderError::ChannelClosed)
}

fn wait_for_completion(
    task: tokio::task::JoinHandle<()>,
    receiver: Receiver<Result<(), AiProviderError>>,
    cancellation: AiCancellationToken,
) -> Result<(), AiProviderError> {
    loop {
        if cancellation.is_cancelled() {
            task.abort();
            return Err(AiProviderError::Cancelled);
        }
        match receiver.recv_timeout(STREAM_POLL_INTERVAL) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(AiProviderError::ChannelClosed);
            }
        }
    }
}

fn chat_request(route: &ModelRoute, request: &AiProviderRequest) -> ChatRequest {
    let task = task_instruction(request.task);
    let prompt = format!(
        "Task: {task}\nInstruction: {}\nSelected text:\n{}\nContext before:\n{}\nContext after:\n{}",
        request.instruction, request.selected_text, request.prefix, request.suffix,
    );
    ChatRequest {
        model: route.model.clone(),
        messages: vec![
            Message::text(
                Role::System,
                "You are an inline writing assistant embedded in a rich text editor. Return only insertion or replacement text. Do not include commentary, labels, or markdown fences unless requested.",
            ),
            Message::text(Role::User, prompt),
        ],
        max_tokens: route
            .config
            .max_tokens
            .and_then(|value| u32::try_from(value).ok()),
        temperature: route.config.temperature,
        ..Default::default()
    }
}

fn task_instruction(task: AiTaskKind) -> &'static str {
    match task {
        AiTaskKind::InlineCompletion => "Continue at the caret. Return only inserted text.",
        AiTaskKind::RewriteSelection => "Rewrite the selection. Return only replacement text.",
        AiTaskKind::RewriteBlocks => {
            "Rewrite the selected blocks. Preserve paragraph boundaries and return only replacement text."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::llm::{ProviderConfig, ProviderType};

    fn config() -> ProviderConfig {
        ProviderConfig {
            id: 7,
            name: "DeepSeek".to_owned(),
            provider_type: ProviderType::DeepSeek,
            model: "deepseek-chat".to_owned(),
            models: vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()],
            is_default: true,
            ..Default::default()
        }
    }

    #[test]
    fn prompt_keeps_editor_context_and_task() {
        let request = AiProviderRequest {
            request_id: 1,
            task: AiTaskKind::RewriteSelection,
            model_id: None,
            instruction: "改善表达".to_owned(),
            selected_text: "原文".to_owned(),
            prefix: "前文".to_owned(),
            suffix: "后文".to_owned(),
        };
        let route = ModelRoute {
            config: config(),
            model: "deepseek-chat".to_owned(),
        };
        let prompt = chat_request(&route, &request);
        assert!(prompt.messages[1].content_as_text().contains("原文"));
        assert!(
            prompt.messages[1]
                .content_as_text()
                .contains("Rewrite the selection")
        );
    }
}
