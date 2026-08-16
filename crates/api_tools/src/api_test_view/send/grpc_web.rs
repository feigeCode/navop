use gpui::{AppContext as _, Context, Window};

use super::{ApiTestView, REQUEST_TIMEOUT_SECS, SendContext, complete_request};

impl ApiTestView {
    pub(super) fn send_grpc_web(
        &mut self,
        context: SendContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let client = cx.http_client();
        let task = cx.background_spawn(async move {
            let response = crate::grpc_web::execute(
                client.as_ref(),
                context.prepared.clone(),
                REQUEST_TIMEOUT_SECS,
            )
            .await;
            complete_request(context, response)
        });
        cx.spawn_in(window, async move |this, cx| {
            let completion = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_request(completion, window, cx);
            });
        })
        .detach();
    }
}
