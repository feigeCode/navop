use gpui::AnyWindowHandle;
use gpui_component::DialogHandle;
use sftp::{DirectCopyApproval, DirectCopyDecision, DirectCopyPreview};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;
use tokio::sync::{Mutex, oneshot};

const PROMPT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct PromptDecisionState {
    sender: StdMutex<Option<oneshot::Sender<DirectCopyDecision>>>,
}

pub(crate) type PromptDecisionSender = Arc<PromptDecisionState>;

pub(crate) struct DirectCopyPromptRequest {
    pub(crate) task_id: usize,
    pub(crate) preview: DirectCopyPreview,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) response: PromptDecisionSender,
}

pub(crate) struct ActiveDirectCopyPrompt {
    pub(crate) task_id: usize,
    pub(crate) dialog_handle: DialogHandle,
    pub(crate) window_handle: AnyWindowHandle,
    pub(crate) response: Weak<PromptDecisionState>,
}

pub(crate) fn direct_copy_approval_bridge(
    task_id: usize,
    cancelled: Arc<AtomicBool>,
    prompt_lock: Arc<Mutex<()>>,
) -> (
    DirectCopyApproval,
    oneshot::Receiver<DirectCopyPromptRequest>,
) {
    let (request_tx, request_rx) = oneshot::channel();
    let request_tx = Arc::new(StdMutex::new(Some(request_tx)));
    let approval = Arc::new(move |preview| {
        let cancelled = cancelled.clone();
        let prompt_lock = prompt_lock.clone();
        let request_tx = request_tx.clone();
        Box::pin(async move {
            approve_direct_copy(task_id, preview, cancelled, prompt_lock, request_tx).await
        }) as _
    });
    (approval, request_rx)
}

async fn approve_direct_copy(
    task_id: usize,
    preview: DirectCopyPreview,
    cancelled: Arc<AtomicBool>,
    prompt_lock: Arc<Mutex<()>>,
    request_tx: Arc<StdMutex<Option<oneshot::Sender<DirectCopyPromptRequest>>>>,
) -> DirectCopyDecision {
    let Some(_guard) = acquire_prompt_lock(prompt_lock, &cancelled).await else {
        return DirectCopyDecision::UseRelay;
    };
    if cancelled.load(Ordering::Relaxed) {
        return DirectCopyDecision::UseRelay;
    }

    let (response_tx, response_rx) = oneshot::channel();
    let response = Arc::new(PromptDecisionState {
        sender: StdMutex::new(Some(response_tx)),
    });
    let request = DirectCopyPromptRequest {
        task_id,
        preview,
        cancelled: cancelled.clone(),
        response,
    };
    if !send_prompt_request(&request_tx, request) {
        return DirectCopyDecision::UseRelay;
    }
    wait_for_prompt_response(response_rx, &cancelled).await
}

async fn acquire_prompt_lock(
    prompt_lock: Arc<Mutex<()>>,
    cancelled: &AtomicBool,
) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        if let Ok(guard) = prompt_lock.clone().try_lock_owned() {
            return Some(guard);
        }
        tokio::time::sleep(PROMPT_POLL_INTERVAL).await;
    }
}

fn send_prompt_request(
    sender: &StdMutex<Option<oneshot::Sender<DirectCopyPromptRequest>>>,
    request: DirectCopyPromptRequest,
) -> bool {
    sender
        .lock()
        .ok()
        .and_then(|mut sender| sender.take())
        .is_some_and(|sender| sender.send(request).is_ok())
}

async fn wait_for_prompt_response(
    mut receiver: oneshot::Receiver<DirectCopyDecision>,
    cancelled: &AtomicBool,
) -> DirectCopyDecision {
    loop {
        tokio::select! {
            decision = &mut receiver => {
                return decision.unwrap_or(DirectCopyDecision::UseRelay);
            }
            () = tokio::time::sleep(PROMPT_POLL_INTERVAL) => {
                if cancelled.load(Ordering::Relaxed) {
                    return DirectCopyDecision::UseRelay;
                }
            }
        }
    }
}

pub(crate) fn send_prompt_decision(sender: &PromptDecisionSender, decision: DirectCopyDecision) {
    if let Ok(mut sender) = sender.sender.lock()
        && let Some(sender) = sender.take()
    {
        let _ = sender.send(decision);
    }
}

#[cfg(test)]
mod tests {
    use super::{direct_copy_approval_bridge, send_prompt_decision};
    use sftp::{DirectCopyDecision, DirectCopyPreview, DirectCopyStrategy, ServerCopyAuthKind};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn dropped_prompt_request_defaults_to_relay() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (approval, request) =
            direct_copy_approval_bridge(7, cancelled, Arc::new(Mutex::new(())));
        drop(request);

        assert_eq!(DirectCopyDecision::UseRelay, approval(preview()).await);
    }

    #[tokio::test]
    async fn prompt_response_is_returned_to_core() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (approval, request) =
            direct_copy_approval_bridge(9, cancelled, Arc::new(Mutex::new(())));
        let approval_task = tokio::spawn(async move { approval(preview()).await });
        let request = request.await.expect("prompt request should be delivered");

        assert_eq!(9, request.task_id);
        send_prompt_decision(&request.response, DirectCopyDecision::UseDirect);
        assert_eq!(
            DirectCopyDecision::UseDirect,
            approval_task.await.expect("approval task should complete")
        );
    }

    #[tokio::test]
    async fn explicit_cancel_is_returned_to_core() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (approval, request) =
            direct_copy_approval_bridge(11, cancelled, Arc::new(Mutex::new(())));
        let approval_task = tokio::spawn(async move { approval(preview()).await });
        let request = request.await.expect("prompt request should be delivered");

        send_prompt_decision(&request.response, DirectCopyDecision::Cancel);
        assert_eq!(
            DirectCopyDecision::Cancel,
            approval_task.await.expect("approval task should complete")
        );
    }

    fn preview() -> DirectCopyPreview {
        DirectCopyPreview {
            strategy: DirectCopyStrategy::Rsync,
            source_host: "source.example".to_string(),
            source_port: 22,
            source_username: "source-user".to_string(),
            navop_source_auth: ServerCopyAuthKind::Password,
            target_host: "target.example".to_string(),
            target_port: 2222,
            target_username: "target-user".to_string(),
            navop_target_auth: ServerCopyAuthKind::PrivateKeyFile,
            target_auth_has_passphrase: true,
            target_auth_has_certificate: true,
            item_count: 2,
        }
    }
}
