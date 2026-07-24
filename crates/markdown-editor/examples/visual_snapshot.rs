#![cfg(target_os = "macos")]

use gpui::{AppContext, Bounds, QuitMode, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, highlighter::HighlightTheme};
use gpui_component_assets::Assets;
use markdown_editor::{MarkdownEditor, MarkdownEditorTheme};

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
                .find(|block| {
                    matches!(
                        block.kind,
                        markdown_source::SourceBlockKind::UnorderedList
                    )
                })
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
                    MarkdownEditor::new(COMBINED_SAMPLE, light_theme(), window, cx).unwrap()
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
                    MarkdownEditor::new(COMBINED_SAMPLE, light_theme(), window, cx).unwrap()
                });
                editor.update(cx, |editor, cx| {
                    assert!(editor.activate_block(task_list_id, window, cx));
                });
                cx.new(|cx| Root::new(editor, window, cx))
            })
            .expect("open Markdown active task list audit window");
        });
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
"#;
