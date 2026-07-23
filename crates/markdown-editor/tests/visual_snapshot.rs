#![cfg(target_os = "macos")]

use gpui::{AppContext, HeadlessAppContext, px, size};
use gpui_component::{Root, highlighter::HighlightTheme};
use markdown_editor::{MarkdownEditor, MarkdownEditorTheme};
use std::sync::Arc;

#[test]
#[ignore = "manual Typora-style visual audit"]
fn capture_markdown_editor_snapshot() {
    capture_sample(SAMPLE, "markdown-editor");
    capture_sample(WRAPPED_LIST, "markdown-editor-wrapped-list");
    capture_table_sample();
}

fn capture_sample(source: &'static str, name: &str) {
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let list_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .blocks
        .iter()
        .find(|block| {
            matches!(
                block.kind,
                markdown_source::SourceBlockKind::OrderedList { .. }
                    | markdown_source::SourceBlockKind::UnorderedList
            )
        })
        .unwrap()
        .id;
    let mut editor = None;
    let window = cx
        .open_window(size(px(960.), px(720.)), |window, cx| {
            let entity =
                cx.new(|cx| MarkdownEditor::new(source, light_theme(), window, cx).unwrap());
            editor = Some(entity.clone());
            cx.new(|cx| Root::new(entity, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    save_snapshot(&mut cx, window.into(), &format!("{name}-preview.png"));
    let editor = editor.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.activate_block(list_id, window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
    save_snapshot(&mut cx, window.into(), &format!("{name}-active-list.png"));
}

fn save_snapshot(cx: &mut HeadlessAppContext, window: gpui::AnyWindowHandle, name: &str) {
    let image = cx.capture_screenshot(window).unwrap();
    let path = std::env::var_os("MARKDOWN_EDITOR_SNAPSHOT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(name);
    image.save(path).unwrap();
}

fn capture_table_sample() {
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let document = markdown_source::SourceMarkdownDocument::parse(TABLE_SAMPLE).unwrap();
    let block_id = document.blocks[2].id;
    let address = markdown_source::TableCellAddress {
        block_id,
        row: 2,
        column: 0,
    };
    let mut editor = None;
    let window = cx
        .open_window(size(px(960.), px(720.)), |window, cx| {
            let entity =
                cx.new(|cx| MarkdownEditor::new(TABLE_SAMPLE, light_theme(), window, cx).unwrap());
            editor = Some(entity.clone());
            cx.new(|cx| Root::new(entity, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    save_snapshot(&mut cx, window.into(), "markdown-editor-table-preview.png");
    let editor = editor.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.activate_table_cell(address, window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
    save_snapshot(&mut cx, window.into(), "markdown-editor-table-active.png");
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

const SAMPLE: &str = r#"# Typora-style Markdown

Paragraph with **bold**, _emphasis_, `inline code`, and $e^{i\pi} + 1 = 0$.

- First list item
- [ ] Task item
- [x] Completed item

> Block quote stays visually structured while editing.

```rust
fn main() {
    println!(\"hello markdown\");
}
```

$$
\frac{a}{b}
$$
"#;

const WRAPPED_LIST: &str = r#"# Wrapped lists

- This is a deliberately long first list item that wraps onto multiple visual lines in a narrow editor while keeping the following marker anchored to its actual input baseline.
- [ ] The task marker belongs to this second item.
- [x] Completed third item.
"#;

const TABLE_SAMPLE: &str = r#"# 接口整理

## 2.1 现有接口清单

| 接口 | 当前用途 | 处理结论 |
| --- | --- | --- |
| POST /ai-manager/dashboard/publish | 从服务器本地绝对路径发布看板 | 保留当前用户的默认看板，不允许普通浏览器直接调用 |
| POST /ai-manager/dashboard/save/resolve | 解决同名冲突：覆盖或另存为新看板 | 复用并增强权限、默认项、版本和修改时间字段 |
| GET /ai-manager/dashboard/view/{id} | 查看编译后的 HTML | 直接复用，增加权限、CSP 和版本缓存控制 |
| POST /ai-manager/dashboard/metricDatas | HTML Runtime 批量查询指标 | 直接复用，只由看板 HTML Runtime 调用 |

## 2.2 现有 Swagger 与接口文档不一致项

下面的内容不应该与上面的表格重叠。
"#;
