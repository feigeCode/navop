#![cfg(target_os = "macos")]

use gpui::{AppContext, Bounds, QuitMode, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, highlighter::HighlightTheme};
use gpui_component_assets::Assets;
use markdown_editor::{
    MarkdownBlockRenderArtifact, MarkdownBlockRenderKind, MarkdownBlockRenderProvider,
    MarkdownEditor, MarkdownEditorTheme,
};
use std::sync::Arc;

fn main() {
    gpui_platform::application()
        .with_assets(Assets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            gpui_component::init(cx);
            markdown_editor::init(cx);

            let task_list_id = markdown_source::SourceMarkdownDocument::parse(COMBINED_SAMPLE)
                .unwrap()
                .blocks
                .iter()
                .find(|block| matches!(block.kind, markdown_source::SourceBlockKind::UnorderedList))
                .unwrap()
                .id;

            let preview_bounds = Bounds::centered(None, size(px(1060.), px(820.)), cx);
            let preview_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(preview_bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Markdown 预览态验收".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(760.), px(560.))),
                ..Default::default()
            };
            cx.open_window(preview_options, |window, cx| {
                window.activate_window();
                let editor = cx.new(|cx| {
                    let mut editor =
                        MarkdownEditor::new(COMBINED_SAMPLE, light_theme(), window, cx).unwrap();
                    editor.set_block_render_provider(Some(audit_render_provider()), cx);
                    editor
                });
                cx.new(|cx| Root::new(editor, window, cx))
            })
            .expect("open Markdown preview audit window");

            let active_bounds = Bounds::centered(None, size(px(1060.), px(820.)), cx);
            let active_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(active_bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Markdown 任务列表编辑态验收".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(760.), px(560.))),
                ..Default::default()
            };
            cx.open_window(active_options, move |window, cx| {
                window.activate_window();
                let editor = cx.new(|cx| {
                    let mut editor =
                        MarkdownEditor::new(COMBINED_SAMPLE, light_theme(), window, cx).unwrap();
                    editor.set_block_render_provider(Some(audit_render_provider()), cx);
                    editor
                });
                editor.update(cx, |editor, cx| {
                    assert!(editor.activate_block(task_list_id, window, cx));
                });
                cx.new(|cx| Root::new(editor, window, cx))
            })
            .expect("open Markdown active task list audit window");
        });
}

/// A deterministic in-process provider keeps this example independent from
/// installed extensions while still exercising the real asynchronous
/// Math/Mermaid artifact path and its permanent source/rendered layers.
fn audit_render_provider() -> MarkdownBlockRenderProvider {
    Arc::new(|request| {
        Box::pin(async move {
            let (svg, width, height) = match request.kind {
                MarkdownBlockRenderKind::Math => (MATH_SVG, 360., 120.),
                MarkdownBlockRenderKind::Mermaid => (MERMAID_SVG, 620., 164.),
            };
            Ok(Some(MarkdownBlockRenderArtifact {
                media_type: "image/svg+xml".to_owned(),
                bytes: svg.as_bytes().to_vec(),
                intrinsic_width: Some(width),
                intrinsic_height: Some(height),
            }))
        })
    })
}

fn light_theme() -> MarkdownEditorTheme {
    MarkdownEditorTheme {
        background: gpui::rgb(0xffffff).into(),
        foreground: gpui::rgb(0x24292f).into(),
        muted_foreground: gpui::rgb(0x667085).into(),
        border: gpui::rgb(0xd0d7de).into(),
        primary: gpui::rgb(0x0969da).into(),
        highlight_theme: HighlightTheme::default_light(),
    }
}

const COMBINED_SAMPLE: &str = r#"# Markdown Typora 体验验收

正文包含 **粗体**、_强调_、`inline code` 与 $e^{i\pi}+1=0$。这一段故意写得更长，用于观察自然换行、激活前后的基线、行高与后续块位置是否保持稳定。

## 标题、引用与列表

> 引用块与后续内容的基线、换行和高度必须稳定，进入编辑态时不能推动页面。

1. 有序列表第一项
2. 有序列表第二项也包含足够长的文字，以验证窄宽度下的自然换行与 marker gutter 是否稳定。

- 普通无序列表
- [ ] 未完成任务
- [x] 已完成任务

```mermaid
graph LR
  A[输入] --> B[渲染]
```

$$
\frac{a+b}{c}
$$

富渲染块后的普通段落必须保持原位，点击上方 Math 或 Mermaid 时不能上下跳动。

<section><h2>Native HTML</h2><p>HTML 使用已有的 <strong>TextView::html</strong> 渲染；点击后直接编辑同一永久 surface。</p></section>

HTML 块后的普通段落也必须保持原位。

![示例图片](missing-combination-regression.png)

```rust
fn main() {
    println!("markdown");
}
```

| 名称 | 状态 | 说明 |
| :--- | :---: | ---: |
| 数学公式 | 完成 | 保持高度 |
| Mermaid | 完成 | 共享缓存 |

表格后的普通段落用于验证点击任意单元格时，整张表格和后续内容都不会跳动。
"#;

const MERMAID_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="620" height="164" viewBox="0 0 620 164">
  <defs>
    <marker id="arrow" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto">
      <path d="M0,0 L0,6 L9,3 z" fill="#667085"/>
    </marker>
  </defs>
  <rect x="44" y="42" width="176" height="80" rx="12" fill="#eef6ff" stroke="#0969da" stroke-width="2"/>
  <rect x="400" y="42" width="176" height="80" rx="12" fill="#eef6ff" stroke="#0969da" stroke-width="2"/>
  <path d="M220 82 H390" stroke="#667085" stroke-width="3" marker-end="url(#arrow)"/>
  <text x="132" y="91" text-anchor="middle" font-family="system-ui, sans-serif" font-size="24" fill="#24292f">输入</text>
  <text x="488" y="91" text-anchor="middle" font-family="system-ui, sans-serif" font-size="24" fill="#24292f">渲染</text>
</svg>"##;

const MATH_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="360" height="120" viewBox="0 0 360 120">
  <text x="180" y="43" text-anchor="middle" font-family="Times New Roman, serif" font-style="italic" font-size="34" fill="#24292f">a + b</text>
  <line x1="118" y1="57" x2="242" y2="57" stroke="#24292f" stroke-width="2"/>
  <text x="180" y="94" text-anchor="middle" font-family="Times New Roman, serif" font-style="italic" font-size="34" fill="#24292f">c</text>
</svg>"##;
