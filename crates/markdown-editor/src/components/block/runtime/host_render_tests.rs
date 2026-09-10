use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::oneshot;
use gpui::{AppContext, TestAppContext, rgba};

use crate::components::{Block, BlockRecord};
use crate::{
    BlockRenderArtifact, BlockRenderKind, BlockRenderProvider, EditorHostServices, EditorHostTheme,
};

fn svg_artifact(marker: u8) -> BlockRenderArtifact {
    BlockRenderArtifact {
        media_type: "image/svg+xml".to_string(),
        bytes: vec![marker],
        intrinsic_width: Some(120.0),
        intrinsic_height: Some(40.0),
    }
}

fn provider_with_counter(
    calls: Arc<AtomicUsize>,
    result: Result<Option<BlockRenderArtifact>, String>,
) -> BlockRenderProvider {
    Arc::new(move |_request| {
        calls.fetch_add(1, Ordering::SeqCst);
        let result = result.clone();
        Box::pin(async move { result })
    })
}

fn install_provider(block: &mut Block, provider: BlockRenderProvider, theme: EditorHostTheme) {
    block.set_host_services(Arc::new(
        EditorHostServices::new(theme).with_block_renderer(provider),
    ));
    block.set_host_render_environment(480.0, 2.0);
}

#[gpui::test]
async fn host_request_uses_installed_theme_and_render_metrics(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("math")));
    let theme = EditorHostTheme {
        background: rgba(0x112233ff).into(),
        foreground: rgba(0x223344ff).into(),
        border: rgba(0x334455ff).into(),
        muted: rgba(0x445566ff).into(),
        accent: rgba(0x556677ff).into(),
    };

    block.update(cx, |block, _cx| {
        block.set_host_services(Arc::new(EditorHostServices::new(theme.clone())));
        block.set_host_render_environment(512.5, 1.75);
        let request = block.host_render_request(BlockRenderKind::InlineMath, "x + y".to_string());

        assert_eq!(request.kind, BlockRenderKind::InlineMath);
        assert_eq!(request.source, "x + y");
        assert_eq!(request.background, theme.background);
        assert_eq!(request.foreground, theme.foreground);
        assert_eq!(request.border, theme.border);
        assert_eq!(request.muted, theme.muted);
        assert_eq!(request.accent, theme.accent);
        assert_eq!(request.available_width, 512.5);
        assert_eq!(request.scale_factor, 1.75);
    });
}

#[gpui::test]
async fn pending_request_is_deduplicated_and_ready_artifact_is_reused(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = provider_with_counter(calls.clone(), Ok(Some(svg_artifact(7))));
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("math")));

    block.update(cx, |block, cx| {
        install_provider(block, provider, EditorHostTheme::default());
        let request = block.host_render_request(BlockRenderKind::Math, "x".to_string());
        assert!(block.resolve_host_render(request.clone(), cx).is_none());
        assert!(block.host_render_is_pending(&request));
        assert!(block.resolve_host_render(request, cx).is_none());
    });
    cx.run_until_parked();

    block.update(cx, |block, cx| {
        let request = block.host_render_request(BlockRenderKind::Math, "x".to_string());
        let artifact = block
            .resolve_host_render(request, cx)
            .expect("completed SVG should be cached");
        assert_eq!(artifact.artifact.bytes, vec![7]);
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[gpui::test]
async fn failed_or_unsupported_results_do_not_retry_each_frame(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let calls = Arc::new(AtomicUsize::new(0));
    let unsupported = BlockRenderArtifact {
        media_type: "text/plain".to_string(),
        bytes: b"not svg".to_vec(),
        intrinsic_width: None,
        intrinsic_height: None,
    };
    let provider = provider_with_counter(calls.clone(), Ok(Some(unsupported)));
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("diagram")));

    block.update(cx, |block, cx| {
        install_provider(block, provider, EditorHostTheme::default());
        let request = block.host_render_request(BlockRenderKind::Mermaid, "graph TD".to_string());
        assert!(block.resolve_host_render(request.clone(), cx).is_none());
        assert!(block.host_render_is_pending(&request));
    });
    cx.run_until_parked();

    block.update(cx, |block, cx| {
        let request = block.host_render_request(BlockRenderKind::Mermaid, "graph TD".to_string());
        assert!(block.resolve_host_render(request.clone(), cx).is_none());
        assert!(!block.host_render_is_pending(&request));
    });
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[gpui::test]
async fn distinct_inline_requests_complete_independently(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_provider = calls.clone();
    let provider: BlockRenderProvider = Arc::new(move |request| {
        calls_for_provider.fetch_add(1, Ordering::SeqCst);
        let marker = request.source.as_bytes()[0];
        Box::pin(async move { Ok(Some(svg_artifact(marker))) })
    });
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("inline")));

    block.update(cx, |block, cx| {
        install_provider(block, provider, EditorHostTheme::default());
        for source in ["alpha", "beta"] {
            let request =
                block.host_render_request(BlockRenderKind::InlineMath, source.to_string());
            assert!(block.resolve_host_render(request, cx).is_none());
        }
    });
    cx.run_until_parked();

    block.update(cx, |block, cx| {
        for (source, marker) in [("alpha", b'a'), ("beta", b'b')] {
            let request =
                block.host_render_request(BlockRenderKind::InlineMath, source.to_string());
            let artifact = block
                .resolve_host_render(request, cx)
                .expect("each inline request should complete");
            assert_eq!(artifact.artifact.bytes, vec![marker]);
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[gpui::test]
async fn services_reset_discards_completion_from_previous_provider(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let (sender, receiver) = oneshot::channel();
    let receiver = Arc::new(Mutex::new(Some(receiver)));
    let old_provider: BlockRenderProvider = Arc::new(move |_request| {
        let receiver = receiver
            .lock()
            .expect("receiver lock should not be poisoned")
            .take()
            .expect("old provider should only run once");
        Box::pin(async move { receiver.await.expect("test sender should complete") })
    });
    let new_calls = Arc::new(AtomicUsize::new(0));
    let new_provider = provider_with_counter(new_calls.clone(), Ok(Some(svg_artifact(2))));
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("math")));

    block.update(cx, |block, cx| {
        install_provider(block, old_provider, EditorHostTheme::default());
        let request = block.host_render_request(BlockRenderKind::Math, "same".to_string());
        assert!(block.resolve_host_render(request, cx).is_none());
        install_provider(block, new_provider, EditorHostTheme::default());
    });
    sender
        .send(Ok(Some(svg_artifact(1))))
        .expect("old completion should be accepted by the channel");
    cx.run_until_parked();

    block.update(cx, |block, cx| {
        let request = block.host_render_request(BlockRenderKind::Math, "same".to_string());
        assert!(block.resolve_host_render(request, cx).is_none());
    });
    cx.run_until_parked();

    block.update(cx, |block, cx| {
        let request = block.host_render_request(BlockRenderKind::Math, "same".to_string());
        let artifact = block
            .resolve_host_render(request, cx)
            .expect("replacement provider should win");
        assert_eq!(artifact.artifact.bytes, vec![2]);
    });
    assert_eq!(new_calls.load(Ordering::SeqCst), 1);
}
