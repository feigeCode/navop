//! Redis 键值视图

use crate::{
    GlobalRedisState, HashField, KeyInfo, KeyValueContent, KeyValueDetail, RedisKeyType, ZSetMember,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gpui::{
    App, AppContext, AsyncApp, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement, Styled, Task, Window, div, prelude::FluentBuilder,
    px, relative,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, IconSize as GpuiIconSize, IndexPath, Sizable, Size,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    notification::Notification,
    radio::Radio,
    select::{Select, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::tab_container::{TabContent, TabContentEvent};
use one_ui::{
    ContentState, IconButton, IconSize, LargeTextEditor, StatusBar,
    create_large_text_editor_with_content,
};
use rust_i18n::t;

/// 键值视图事件
#[derive(Clone, Debug)]
pub enum KeyValueViewEvent {
    /// 值已更新
    ValueUpdated { key: String },
    /// 值已删除
    ValueDeleted {
        connection_id: String,
        db_index: u8,
        key: String,
    },
}

/// 加载状态
#[derive(Clone, Debug, PartialEq)]
enum LoadState {
    Empty,
    Loading,
    Loaded,
    Error(String),
}

/// 查看格式
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewFormat {
    #[default]
    Raw,
    Json,
    Hex,
    Binary,
}

impl ViewFormat {
    pub fn all() -> Vec<Self> {
        vec![
            ViewFormat::Raw,
            ViewFormat::Json,
            ViewFormat::Hex,
            ViewFormat::Binary,
        ]
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ViewFormat::Raw => "Raw",
            ViewFormat::Json => "JSON",
            ViewFormat::Hex => "Hex",
            ViewFormat::Binary => "Binary",
        }
    }
}

impl SelectItem for ViewFormat {
    type Value = ViewFormat;

    fn title(&self) -> SharedString {
        self.display_name().into()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

fn format_redis_string_value(value: &[u8], format: ViewFormat) -> String {
    match format {
        ViewFormat::Raw => match readable_redis_text(value) {
            Some(value) => value.to_string(),
            None => escape_binary_redis_string(value),
        },
        ViewFormat::Json => match readable_redis_text(value) {
            Some(value) => match serde_json::from_str::<serde_json::Value>(value) {
                Ok(value) => {
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
                }
                Err(_) => value.to_string(),
            },
            None => escape_binary_redis_string(value),
        },
        ViewFormat::Hex => value
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" "),
        ViewFormat::Binary => value
            .iter()
            .map(|byte| format!("{byte:08b}"))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn escape_binary_redis_string(value: &[u8]) -> String {
    value
        .iter()
        .map(|byte| match byte {
            b'\\' => r"\\".to_string(),
            b' '..=b'~' => char::from(*byte).to_string(),
            _ => format!("\\x{byte:02x}"),
        })
        .collect()
}

fn readable_redis_text(value: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(value).ok()?;
    text.chars()
        .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .then_some(text)
}

fn is_binary_redis_string(value: &[u8]) -> bool {
    readable_redis_text(value).is_none()
}

fn redis_bytes_text(value: &[u8]) -> Option<String> {
    readable_redis_text(value).map(str::to_string)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RedisBytesDisplay {
    Text(String),
    Binary { byte_len: usize },
}

fn redis_bytes_display(value: &[u8]) -> RedisBytesDisplay {
    match redis_bytes_text(value) {
        Some(text) => RedisBytesDisplay::Text(text),
        None => RedisBytesDisplay::Binary {
            byte_len: value.len(),
        },
    }
}

fn redis_bytes_copy_text(value: &[u8]) -> String {
    redis_bytes_text(value).unwrap_or_else(|| BASE64.encode(value))
}

fn redis_bytes_pair_copy_text(left: &[u8], right: &[u8]) -> String {
    format!(
        "{}: {}",
        redis_bytes_copy_text(left),
        redis_bytes_copy_text(right)
    )
}

fn zset_member_copy_text(score: f64, member: &[u8]) -> String {
    match redis_bytes_text(member) {
        Some(member) => format!("{score}: {member}"),
        None => redis_bytes_copy_text(member),
    }
}

fn binary_download_file_name(kind: &str, zero_based_index: usize) -> String {
    format!("redis-{kind}-{}.bin", zero_based_index + 1)
}

fn display_redis_bytes(value: &[u8]) -> String {
    format_redis_string_value(value, ViewFormat::Raw)
}

fn large_text_preview_title(label: &str, context: Option<&str>) -> String {
    match context.map(str::trim).filter(|context| !context.is_empty()) {
        Some(context) => format!("{label} {context}"),
        None => label.to_string(),
    }
}

fn should_replace_set_member(old_member: &str, new_member: &str) -> bool {
    old_member != new_member
}

/// List 插入位置
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListInsertPosition {
    /// 头部插入 (LPUSH)
    #[default]
    Head,
    /// 尾部插入 (RPUSH)
    Tail,
}

/// 排序方向
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// ZSet 排序字段
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZSetSortBy {
    #[default]
    Score,
    Member,
}

/// 键值视图
pub struct KeyValueView {
    /// 当前连接 ID
    connection_id: Option<String>,
    /// 当前数据库索引
    db_index: u8,
    /// 当前键名
    current_key: Option<String>,
    /// 键信息
    key_info: Option<KeyInfo>,
    /// 键值内容
    value_content: Option<KeyValueContent>,
    /// 加载状态
    load_state: LoadState,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 是否修改过
    is_dirty: bool,
    /// 当前查看格式
    view_format: ViewFormat,
    /// 查看格式选择器状态
    format_select: Entity<SelectState<Vec<ViewFormat>>>,
    /// String 值编辑器状态
    string_editor: Entity<TextareaState>,
    /// 待设置的编辑器值（异步加载完成后设置）
    pending_editor_value: Option<String>,
    /// 单调递增的加载代次，用于忽略晚到的旧请求。
    load_generation: u64,

    // === 筛选功能 ===
    /// 筛选输入框状态
    filter_input: Entity<InputState>,
    /// 当前筛选文本
    filter_text: String,
    /// 是否全文匹配
    filter_exact_match: bool,

    // === 排序功能 ===
    /// 排序方向
    sort_order: SortOrder,
    /// ZSet 排序字段
    zset_sort_by: ZSetSortBy,

    // === List 插入位置 ===
    list_insert_position: ListInsertPosition,
    /// 是否允许关闭标签页
    closeable: bool,
}

impl KeyValueView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_closeable(false, window, cx)
    }

    pub fn new_with_closeable(
        closeable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let format_select = cx.new(|cx| {
            SelectState::new(
                ViewFormat::all(),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });

        let string_editor = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("KeyValueView.select_key_placeholder").to_string())
        });

        let filter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("KeyValueView.filter_placeholder").to_string())
        });

        cx.subscribe_in(&format_select, window, Self::on_format_changed)
            .detach();

        cx.subscribe_in(&string_editor, window, Self::on_editor_changed)
            .detach();

        cx.subscribe_in(&filter_input, window, Self::on_filter_changed)
            .detach();

        Self {
            connection_id: None,
            db_index: 0,
            current_key: None,
            key_info: None,
            value_content: None,
            load_state: LoadState::Empty,
            focus_handle: cx.focus_handle(),
            is_dirty: false,
            view_format: ViewFormat::Raw,
            format_select,
            string_editor,
            pending_editor_value: None,
            load_generation: 0,
            filter_input,
            filter_text: String::new(),
            filter_exact_match: false,
            sort_order: SortOrder::Asc,
            zset_sort_by: ZSetSortBy::Score,
            list_insert_position: ListInsertPosition::Tail,
            closeable,
        }
    }

    /// 格式选择器变化处理
    fn on_format_changed(
        &mut self,
        _select: &Entity<SelectState<Vec<ViewFormat>>>,
        event: &SelectEvent<Vec<ViewFormat>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(format)) = event {
            self.view_format = *format.value();
            self.update_editor_content(window, cx);
            cx.notify();
        }
    }

    /// 编辑器内容变化处理
    fn on_editor_changed(
        &mut self,
        _editor: &Entity<TextareaState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::Change = event
            && self.string_value_is_editable()
        {
            self.is_dirty = true;
            cx.notify();
        }
    }

    /// 筛选输入变化处理
    fn on_filter_changed(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::Change = event {
            self.filter_text = self.filter_input.read(cx).text().to_string();
            cx.notify();
        }
    }

    /// 应用筛选到字符串列表
    fn apply_filter(&self, items: &[Vec<u8>]) -> Vec<(usize, Vec<u8>)> {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                if self.filter_text.is_empty() {
                    return true;
                }
                let item = display_redis_bytes(item);
                if self.filter_exact_match {
                    item.contains(&self.filter_text)
                } else {
                    item.to_lowercase()
                        .contains(&self.filter_text.to_lowercase())
                }
            })
            .map(|(idx, s)| (idx, s.clone()))
            .collect()
    }

    /// 应用筛选到 Hash 字段
    fn apply_filter_hash(&self, items: &[HashField]) -> Vec<(usize, HashField)> {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                if self.filter_text.is_empty() {
                    return true;
                }
                let search_text = format!(
                    "{} {}",
                    display_redis_bytes(&item.field),
                    display_redis_bytes(&item.value)
                );
                if self.filter_exact_match {
                    search_text.contains(&self.filter_text)
                } else {
                    search_text
                        .to_lowercase()
                        .contains(&self.filter_text.to_lowercase())
                }
            })
            .map(|(idx, f)| (idx, f.clone()))
            .collect()
    }

    /// 应用筛选到 ZSet 成员
    fn apply_filter_zset(&self, items: &[ZSetMember]) -> Vec<(usize, ZSetMember)> {
        let mut filtered: Vec<(usize, ZSetMember)> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                if self.filter_text.is_empty() {
                    return true;
                }
                let member = display_redis_bytes(&item.member);
                if self.filter_exact_match {
                    member.contains(&self.filter_text)
                } else {
                    member
                        .to_lowercase()
                        .contains(&self.filter_text.to_lowercase())
                }
            })
            .map(|(idx, m)| (idx, m.clone()))
            .collect();

        // 应用排序
        match self.zset_sort_by {
            ZSetSortBy::Score => {
                filtered.sort_by(|a, b| {
                    if self.sort_order == SortOrder::Asc {
                        a.1.score
                            .partial_cmp(&b.1.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    } else {
                        b.1.score
                            .partial_cmp(&a.1.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    }
                });
            }
            ZSetSortBy::Member => {
                filtered.sort_by(|a, b| {
                    if self.sort_order == SortOrder::Asc {
                        a.1.member.cmp(&b.1.member)
                    } else {
                        b.1.member.cmp(&a.1.member)
                    }
                });
            }
        }
        filtered
    }

    /// 切换排序方向
    fn toggle_sort_order(&mut self, cx: &mut Context<Self>) {
        self.sort_order = match self.sort_order {
            SortOrder::Asc => SortOrder::Desc,
            SortOrder::Desc => SortOrder::Asc,
        };
        cx.notify();
    }

    /// 切换 ZSet 排序字段
    fn toggle_zset_sort_by(&mut self, cx: &mut Context<Self>) {
        self.zset_sort_by = match self.zset_sort_by {
            ZSetSortBy::Score => ZSetSortBy::Member,
            ZSetSortBy::Member => ZSetSortBy::Score,
        };
        cx.notify();
    }

    fn large_text_editor_value(
        editor: &Entity<LargeTextEditor>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        match editor.read(cx).get_writeback_text(cx) {
            Ok(value) => Some(value),
            Err(err) => {
                window.push_notification(err.to_string(), cx);
                None
            }
        }
    }

    fn show_large_text_preview_dialog(
        &self,
        title: String,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = create_large_text_editor_with_content(Some(value), window, cx);

        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(SharedString::from(title.clone()))
                .w(px(900.))
                .h(px(680.))
                .child(v_flex().size_full().child(editor.clone()))
                .close_button(true)
                .overlay(false)
                .content_center()
        });
    }

    fn download_binary_value(&self, bytes: Vec<u8>, file_name: String, cx: &mut Context<Self>) {
        let window_id = cx.active_window();
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            multiple: false,
            directories: true,
            prompt: Some(t!("KeyValueView.select_binary_download_directory").into()),
        });

        cx.spawn(async move |_this, cx| {
            let output_dir = match future.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                _ => return,
            };
            let output_path = output_dir.join(file_name);
            let path_for_write = output_path.clone();
            let write_result = cx
                .background_spawn(async move { std::fs::write(path_for_write, bytes) })
                .await;

            let _ = cx.update(|cx| {
                if let Some(window_id) = window_id {
                    let _ = cx.update_window(window_id, |_entity, window, cx| match write_result {
                        Ok(()) => window.push_notification(
                            t!(
                                "KeyValueView.binary_download_complete",
                                path = output_path.display()
                            )
                            .to_string(),
                            cx,
                        ),
                        Err(error) => window.push_notification(
                            Notification::error(
                                t!("KeyValueView.binary_download_failed", error = error)
                                    .to_string(),
                            ),
                            cx,
                        ),
                    });
                }
            });
        })
        .detach();
    }

    /// 更新编辑器内容（根据格式转换）
    fn update_editor_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyValueContent::String(ref value)) = self.value_content else {
            return;
        };

        let formatted = self.format_value(value);

        self.string_editor.update(cx, |state, cx| {
            state.set_value(formatted, window, cx);
        });
    }

    /// 格式化值
    fn format_value(&self, value: &[u8]) -> String {
        format_redis_string_value(value, self.view_format)
    }

    fn string_value_is_editable(&self) -> bool {
        matches!(
            &self.value_content,
            Some(KeyValueContent::String(value)) if !is_binary_redis_string(value)
        )
    }

    /// 加载键
    pub fn load_key(
        &mut self,
        connection_id: String,
        db_index: u8,
        key: String,
        cx: &mut Context<Self>,
    ) {
        self.connection_id = Some(connection_id.clone());
        self.db_index = db_index;
        self.current_key = Some(key.clone());
        self.load_state = LoadState::Loading;
        self.is_dirty = false;
        self.pending_editor_value = None;
        self.load_generation = self.load_generation.wrapping_add(1);
        let load_generation = self.load_generation;
        cx.notify();

        let global_state = cx.global::<GlobalRedisState>().clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result =
                Tokio::spawn_result(cx, {
                    let connection_id = connection_id.clone();
                    let key = key.clone();
                    async move {
                        Self::fetch_key_value(&global_state, &connection_id, db_index, &key).await
                    }
                })
                .await;

            _ = this.update(cx, |view, cx| {
                if !view.is_current_load(load_generation, &connection_id, db_index, &key) {
                    return;
                }
                match result {
                    Ok(detail) => {
                        if let KeyValueContent::String(ref value) = detail.value {
                            view.pending_editor_value = Some(view.format_value(value));
                        }
                        view.key_info = Some(detail.key_info);
                        view.value_content = Some(detail.value);
                        view.load_state = LoadState::Loaded;
                    }
                    Err(e) => {
                        view.load_state = LoadState::Error(format!("{e:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn is_current_load(
        &self,
        generation: u64,
        connection_id: &str,
        db_index: u8,
        key: &str,
    ) -> bool {
        self.load_generation == generation
            && self.connection_id.as_deref() == Some(connection_id)
            && self.db_index == db_index
            && self.current_key.as_deref() == Some(key)
    }

    /// 在 render 中应用待设置的编辑器值
    fn apply_pending_editor_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(value) = self.pending_editor_value.take() {
            self.string_editor.update(cx, |state, cx| {
                state.set_value(value, window, cx);
            });
        }
    }

    /// 获取键值
    async fn fetch_key_value(
        global_state: &GlobalRedisState,
        connection_id: &str,
        db_index: u8,
        key: &str,
    ) -> anyhow::Result<KeyValueDetail> {
        let conn = global_state
            .get_connection(connection_id)
            .ok_or_else(|| anyhow::anyhow!("{}", t!("KeyValueView.connection_missing")))?;

        let guard = conn.read().await;
        guard
            .get_key_value_detail_in_db(db_index, key)
            .await
            .map_err(anyhow::Error::new)
    }

    /// 获取编辑器内容
    fn get_editor_content(&self, cx: &App) -> String {
        self.string_editor.read(cx).text().to_string()
    }

    /// 渲染键信息面板（工具栏）
    fn render_key_info(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(info) = &self.key_info else {
            return div().into_any_element();
        };

        let key_name = info.name.clone();
        let key_type = info.key_type;
        let key_type_display = info.key_type.display_name();
        let ttl_display = info.ttl_display();
        let view = cx.entity().clone();
        let key_for_copy = key_name.clone();
        let editor_content = self.get_editor_content(cx);
        let can_add_element = matches!(
            key_type,
            RedisKeyType::List | RedisKeyType::Set | RedisKeyType::ZSet | RedisKeyType::Hash
        );
        let is_string = matches!(key_type, RedisKeyType::String);
        let is_string_editable = self.string_value_is_editable();
        let is_zset = matches!(key_type, RedisKeyType::ZSet);
        let zset_sort_label = match self.zset_sort_by {
            ZSetSortBy::Score => t!("KeyValueView.sort_by_score"),
            ZSetSortBy::Member => t!("KeyValueView.sort_by_member"),
        };
        let zset_sort_text = t!("KeyValueView.sort_by", by = zset_sort_label);

        v_flex()
            .w_full()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            // 第一行：类型 + 键名 + 操作按钮
            .child(
                h_flex()
                    .w_full()
                    .p_2()
                    .gap_2()
                    .items_center()
                    // 类型徽章
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(px(4.0))
                            .bg(cx.theme().primary)
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(cx.theme().primary_foreground)
                            .child(key_type_display),
                    )
                    // 键名
                    .child(div().flex_1().text_sm().truncate().child(key_name.clone()))
                    // 刷新按钮
                    .child(
                        IconButton::new("refresh-key", IconName::Refresh)
                            .glyph_size(IconSize::Default)
                            .tooltip(t!("Common.refresh"))
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |view, cx| {
                                        if let (Some(conn_id), Some(key)) =
                                            (view.connection_id.clone(), view.current_key.clone())
                                        {
                                            view.load_key(conn_id, view.db_index, key, cx);
                                        }
                                    });
                                }
                            }),
                    )
                    // 复制键名按钮
                    .child(
                        IconButton::new("copy-key", IconName::Copy)
                            .glyph_size(IconSize::Default)
                            .tooltip(t!("Common.copy"))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    key_for_copy.clone(),
                                ));
                            }),
                    )
                    // TTL 显示（可点击编辑）
                    .child(
                        Button::new("ttl-display")
                            .ghost()
                            .with_size(Size::Medium)
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .px_2()
                                    .py_0p5()
                                    .rounded(px(4.0))
                                    .bg(cx.theme().secondary)
                                    .child(
                                        Icon::new(IconName::Calendar)
                                            .with_size(Size::Small)
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().secondary_foreground)
                                            .child(ttl_display),
                                    ),
                            )
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.show_ttl_dialog(window, cx);
                                    });
                                }
                            }),
                    )
                    // 重命名按钮
                    .child(
                        IconButton::new("rename-key", IconName::Edit)
                            .glyph_size(IconSize::Default)
                            .tooltip(t!("KeyValueView.rename_key_title"))
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.show_rename_dialog(window, cx);
                                    });
                                }
                            }),
                    )
                    // 删除按钮
                    .child(
                        IconButton::new("delete-key", IconName::Remove)
                            .glyph_size(IconSize::Default)
                            .tooltip(t!("Common.delete"))
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.show_delete_dialog(window, cx);
                                    });
                                }
                            }),
                    ),
            )
            // 第二行：筛选 + 操作按钮
            .child(
                h_flex()
                    .w_full()
                    .px_2()
                    .pb_2()
                    .gap_2()
                    .items_center()
                    // 左侧：筛选输入框（集合类型）或格式选择器（String 类型）
                    .child(
                        h_flex()
                            .flex_1()
                            .gap_2()
                            .items_center()
                            .when(is_string, |this| {
                                this.child(
                                    div()
                                        .text_base()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(t!("KeyValueView.view_mode").to_string()),
                                )
                                .child(
                                    Select::new(&self.format_select)
                                        .with_size(Size::Medium)
                                        .w(px(100.)),
                                )
                            })
                            .when(can_add_element, |this| {
                                this.child(
                                    Input::new(&self.filter_input)
                                        .with_size(Size::Medium)
                                        .w(px(200.)),
                                )
                                .child(
                                    Checkbox::new("exact-match")
                                        .label(t!("KeyValueView.exact_match").to_string())
                                        .with_size(Size::Medium)
                                        .checked(self.filter_exact_match)
                                        .on_click({
                                            let view = view.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |view, cx| {
                                                    view.filter_exact_match =
                                                        !view.filter_exact_match;
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                )
                            }),
                    )
                    // 右侧按钮组
                    .child(
                        h_flex()
                            .gap_2()
                            // 排序按钮（集合类型）
                            .when(can_add_element, |this| {
                                this.child(
                                    IconButton::new(
                                        "sort-order",
                                        if self.sort_order == SortOrder::Asc {
                                            IconName::SortAscending
                                        } else {
                                            IconName::SortDescending
                                        },
                                    )
                                    .glyph_size(IconSize::Default)
                                    .tooltip(t!("KeyValueView.toggle_sort_order"))
                                    .on_click({
                                        let view = view.clone();
                                        move |_, _, cx| {
                                            view.update(cx, |view, cx| {
                                                view.toggle_sort_order(cx);
                                            });
                                        }
                                    }),
                                )
                            })
                            .when(is_zset, |this| {
                                this.child(
                                    Button::new("zset-sort-by")
                                        .icon(IconName::ChevronsUpDown)
                                        .label(zset_sort_text.to_string())
                                        .ghost()
                                        .with_size(Size::Medium)
                                        .on_click({
                                            let view = view.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |view, cx| {
                                                    view.toggle_zset_sort_by(cx);
                                                });
                                            }
                                        }),
                                )
                            })
                            // 插入行按钮（仅对集合类型显示）
                            .when(can_add_element, |this| {
                                this.child(
                                    Button::new("add-element")
                                        .icon(IconName::Plus)
                                        .label(t!("KeyValueView.insert_row").to_string())
                                        .primary()
                                        .with_size(Size::Medium)
                                        .on_click({
                                            let view = view.clone();
                                            move |_, window, cx| {
                                                view.update(cx, |view, cx| {
                                                    view.show_add_dialog(window, cx);
                                                });
                                            }
                                        }),
                                )
                            })
                            // 复制值按钮
                            .child(
                                Button::new("copy-value")
                                    .icon(IconName::Copy)
                                    .label(t!("KeyValueView.copy_value").to_string())
                                    .ghost()
                                    .with_size(Size::Medium)
                                    .on_click({
                                        let content = editor_content.clone();
                                        move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                content.clone(),
                                            ));
                                        }
                                    }),
                            )
                            // 保存按钮（仅 String 类型显示）
                            .when(is_string_editable && self.is_dirty, |this| {
                                this.child(
                                    Button::new("save-value")
                                        .icon(IconName::Check)
                                        .label(t!("KeyValueView.save").to_string())
                                        .success()
                                        .with_size(Size::Medium)
                                        .on_click({
                                            let view = view.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |view, cx| {
                                                    view.save_string_value(cx);
                                                });
                                            }
                                        }),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    /// 显示新增对话框（根据当前键类型分发）
    fn show_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = &self.key_info else { return };
        match info.key_type {
            RedisKeyType::List => self.show_list_add_dialog(window, cx),
            RedisKeyType::Set => self.show_set_add_dialog(window, cx),
            RedisKeyType::ZSet => self.show_zset_add_dialog(window, cx),
            RedisKeyType::Hash => self.show_hash_add_dialog(window, cx),
            _ => {}
        }
    }

    // === List 对话框 ===

    /// 显示 List 添加对话框
    fn show_list_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value_editor = create_large_text_editor_with_content(None, window, cx);
        let view = cx.entity().downgrade();
        let position = self.list_insert_position;

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let editor_for_ok = value_editor.clone();
            let view_for_ok = view.clone();

            dialog
                .title(t!("KeyValueView.list_insert_title").to_string())
                .w(px(800.))
                .h(px(620.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_4()
                                .child(
                                    Radio::new("insert-head")
                                        .label(t!("KeyValueView.list_insert_head").to_string())
                                        .checked(position == ListInsertPosition::Head),
                                )
                                .child(
                                    Radio::new("insert-tail")
                                        .label(t!("KeyValueView.list_insert_tail").to_string())
                                        .checked(position == ListInsertPosition::Tail),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.value_label").to_string()),
                                )
                                .child(div().h(px(460.)).w_full().child(value_editor.clone())),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.add").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let Some(value) = Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    if value.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.add_list_element(value, position, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 显示 List 编辑对话框
    fn show_list_edit_dialog(
        &mut self,
        index: usize,
        current_value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value_editor = create_large_text_editor_with_content(Some(current_value), window, cx);
        let view = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let editor_for_ok = value_editor.clone();
            let view_for_ok = view.clone();

            dialog
                .title(t!("KeyValueView.edit_list_item", index = index + 1).to_string())
                .w(px(800.))
                .h(px(600.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("KeyValueView.value_label").to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(value_editor.clone()),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.save").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let Some(value) = Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    if value.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.edit_list_element(index, value, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 添加 List 元素
    fn add_list_element(
        &mut self,
        value: String,
        position: ListInsertPosition,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let value = value.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    match position {
                        ListInsertPosition::Head => {
                            guard
                                .lpush_in_db(db_index, &key, &[value.as_str()])
                                .await
                                .map_err(anyhow::Error::new)?;
                        }
                        ListInsertPosition::Tail => {
                            guard
                                .rpush_in_db(db_index, &key, &[value.as_str()])
                                .await
                                .map_err(anyhow::Error::new)?;
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 编辑 List 元素（通过 LSET）
    fn edit_list_element(&mut self, index: usize, new_value: String, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let new_value = new_value.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .lset_in_db(db_index, &key, index as i64, &new_value)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // === Set 对话框 ===

    /// 显示 Set 添加对话框
    fn show_set_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let member_editor = create_large_text_editor_with_content(None, window, cx);
        let view = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let editor_for_ok = member_editor.clone();
            let view_for_ok = view.clone();

            dialog
                .title(t!("KeyValueView.add_set_member").to_string())
                .w(px(800.))
                .h(px(600.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("KeyValueView.member_label").to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(member_editor.clone()),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.add").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let Some(member) = Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    if member.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.add_set_member(member, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 显示 Set 编辑对话框
    fn show_set_edit_dialog(
        &mut self,
        current_member: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let member_editor =
            create_large_text_editor_with_content(Some(current_member.clone()), window, cx);
        let view = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let editor_for_ok = member_editor.clone();
            let view_for_ok = view.clone();
            let old_member = current_member.clone();

            dialog
                .title(t!("KeyValueView.edit_set_member").to_string())
                .w(px(800.))
                .h(px(600.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("KeyValueView.member_label").to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(member_editor.clone()),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.save").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let Some(new_member) =
                        Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    if new_member.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.update_set_member(old_member.clone(), new_member, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 添加 Set 成员
    fn add_set_member(&mut self, member: String, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let member = member.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .sadd_in_db(db_index, &key, &[member.as_str()])
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // === ZSet 对话框 ===

    /// 显示 ZSet 添加对话框
    fn show_zset_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let score_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("KeyValueView.score_placeholder").to_string());
            state.set_value("0", window, cx);
            state
        });
        let member_editor = create_large_text_editor_with_content(None, window, cx);
        let view = cx.entity().downgrade();

        // 在打开对话框前设置焦点，避免闪烁
        score_input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let score_for_ok = score_input.clone();
            let editor_for_ok = member_editor.clone();
            let view_for_ok = view.clone();

            dialog
                .title(t!("KeyValueView.add_zset_member").to_string())
                .w(px(800.))
                .h(px(660.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.score_label").to_string()),
                                )
                                .child(Input::new(&score_input).w_full()),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.member_label").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .w_full()
                                        .child(member_editor.clone()),
                                ),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.add").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let Some(member) = Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    let score_str = score_for_ok.read(cx).text().to_string();
                    let score: f64 = score_str.parse().unwrap_or(0.0);
                    if member.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.add_zset_member(member, score, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 显示 ZSet 编辑对话框
    fn show_zset_edit_dialog(
        &mut self,
        member: String,
        current_score: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let score_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("KeyValueView.score_placeholder").to_string());
            state.set_value(format!("{}", current_score), window, cx);
            state
        });
        let member_editor = create_large_text_editor_with_content(Some(member.clone()), window, cx);
        let view = cx.entity().downgrade();
        let old_member = member;

        // 在打开对话框前设置焦点，避免闪烁
        score_input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let score_for_ok = score_input.clone();
            let editor_for_ok = member_editor.clone();
            let view_for_ok = view.clone();
            let old_member_for_ok = old_member.clone();

            dialog
                .title(t!("KeyValueView.edit_zset_member").to_string())
                .w(px(800.))
                .h(px(660.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.score_label").to_string()),
                                )
                                .child(Input::new(&score_input).w_full()),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.member_label").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .w_full()
                                        .child(member_editor.clone()),
                                ),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.save").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let Some(new_member) =
                        Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    let score_str = score_for_ok.read(cx).text().to_string();
                    let score: f64 = score_str.parse().unwrap_or(0.0);
                    if new_member.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.update_zset_member(old_member_for_ok.clone(), new_member, score, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 添加 ZSet 成员
    fn add_zset_member(&mut self, member: String, score: f64, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let member = member.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .zadd_in_db(db_index, &key, &[(score, member.as_str())])
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 更新 ZSet 成员（删除旧成员后添加新成员）
    fn update_zset_member(
        &mut self,
        old_member: String,
        new_member: String,
        score: f64,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let old_member = old_member.clone();
                let new_member = new_member.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    // 如果成员名变了，先删除旧的
                    if old_member != new_member {
                        guard
                            .zrem_in_db(db_index, &key, &[old_member.as_str()])
                            .await
                            .map_err(anyhow::Error::new)?;
                    }
                    guard
                        .zadd_in_db(db_index, &key, &[(score, new_member.as_str())])
                        .await
                        .map_err(anyhow::Error::new)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // === Hash 对话框 ===

    /// 显示 Hash 添加对话框
    fn show_hash_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("KeyValueView.hash_field_placeholder").to_string())
        });
        let value_editor = create_large_text_editor_with_content(None, window, cx);
        let view = cx.entity().downgrade();

        // 在打开对话框前设置焦点，避免闪烁
        field_input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let field_for_ok = field_input.clone();
            let editor_for_ok = value_editor.clone();
            let view_for_ok = view.clone();

            dialog
                .title(t!("KeyValueView.add_hash_field").to_string())
                .w(px(800.))
                .h(px(660.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.field_label").to_string()),
                                )
                                .child(Input::new(&field_input).w_full()),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.value_label").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .w_full()
                                        .child(value_editor.clone()),
                                ),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.add").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let field = field_for_ok.read(cx).text().to_string();
                    let Some(value) = Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    if field.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.set_hash_field(field, value, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 显示 Hash 编辑对话框
    fn show_hash_edit_dialog(
        &mut self,
        field: String,
        current_value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("KeyValueView.hash_field_placeholder_edit").to_string());
            state.set_value(field.clone(), window, cx);
            state
        });
        let value_editor = create_large_text_editor_with_content(Some(current_value), window, cx);
        let view = cx.entity().downgrade();
        let old_field = field;

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let field_for_ok = field_input.clone();
            let editor_for_ok = value_editor.clone();
            let view_for_ok = view.clone();
            let old_field_for_ok = old_field.clone();

            dialog
                .title(t!("KeyValueView.edit_hash_field").to_string())
                .w(px(800.))
                .h(px(660.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.field_label").to_string()),
                                )
                                .child(Input::new(&field_input).w_full()),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.value_label").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .w_full()
                                        .child(value_editor.clone()),
                                ),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("KeyValueView.save").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let new_field = field_for_ok.read(cx).text().to_string();
                    let Some(value) = Self::large_text_editor_value(&editor_for_ok, window, cx)
                    else {
                        return false;
                    };
                    if new_field.is_empty() {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        // 如果字段名变了，先删除旧字段
                        if old_field_for_ok != new_field {
                            v.delete_hash_field_then_set(
                                old_field_for_ok.clone(),
                                new_field.clone(),
                                value.clone(),
                                cx,
                            );
                        } else {
                            v.set_hash_field(new_field, value, cx);
                        }
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 设置 Hash 字段
    fn set_hash_field(&mut self, field: String, value: String, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let field = field.clone();
                let value = value.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .hset_in_db(db_index, &key, &field, &value)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 删除旧 Hash 字段并设置新字段
    fn delete_hash_field_then_set(
        &mut self,
        old_field: String,
        new_field: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .hdel_in_db(db_index, &key, &[old_field.as_str()])
                        .await
                        .map_err(anyhow::Error::new)?;
                    guard
                        .hset_in_db(db_index, &key, &new_field, &value)
                        .await
                        .map_err(anyhow::Error::new)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // === TTL 对话框 ===

    /// 显示删除键确认对话框
    fn show_delete_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let db_index = self.db_index;
        let view = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let connection_id_for_delete = connection_id.clone();
            let key_for_delete = key.clone();
            let view_for_ok = view.clone();

            dialog
                .overlay(false)
                .title(t!("RedisTree.confirm_delete_title").to_string())
                .confirm()
                .button_props(DialogButtonProps::default().show_cancel(true))
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("RedisTree.confirm_delete_key", key = key).to_string())
                        .child(t!("RedisTree.irreversible").to_string()),
                )
                .on_ok(move |_, _window, cx: &mut App| {
                    let _ = view_for_ok.update(cx, |view, cx| {
                        view.delete_key(
                            connection_id_for_delete.clone(),
                            db_index,
                            key_for_delete.clone(),
                            cx,
                        );
                    });
                    true
                })
        });
    }

    /// 删除当前键
    fn delete_key(
        &mut self,
        connection_id: String,
        db_index: u8,
        key: String,
        cx: &mut Context<Self>,
    ) {
        let global_state = cx.global::<GlobalRedisState>().clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .del_in_db(db_index, &[key.as_str()])
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            match result {
                Ok(_) => {
                    _ = this.update(cx, |view, cx| {
                        if view.connection_id.as_deref() == Some(connection_id.as_str())
                            && view.db_index == db_index
                            && view.current_key.as_deref() == Some(key.as_str())
                        {
                            view.current_key = None;
                            view.key_info = None;
                            view.value_content = None;
                            view.pending_editor_value = None;
                            view.load_state = LoadState::Empty;
                            view.is_dirty = false;
                        }
                        cx.emit(KeyValueViewEvent::ValueDeleted {
                            connection_id,
                            db_index,
                            key,
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let message =
                        t!("RedisTree.delete_key_failed", error = format!("{error:#}")).to_string();
                    let _ = cx.update(|cx| {
                        if let Some(window) = cx.active_window() {
                            _ = window.update(cx, |_, window, cx| {
                                window.push_notification(
                                    Notification::error(message).autohide(true),
                                    cx,
                                );
                            });
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// 显示 TTL 设置对话框
    fn show_ttl_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current_ttl = self.key_info.as_ref().map(|i| i.ttl).unwrap_or(-1);
        let is_permanent = current_ttl == -1;

        let ttl_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("KeyValueView.ttl_placeholder").to_string());
            if current_ttl > 0 {
                state.set_value(current_ttl.to_string(), window, cx);
            }
            state
        });
        let view = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _window, cx| {
            let ttl_for_ok = ttl_input.clone();
            let view_for_ok = view.clone();

            dialog
                .title(t!("KeyValueView.ttl_title").to_string())
                .w(px(400.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(if is_permanent {
                                    t!("KeyValueView.ttl_current_permanent").to_string()
                                } else {
                                    t!("KeyValueView.ttl_current", ttl = current_ttl).to_string()
                                }),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(t!("KeyValueView.ttl_label").to_string()),
                                )
                                .child(Input::new(&ttl_input).w_full()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("KeyValueView.ttl_hint").to_string()),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.ok").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let ttl_str = ttl_for_ok.read(cx).text().to_string();
                    let ttl: Option<i64> = if ttl_str.is_empty() || ttl_str == "-1" {
                        None // 永久
                    } else {
                        ttl_str.parse().ok()
                    };
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.set_key_ttl(ttl, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 设置键的 TTL
    fn set_key_ttl(&mut self, ttl: Option<i64>, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    match ttl {
                        Some(seconds) if seconds > 0 => {
                            guard
                                .expire_in_db(db_index, &key, seconds)
                                .await
                                .map_err(anyhow::Error::new)?;
                        }
                        _ => {
                            guard
                                .persist_in_db(db_index, &key)
                                .await
                                .map_err(anyhow::Error::new)?;
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // === 重命名键 ===

    /// 显示重命名对话框
    fn show_rename_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current_name) = self.current_key.clone() else {
            return;
        };
        let name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("KeyValueView.new_key_name_placeholder").to_string());
            state.set_value(current_name.clone(), window, cx);
            state
        });
        let view = cx.entity().downgrade();

        // 在打开对话框前设置焦点，避免闪烁
        name_input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = name_input.clone();
            let view_for_ok = view.clone();
            let old_name = current_name.clone();

            dialog
                .title(t!("KeyValueView.rename_key_title").to_string())
                .w(px(400.))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("KeyValueView.new_key_name_label").to_string()),
                        )
                        .child(Input::new(&name_input).w_full()),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.ok").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let new_name = input_for_ok.read(cx).text().to_string();
                    if new_name.is_empty() || new_name == old_name {
                        return false;
                    }
                    let _ = view_for_ok.update(cx, |v, cx| {
                        v.rename_key(new_name, cx);
                        window.close_dialog(cx);
                    });
                    false
                })
        });
    }

    /// 重命名键
    fn rename_key(&mut self, new_name: String, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(old_name) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let old_name = old_name.clone();
                let new_name = new_name.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .rename_in_db(db_index, &old_name, &new_name)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.current_key = Some(new_name.clone());
                    view.load_key(connection_id, db_index, new_name, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 保存 String 值
    fn save_string_value(&mut self, cx: &mut Context<Self>) {
        if !self.string_value_is_editable() {
            return;
        }

        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let value = self.get_editor_content(cx);
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let value = value.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .set_in_db(db_index, &key, &value, None)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.is_dirty = false;
                    cx.emit(KeyValueViewEvent::ValueUpdated { key });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 通用删除操作的异步模板
    fn reload_after_operation(&mut self, cx: &mut Context<Self>) {
        if let (Some(conn_id), Some(key)) = (self.connection_id.clone(), self.current_key.clone()) {
            self.load_key(conn_id, self.db_index, key, cx);
        }
    }

    /// 删除 List 元素
    fn delete_list_element(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let delete_marker = "__DELETED_ELEMENT_MARKER__";
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .lset_in_db(db_index, &key, index as i64, delete_marker)
                        .await
                        .map_err(anyhow::Error::new)?;
                    guard
                        .execute_command_in_db(
                            db_index,
                            &format!("LREM {} 1 {}", key, delete_marker),
                        )
                        .await
                        .map_err(anyhow::Error::new)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 删除 Set 元素
    fn delete_set_element(&mut self, member: String, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let member = member.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .srem_in_db(db_index, &key, &[member.as_str()])
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 更新 Set 成员
    fn update_set_member(
        &mut self,
        old_member: String,
        new_member: String,
        cx: &mut Context<Self>,
    ) {
        if !should_replace_set_member(&old_member, &new_member) {
            return;
        }
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let old_member = old_member.clone();
                let new_member = new_member.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .srem_in_db(db_index, &key, &[old_member.as_str()])
                        .await
                        .map_err(anyhow::Error::new)?;
                    guard
                        .sadd_in_db(db_index, &key, &[new_member.as_str()])
                        .await
                        .map_err(anyhow::Error::new)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 删除 ZSet 元素
    fn delete_zset_element(&mut self, member: String, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let member = member.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .zrem_in_db(db_index, &key, &[member.as_str()])
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 删除 Hash 字段
    fn delete_hash_field(&mut self, field: String, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        let Some(key) = self.current_key.clone() else {
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let db_index = self.db_index;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let connection_id = connection_id.clone();
                let key = key.clone();
                let field = field.clone();
                async move {
                    let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                        anyhow::anyhow!("{}", t!("KeyValueView.connection_missing"))
                    })?;
                    let guard = conn.read().await;
                    guard
                        .hdel_in_db(db_index, &key, &[field.as_str()])
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| {
                if result.is_ok() {
                    view.reload_after_operation(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 渲染值编辑器
    fn render_value_editor(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(content) = &self.value_content else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(t!("KeyValueView.select_key_placeholder").to_string())
                .into_any_element();
        };

        match content {
            KeyValueContent::String(_) => self.render_string_editor(cx).into_any_element(),
            KeyValueContent::List(items) => self.render_list_view(items, cx).into_any_element(),
            KeyValueContent::Set(items) => self.render_set_view(items, cx).into_any_element(),
            KeyValueContent::ZSet(items) => self.render_zset_view(items, cx).into_any_element(),
            KeyValueContent::Hash(items) => self.render_hash_view(items, cx).into_any_element(),
            KeyValueContent::Stream(entries) => {
                self.render_stream_view(entries, cx).into_any_element()
            }
            KeyValueContent::None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(t!("KeyValueView.empty_value").to_string())
                .into_any_element(),
        }
    }

    /// 渲染 String 编辑器
    fn render_string_editor(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        Textarea::new(&self.string_editor)
            .size_full()
            .disabled(!self.string_value_is_editable())
    }

    /// 渲染底部状态栏
    fn render_status_bar(&self, _cx: &App) -> impl IntoElement {
        let Some(info) = &self.key_info else {
            return div().into_any_element();
        };

        let size = info.size.unwrap_or(0);
        let content_len = match &self.value_content {
            Some(KeyValueContent::String(s)) => s.len(),
            Some(KeyValueContent::List(v)) => v.len(),
            Some(KeyValueContent::Set(v)) => v.len(),
            Some(KeyValueContent::ZSet(v)) => v.len(),
            Some(KeyValueContent::Hash(v)) => v.len(),
            Some(KeyValueContent::Stream(v)) => v.len(),
            _ => 0,
        };

        let memory_display = info
            .memory_usage
            .map(|m| t!("KeyValueView.status_memory", memory = m).to_string())
            .unwrap_or_default();

        StatusBar::new("redis-key-value-status")
            .muted_background()
            .leading(
                h_flex()
                    .gap_4()
                    .child(
                        div().child(
                            t!("KeyValueView.status_length", count = content_len).to_string(),
                        ),
                    )
                    .when(size > 0, |this| {
                        this.child(
                            div().child(t!("KeyValueView.status_size", size = size).to_string()),
                        )
                    })
                    .when(!memory_display.is_empty(), |this| {
                        this.child(div().child(memory_display.clone()))
                    }),
            )
            .trailing(div().child(self.view_format.display_name()))
            .into_any_element()
    }

    /// 渲染 List 视图
    fn render_list_view(&self, items: &[Vec<u8>], cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        // 应用筛选
        let items = self.apply_filter(items);

        v_flex()
            .id("list-value-scroll")
            .size_full()
            .overflow_scroll()
            .child(self.render_table_header(
                vec![
                    (t!("KeyValueView.column_index").to_string(), 60.0),
                    (t!("KeyValueView.column_value").to_string(), 0.0),
                    (t!("KeyValueView.column_action").to_string(), 120.0),
                ],
                cx,
            ))
            .children(items.into_iter().map({
                let view = view.clone();
                move |(idx, item)| {
                    let view = view.clone();
                    let item_display = redis_bytes_display(&item);
                    let binary_item = match &item_display {
                        RedisBytesDisplay::Binary { .. } => Some(item.clone()),
                        RedisBytesDisplay::Text(_) => None,
                    };
                    let display_value = match &item_display {
                        RedisBytesDisplay::Text(value) => value.clone(),
                        RedisBytesDisplay::Binary { byte_len } => {
                            t!("KeyValueView.binary_value", size = byte_len).to_string()
                        }
                    };
                    let editable_value = redis_bytes_text(&item);
                    let value_for_copy = redis_bytes_copy_text(&item);
                    let value_for_edit = editable_value.clone().unwrap_or_default();
                    let value_for_preview = display_redis_bytes(&item);
                    let preview_title =
                        large_text_preview_title("List item", Some(&format!("#{}", idx + 1)));
                    let binary_file_name = binary_download_file_name("list-item", idx);

                    h_flex()
                        .id(("list-row", idx))
                        .group("list-row")
                        .w_full()
                        .min_h(px(40.0))
                        .px_2()
                        .items_center()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .hover(|this| this.bg(cx.theme().muted))
                        .child(
                            div()
                                .w(px(60.0))
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{}", idx + 1)),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .items_center()
                                .child(div().flex_1().text_base().truncate().child(display_value))
                                .when(binary_item.is_none(), |this| {
                                    this.child(
                                        IconButton::new(("preview-list", idx), IconName::Maximize)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("RedisTool.view_full_value").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let title = preview_title.clone();
                                                let value = value_for_preview.clone();
                                                move |_, window, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.show_large_text_preview_dialog(
                                                            title.clone(),
                                                            value.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .w(px(120.0))
                                .justify_end()
                                .gap_1()
                                .opacity(0.)
                                .group_hover("list-row", |this| this.opacity(1.))
                                .child(
                                    IconButton::new(("copy-list", idx), IconName::Copy)
                                        .glyph_size(IconSize::Default)
                                        .tooltip(if binary_item.is_some() {
                                            t!("KeyValueView.copy_binary_base64").to_string()
                                        } else {
                                            t!("Common.copy").to_string()
                                        })
                                        .on_click({
                                            let value = value_for_copy.clone();
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    value.clone(),
                                                ));
                                            }
                                        }),
                                )
                                .when_some(binary_item.clone(), |this, bytes| {
                                    this.child(
                                        IconButton::new(
                                            ("download-list", idx),
                                            IconName::ArrowDown,
                                        )
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("KeyValueView.download_binary").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            let file_name = binary_file_name.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.download_binary_value(
                                                        bytes.clone(),
                                                        file_name.clone(),
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                    )
                                })
                                .when(binary_item.is_none(), |this| {
                                    this.child(
                                        IconButton::new(("edit-list", idx), IconName::Edit)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(
                                                t!("KeyValueView.edit_list_item", index = idx + 1)
                                                    .to_string(),
                                            )
                                            .on_click({
                                                let view = view.clone();
                                                let value = value_for_edit.clone();
                                                move |_, window, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.show_list_edit_dialog(
                                                            idx,
                                                            value.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    )
                                })
                                .child(
                                    IconButton::new(("delete-list", idx), IconName::Remove)
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("Common.delete").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.delete_list_element(idx, cx);
                                                });
                                            }
                                        }),
                                ),
                        )
                }
            }))
    }

    /// 渲染 Set 视图
    fn render_set_view(&self, items: &[Vec<u8>], cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        // 应用筛选
        let items = self.apply_filter(items);

        v_flex()
            .id("set-value-scroll")
            .size_full()
            .overflow_scroll()
            .child(self.render_table_header(
                vec![
                    (t!("KeyValueView.column_member").to_string(), 0.0),
                    (t!("KeyValueView.column_action").to_string(), 120.0),
                ],
                cx,
            ))
            .children(items.into_iter().map({
                let view = view.clone();
                move |(idx, item)| {
                    let view = view.clone();
                    let member_display = redis_bytes_display(&item);
                    let binary_member = match &member_display {
                        RedisBytesDisplay::Binary { .. } => Some(item.clone()),
                        RedisBytesDisplay::Text(_) => None,
                    };
                    let display_value = match &member_display {
                        RedisBytesDisplay::Text(value) => value.clone(),
                        RedisBytesDisplay::Binary { byte_len } => {
                            t!("KeyValueView.binary_value", size = byte_len).to_string()
                        }
                    };
                    let editable_member = redis_bytes_text(&item);
                    let value_for_copy = redis_bytes_copy_text(&item);
                    let value_for_delete = editable_member.clone().unwrap_or_default();
                    let value_for_edit = editable_member.clone().unwrap_or_default();
                    let value_for_preview = display_redis_bytes(&item);
                    let preview_title = large_text_preview_title("Set member", None);
                    let binary_file_name = binary_download_file_name("set-member", idx);

                    h_flex()
                        .id(("set-row", idx))
                        .group("set-row")
                        .w_full()
                        .min_h(px(40.0))
                        .px_2()
                        .items_center()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .hover(|this| this.bg(cx.theme().muted))
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(IconName::Minus)
                                        .with_size(Size::Small)
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(div().flex_1().text_base().truncate().child(display_value))
                                .when(binary_member.is_none(), |this| {
                                    this.child(
                                        IconButton::new(("preview-set", idx), IconName::Maximize)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("RedisTool.view_full_value").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let title = preview_title.clone();
                                                let value = value_for_preview.clone();
                                                move |_, window, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.show_large_text_preview_dialog(
                                                            title.clone(),
                                                            value.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .w(px(120.0))
                                .justify_end()
                                .gap_1()
                                .opacity(0.)
                                .group_hover("set-row", |this| this.opacity(1.))
                                .child(
                                    IconButton::new(("copy-set", idx), IconName::Copy)
                                        .glyph_size(IconSize::Default)
                                        .tooltip(if binary_member.is_some() {
                                            t!("KeyValueView.copy_binary_base64").to_string()
                                        } else {
                                            t!("Common.copy").to_string()
                                        })
                                        .on_click({
                                            let value = value_for_copy.clone();
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    value.clone(),
                                                ));
                                            }
                                        }),
                                )
                                .when_some(binary_member.clone(), |this, bytes| {
                                    this.child(
                                        IconButton::new(("download-set", idx), IconName::ArrowDown)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("KeyValueView.download_binary").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let file_name = binary_file_name.clone();
                                                move |_, _, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.download_binary_value(
                                                            bytes.clone(),
                                                            file_name.clone(),
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    )
                                })
                                .when(binary_member.is_none(), |this| {
                                    this.child(
                                        IconButton::new(("edit-set", idx), IconName::Edit)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("KeyValueView.edit_set_member").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let member = value_for_edit.clone();
                                                move |_, window, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.show_set_edit_dialog(
                                                            member.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    )
                                    .child(
                                        IconButton::new(("delete-set", idx), IconName::Remove)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("Common.delete").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let member = value_for_delete.clone();
                                                move |_, _, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.delete_set_element(member.clone(), cx);
                                                    });
                                                }
                                            }),
                                    )
                                }),
                        )
                }
            }))
    }

    /// 渲染 ZSet 视图
    fn render_zset_view(&self, items: &[ZSetMember], cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        // 应用筛选和排序
        let filtered_items = self.apply_filter_zset(items);

        // 计算分数范围用于可视化
        let (min_score, max_score) = if filtered_items.is_empty() {
            (0.0, 1.0)
        } else {
            let min = filtered_items
                .iter()
                .map(|(_, m)| m.score)
                .fold(f64::INFINITY, f64::min);
            let max = filtered_items
                .iter()
                .map(|(_, m)| m.score)
                .fold(f64::NEG_INFINITY, f64::max);
            if (max - min).abs() < f64::EPSILON {
                (min - 1.0, max + 1.0)
            } else {
                (min, max)
            }
        };

        v_flex()
            .id("zset-value-scroll")
            .size_full()
            .overflow_scroll()
            .child(self.render_table_header(
                vec![
                    (t!("KeyValueView.column_rank").to_string(), 50.0),
                    (t!("KeyValueView.column_score").to_string(), 140.0),
                    (t!("KeyValueView.column_member").to_string(), 0.0),
                    (t!("KeyValueView.column_action").to_string(), 120.0),
                ],
                cx,
            ))
            .children(filtered_items.into_iter().enumerate().map({
                let view = view.clone();
                move |(display_idx, (original_idx, item))| {
                    let view = view.clone();
                    let member_display = redis_bytes_display(&item.member);
                    let binary_member = match &member_display {
                        RedisBytesDisplay::Binary { .. } => Some(item.member.clone()),
                        RedisBytesDisplay::Text(_) => None,
                    };
                    let display_member = match &member_display {
                        RedisBytesDisplay::Text(value) => value.clone(),
                        RedisBytesDisplay::Binary { byte_len } => {
                            t!("KeyValueView.binary_value", size = byte_len).to_string()
                        }
                    };
                    let editable_member = redis_bytes_text(&item.member);
                    let value_for_copy = zset_member_copy_text(item.score, &item.member);
                    let member_for_edit = editable_member.clone().unwrap_or_default();
                    let score_for_edit = item.score;
                    let member_for_delete = editable_member.clone().unwrap_or_default();
                    let member_for_preview = display_redis_bytes(&item.member);
                    let preview_title = large_text_preview_title(
                        "ZSet member",
                        Some(&format!("score {:.2}", item.score)),
                    );
                    let binary_file_name = binary_download_file_name("zset-member", original_idx);

                    // 计算分数百分比用于可视化柱状图 (0.0-1.0)
                    let score_ratio = if (max_score - min_score).abs() < f64::EPSILON {
                        0.5
                    } else {
                        ((item.score - min_score) / (max_score - min_score)).clamp(0.05, 1.0)
                    };

                    // 排名徽章
                    let rank_display = match display_idx {
                        0 => "🥇".to_string(),
                        1 => "🥈".to_string(),
                        2 => "🥉".to_string(),
                        n => format!("{}", n + 1),
                    };

                    h_flex()
                        .id(("zset-row", original_idx))
                        .group("zset-row")
                        .w_full()
                        .min_h(px(40.0))
                        .px_2()
                        .items_center()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .hover(|this| this.bg(cx.theme().muted))
                        // 排名徽章
                        .child(
                            div()
                                .w(px(50.0))
                                .text_sm()
                                .font_weight(if display_idx < 3 {
                                    gpui::FontWeight::BOLD
                                } else {
                                    gpui::FontWeight::NORMAL
                                })
                                .text_color(if display_idx < 3 {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(rank_display),
                        )
                        // 分数可视化柱状图
                        .child(
                            h_flex()
                                .w(px(140.0))
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .h(px(16.0))
                                        .w(px(60.0))
                                        .rounded(px(2.0))
                                        .bg(cx.theme().muted)
                                        .child(
                                            div()
                                                .h_full()
                                                .w(relative(score_ratio as f32))
                                                .rounded(px(2.0))
                                                .bg(cx.theme().primary),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().primary)
                                        .child(format!("{:.2}", item.score)),
                                ),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .items_center()
                                .child(div().flex_1().text_base().truncate().child(display_member))
                                .when(binary_member.is_none(), |this| {
                                    this.child(
                                        IconButton::new(
                                            ("preview-zset", original_idx),
                                            IconName::Maximize,
                                        )
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("RedisTool.view_full_value").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            let title = preview_title.clone();
                                            let value = member_for_preview.clone();
                                            move |_, window, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.show_large_text_preview_dialog(
                                                        title.clone(),
                                                        value.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .w(px(120.0))
                                .justify_end()
                                .gap_1()
                                .opacity(0.)
                                .group_hover("zset-row", |this| this.opacity(1.))
                                .child(
                                    IconButton::new(("copy-zset", original_idx), IconName::Copy)
                                        .glyph_size(IconSize::Default)
                                        .tooltip(if binary_member.is_some() {
                                            t!("KeyValueView.copy_binary_base64").to_string()
                                        } else {
                                            t!("Common.copy").to_string()
                                        })
                                        .on_click({
                                            let value = value_for_copy.clone();
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    value.clone(),
                                                ));
                                            }
                                        }),
                                )
                                .when_some(binary_member.clone(), |this, bytes| {
                                    this.child(
                                        IconButton::new(
                                            ("download-zset", original_idx),
                                            IconName::ArrowDown,
                                        )
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("KeyValueView.download_binary").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            let file_name = binary_file_name.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.download_binary_value(
                                                        bytes.clone(),
                                                        file_name.clone(),
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                    )
                                })
                                .when(binary_member.is_none(), |this| {
                                    this.child(
                                        IconButton::new(
                                            ("edit-zset", original_idx),
                                            IconName::Edit,
                                        )
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("KeyValueView.edit_zset_member").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            let member = member_for_edit.clone();
                                            let score = score_for_edit;
                                            move |_, window, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.show_zset_edit_dialog(
                                                        member.clone(),
                                                        score,
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                    )
                                    .child(
                                        IconButton::new(
                                            ("delete-zset", original_idx),
                                            IconName::Remove,
                                        )
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("Common.delete").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            let member = member_for_delete.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.delete_zset_element(member.clone(), cx);
                                                });
                                            }
                                        }),
                                    )
                                }),
                        )
                }
            }))
    }

    /// 渲染 Hash 视图
    fn render_hash_view(&self, items: &[HashField], cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        // 应用筛选
        let items = self.apply_filter_hash(items);

        v_flex()
            .id("hash-value-scroll")
            .size_full()
            .overflow_scroll()
            .child(self.render_table_header(
                vec![
                    (t!("KeyValueView.column_field").to_string(), 150.0),
                    (t!("KeyValueView.column_value").to_string(), 0.0),
                    (t!("KeyValueView.column_action").to_string(), 120.0),
                ],
                cx,
            ))
            .children(items.into_iter().map({
                let view = view.clone();
                move |(idx, item)| {
                    let view = view.clone();
                    let field_presentation = redis_bytes_display(&item.field);
                    let value_presentation = redis_bytes_display(&item.value);
                    let binary_field = match &field_presentation {
                        RedisBytesDisplay::Binary { .. } => Some(item.field.clone()),
                        RedisBytesDisplay::Text(_) => None,
                    };
                    let binary_value = match &value_presentation {
                        RedisBytesDisplay::Binary { .. } => Some(item.value.clone()),
                        RedisBytesDisplay::Text(_) => None,
                    };
                    let field_display = match &field_presentation {
                        RedisBytesDisplay::Text(value) => value.clone(),
                        RedisBytesDisplay::Binary { byte_len } => {
                            t!("KeyValueView.binary_value", size = byte_len).to_string()
                        }
                    };
                    let value_display = match &value_presentation {
                        RedisBytesDisplay::Text(value) => value.clone(),
                        RedisBytesDisplay::Binary { byte_len } => {
                            t!("KeyValueView.binary_value", size = byte_len).to_string()
                        }
                    };
                    let editable_field = redis_bytes_text(&item.field);
                    let editable_value = redis_bytes_text(&item.value);
                    let field_for_copy = redis_bytes_pair_copy_text(&item.field, &item.value);
                    let field_for_edit = editable_field.clone().unwrap_or_default();
                    let value_for_edit = editable_value.clone().unwrap_or_default();
                    let field_for_delete = editable_field.clone().unwrap_or_default();
                    let value_for_preview = display_redis_bytes(&item.value);
                    let preview_title =
                        large_text_preview_title("Hash value", Some(&field_display));
                    let binary_field_file_name = binary_download_file_name("hash-field", idx);
                    let binary_value_file_name = binary_download_file_name("hash-value", idx);

                    h_flex()
                        .id(("hash-row", idx))
                        .group("hash-row")
                        .w_full()
                        .min_h(px(40.0))
                        .px_2()
                        .items_center()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .hover(|this| this.bg(cx.theme().muted))
                        .child(
                            div()
                                .w(px(150.0))
                                .text_base()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .truncate()
                                .child(field_display),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .items_center()
                                .child(div().flex_1().text_base().truncate().child(value_display))
                                .when(binary_value.is_none(), |this| {
                                    this.child(
                                        IconButton::new(("preview-hash", idx), IconName::Maximize)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("RedisTool.view_full_value").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let title = preview_title.clone();
                                                let value = value_for_preview.clone();
                                                move |_, window, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.show_large_text_preview_dialog(
                                                            title.clone(),
                                                            value.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .w(px(120.0))
                                .justify_end()
                                .gap_1()
                                .opacity(0.)
                                .group_hover("hash-row", |this| this.opacity(1.))
                                .child(
                                    IconButton::new(("copy-hash", idx), IconName::Copy)
                                        .glyph_size(IconSize::Default)
                                        .tooltip(
                                            if binary_field.is_some() || binary_value.is_some() {
                                                t!("KeyValueView.copy_binary_base64").to_string()
                                            } else {
                                                t!("Common.copy").to_string()
                                            },
                                        )
                                        .on_click({
                                            let value = field_for_copy.clone();
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    value.clone(),
                                                ));
                                            }
                                        }),
                                )
                                .when_some(binary_field.clone(), |this, bytes| {
                                    this.child(
                                        IconButton::new(
                                            ("download-hash-field", idx),
                                            IconName::ArrowDown,
                                        )
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("KeyValueView.download_binary").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            let file_name = binary_field_file_name.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.download_binary_value(
                                                        bytes.clone(),
                                                        file_name.clone(),
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                    )
                                })
                                .when_some(binary_value.clone(), |this, bytes| {
                                    this.child(
                                        IconButton::new(
                                            ("download-hash-value", idx),
                                            IconName::ArrowDown,
                                        )
                                        .glyph_size(IconSize::Default)
                                        .tooltip(t!("KeyValueView.download_binary").to_string())
                                        .on_click({
                                            let view = view.clone();
                                            let file_name = binary_value_file_name.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |v, cx| {
                                                    v.download_binary_value(
                                                        bytes.clone(),
                                                        file_name.clone(),
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                    )
                                })
                                .when(binary_field.is_none() && binary_value.is_none(), |this| {
                                    this.child(
                                        IconButton::new(("edit-hash", idx), IconName::Edit)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("KeyValueView.edit_hash_field").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let field = field_for_edit.clone();
                                                let value = value_for_edit.clone();
                                                move |_, window, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.show_hash_edit_dialog(
                                                            field.clone(),
                                                            value.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    )
                                })
                                .when(binary_field.is_none(), |this| {
                                    this.child(
                                        IconButton::new(("delete-hash", idx), IconName::Remove)
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("Common.delete").to_string())
                                            .on_click({
                                                let view = view.clone();
                                                let field = field_for_delete.clone();
                                                move |_, _, cx| {
                                                    view.update(cx, |v, cx| {
                                                        v.delete_hash_field(field.clone(), cx);
                                                    });
                                                }
                                            }),
                                    )
                                }),
                        )
                }
            }))
    }

    /// 渲染表格头部
    fn render_table_header(
        &self,
        columns: Vec<(String, f32)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut header = h_flex()
            .w_full()
            .h(px(36.0))
            .px_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted);

        let last_index = columns.len().saturating_sub(1);
        for (index, (name, width)) in columns.into_iter().enumerate() {
            let col = div().text_sm().font_weight(gpui::FontWeight::SEMIBOLD);

            if width > 0.0 {
                let col = col.w(px(width));
                if index == last_index {
                    header = header.child(col.text_right().child(name));
                } else {
                    header = header.child(col.child(name));
                }
            } else {
                header = header.child(col.flex_1().child(name));
            }
        }

        header
    }

    /// 渲染 Stream 视图
    fn render_stream_view(
        &self,
        entries: &[crate::StreamEntry],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();
        let muted = cx.theme().muted;
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;

        v_flex()
            .id("stream-value-scroll")
            .size_full()
            .p_2()
            .gap_2()
            .overflow_scroll()
            .children(entries.iter().enumerate().map({
                let view = view.clone();
                move |(entry_idx, entry)| {
                    let view = view.clone();

                    v_flex()
                        .w_full()
                        .p_2()
                        .rounded(px(4.0))
                        .bg(muted)
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(foreground)
                                .child(format!("ID: {}", entry.id)),
                        )
                        .children(entry.fields.iter().enumerate().map({
                            let view = view.clone();
                            let entry_id = entry.id.clone();
                            move |(field_idx, field)| {
                                let field_presentation = redis_bytes_display(&field.field);
                                let value_presentation = redis_bytes_display(&field.value);
                                let binary_field = match &field_presentation {
                                    RedisBytesDisplay::Binary { .. } => Some(field.field.clone()),
                                    RedisBytesDisplay::Text(_) => None,
                                };
                                let binary_value = match &value_presentation {
                                    RedisBytesDisplay::Binary { .. } => Some(field.value.clone()),
                                    RedisBytesDisplay::Text(_) => None,
                                };
                                let field_display = match &field_presentation {
                                    RedisBytesDisplay::Text(value) => value.clone(),
                                    RedisBytesDisplay::Binary { byte_len } => {
                                        t!("KeyValueView.binary_value", size = byte_len).to_string()
                                    }
                                };
                                let value_display = match &value_presentation {
                                    RedisBytesDisplay::Text(value) => value.clone(),
                                    RedisBytesDisplay::Binary { byte_len } => {
                                        t!("KeyValueView.binary_value", size = byte_len).to_string()
                                    }
                                };
                                let title = large_text_preview_title(
                                    "Stream value",
                                    Some(&format!("{} @ {}", field_display, entry_id)),
                                );
                                let value = display_redis_bytes(&field.value);
                                let value_for_copy =
                                    redis_bytes_pair_copy_text(&field.field, &field.value);
                                let field_file_name = binary_download_file_name(
                                    &format!("stream-field-{}", entry_idx + 1),
                                    field_idx,
                                );
                                let value_file_name = binary_download_file_name(
                                    &format!("stream-value-{}", entry_idx + 1),
                                    field_idx,
                                );
                                let row_id = ((entry_idx as u64) << 32) | field_idx as u64;
                                let view = view.clone();

                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted_foreground)
                                            .w(px(100.0))
                                            .child(field_display),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .truncate()
                                            .child(value_display),
                                    )
                                    .when(binary_value.is_none(), |this| {
                                        this.child(
                                            IconButton::new(
                                                ("preview-stream", row_id),
                                                IconName::Maximize,
                                            )
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("RedisTool.view_full_value").to_string())
                                            .on_click(
                                                {
                                                    let view = view.clone();
                                                    move |_, window, cx| {
                                                        view.update(cx, |v, cx| {
                                                            v.show_large_text_preview_dialog(
                                                                title.clone(),
                                                                value.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                },
                                            ),
                                        )
                                    })
                                    .when(
                                        binary_field.is_some() || binary_value.is_some(),
                                        |this| {
                                            this.child(
                                                IconButton::new(
                                                    ("copy-stream", row_id),
                                                    IconName::Copy,
                                                )
                                                .glyph_size(IconSize::Default)
                                                .tooltip(
                                                    t!("KeyValueView.copy_binary_base64")
                                                        .to_string(),
                                                )
                                                .on_click({
                                                    let value = value_for_copy.clone();
                                                    move |_, _, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(
                                                                value.clone(),
                                                            ),
                                                        );
                                                    }
                                                }),
                                            )
                                        },
                                    )
                                    .when_some(binary_field.clone(), |this, bytes| {
                                        this.child(
                                            IconButton::new(
                                                ("download-stream-field", row_id),
                                                IconName::ArrowDown,
                                            )
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("KeyValueView.download_binary").to_string())
                                            .on_click(
                                                {
                                                    let view = view.clone();
                                                    let file_name = field_file_name.clone();
                                                    move |_, _, cx| {
                                                        view.update(cx, |v, cx| {
                                                            v.download_binary_value(
                                                                bytes.clone(),
                                                                file_name.clone(),
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                },
                                            ),
                                        )
                                    })
                                    .when_some(binary_value.clone(), |this, bytes| {
                                        this.child(
                                            IconButton::new(
                                                ("download-stream-value", row_id),
                                                IconName::ArrowDown,
                                            )
                                            .glyph_size(IconSize::Default)
                                            .tooltip(t!("KeyValueView.download_binary").to_string())
                                            .on_click(
                                                {
                                                    let view = view.clone();
                                                    let file_name = value_file_name.clone();
                                                    move |_, _, cx| {
                                                        view.update(cx, |v, cx| {
                                                            v.download_binary_value(
                                                                bytes.clone(),
                                                                file_name.clone(),
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                },
                                            ),
                                        )
                                    })
                            }
                        }))
                }
            }))
    }

    /// 渲染空状态
    fn render_empty_state(&self, _cx: &App) -> impl IntoElement {
        ContentState::empty(t!("KeyValueView.select_key_placeholder")).icon(
            Icon::new(IconName::Database)
                .color()
                .with_size(GpuiIconSize::Large),
        )
    }

    /// 渲染加载状态
    fn render_loading_state(&self, _cx: &App) -> impl IntoElement {
        ContentState::loading(t!("KeyValueView.loading"))
    }

    /// 渲染错误状态
    fn render_error_state(&self, error: &str, _cx: &App) -> impl IntoElement {
        ContentState::error(t!("KeyValueView.load_failed", error = error))
    }
}

impl EventEmitter<KeyValueViewEvent> for KeyValueView {}

impl Focusable for KeyValueView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for KeyValueView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 应用异步加载后的待设置值
        self.apply_pending_editor_value(window, cx);

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .when(matches!(self.load_state, LoadState::Empty), |this| {
                this.child(self.render_empty_state(cx))
            })
            .when(matches!(self.load_state, LoadState::Loading), |this| {
                this.child(self.render_loading_state(cx))
            })
            .when(matches!(self.load_state, LoadState::Error(_)), |this| {
                if let LoadState::Error(ref e) = self.load_state {
                    this.child(self.render_error_state(e, cx))
                } else {
                    this
                }
            })
            .when(matches!(self.load_state, LoadState::Loaded), |this| {
                this.child(self.render_key_info(cx))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(self.render_value_editor(window, cx)),
                    )
                    .child(self.render_status_bar(cx))
            })
    }
}

impl EventEmitter<TabContentEvent> for KeyValueView {}

#[cfg(test)]
mod tests {
    use super::{
        KeyValueContent, KeyValueView, LoadState, RedisBytesDisplay, ViewFormat,
        binary_download_file_name, display_redis_bytes, format_redis_string_value,
        is_binary_redis_string, large_text_preview_title, redis_bytes_copy_text,
        redis_bytes_display, redis_bytes_pair_copy_text, redis_bytes_text,
        should_replace_set_member, zset_member_copy_text,
    };
    use crate::{KeyInfo, RedisKeyType};
    use gpui::{AppContext, TestAppContext, VisualTestContext, WindowOptions};
    use gpui_component::Root;

    #[gpui::test]
    fn delete_key_confirmation_dialog_opens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });

        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity = cx.new(|cx| KeyValueView::new(window, cx));
                    entity.update(cx, |view, cx| {
                        view.connection_id = Some("redis-1".into());
                        view.db_index = 2;
                        view.current_key = Some("session:42".into());
                        view.key_info =
                            Some(KeyInfo::new("session:42".into(), RedisKeyType::String));
                        view.value_content = Some(KeyValueContent::String(b"value".to_vec()));
                        view.load_state = LoadState::Loaded;
                        cx.notify();
                    });
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .expect("open Redis key value test window");
            (window, view.expect("key value view"))
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();

        assert!(view.read_with(&cx, |view, _| view.current_key.is_some()));
        assert!(
            !cx.update(|window, cx| Root::render_dialog_layer(window, cx).is_some()),
            "confirmation dialog should initially be closed"
        );
        view.update_in(&mut cx, |view, window, cx| {
            view.show_delete_dialog(window, cx);
        });
        cx.run_until_parked();

        assert!(
            cx.update(|window, cx| Root::render_dialog_layer(window, cx).is_some()),
            "delete confirmation dialog should be registered with the window root"
        );
    }

    #[test]
    fn delete_key_button_routes_to_confirmation_dialog() {
        let source = include_str!("key_value_view.rs");
        let button_start = source
            .find("IconButton::new(\"delete-key\", IconName::Remove)")
            .expect("delete key button");
        let button_end = source[button_start..]
            .find("// 第二行：筛选 + 操作按钮")
            .map(|offset| button_start + offset)
            .expect("end of key toolbar");
        let delete_button = &source[button_start..button_end];

        assert!(delete_button.contains("move |_, window, cx|"));
        assert!(delete_button.contains("view.show_delete_dialog(window, cx)"));
        assert!(!delete_button.contains("cx.emit(KeyValueViewEvent::ValueDeleted"));
    }

    #[test]
    fn redis_string_format_preserves_utf8_text() {
        let value = "你好，Redis".as_bytes();

        assert_eq!(
            "你好，Redis",
            format_redis_string_value(value, ViewFormat::Raw)
        );
    }

    #[test]
    fn redis_string_format_displays_java_serialized_bytes_losslessly() {
        let value = [0xac, 0xed, 0x00, 0x05, b's', b'r'];

        assert_eq!(
            "\\xac\\xed\\x00\\x05sr",
            format_redis_string_value(&value, ViewFormat::Raw)
        );
        assert_eq!(
            "ac ed 00 05 73 72",
            format_redis_string_value(&value, ViewFormat::Hex)
        );
        assert_eq!(
            "10101100 11101101 00000000 00000101 01110011 01110010",
            format_redis_string_value(&value, ViewFormat::Binary)
        );
        assert_eq!(
            r"\\\xac",
            format_redis_string_value(&[b'\\', 0xac], ViewFormat::Raw)
        );
    }

    #[test]
    fn redis_string_binary_detection_protects_binary_values_from_text_editing() {
        assert!(!is_binary_redis_string("普通文本".as_bytes()));
        assert!(is_binary_redis_string(&[0xac, 0xed, 0x00, 0x05]));
        assert!(is_binary_redis_string(&[0x00, 0x01]));
        assert_eq!(
            "\\x00\\x01",
            format_redis_string_value(&[0x00, 0x01], ViewFormat::Raw)
        );
    }

    #[test]
    fn collection_binary_values_are_lossless_and_read_only() {
        let value = [0x0b, 0xcf, 0xdb, 0xde, 0x01, 0x00];

        assert_eq!(
            "\\x0b\\xcf\\xdb\\xde\\x01\\x00",
            display_redis_bytes(&value)
        );
        assert_eq!(None, redis_bytes_text(&value));
        assert_eq!(
            Some("普通 member".into()),
            redis_bytes_text("普通 member".as_bytes())
        );
    }

    #[test]
    fn binary_zset_member_uses_compact_display_and_base64_copy() {
        let value = [0x0b, 0xcf, 0xdb, 0xde, 0x01, 0x00];

        assert_eq!(
            RedisBytesDisplay::Binary { byte_len: 6 },
            redis_bytes_display(&value)
        );
        assert_eq!("C8/b3gEA", redis_bytes_copy_text(&value));
        assert_eq!("C8/b3gEA", zset_member_copy_text(1.25, &value));
    }

    #[test]
    fn utf8_control_bytes_are_treated_as_binary_collection_values() {
        for (value, expected_base64) in [
            (&[0x00, 0x01][..], "AAE="),
            (&[0x10, 0x11, 0x12, 0x00][..], "EBESAA=="),
        ] {
            assert_eq!(
                RedisBytesDisplay::Binary {
                    byte_len: value.len()
                },
                redis_bytes_display(value)
            );
            assert_eq!(expected_base64, redis_bytes_copy_text(value));
        }
    }

    #[test]
    fn text_zset_member_keeps_plain_text_display_and_copy() {
        let value = "普通 member".as_bytes();

        assert_eq!(
            RedisBytesDisplay::Text("普通 member".into()),
            redis_bytes_display(value)
        );
        assert_eq!("普通 member", redis_bytes_copy_text(value));
        assert_eq!("1.25: 普通 member", zset_member_copy_text(1.25, value));
    }

    #[test]
    fn binary_download_file_name_is_stable_and_one_based() {
        assert_eq!(
            "redis-zset-member-1.bin",
            binary_download_file_name("zset-member", 0)
        );
        assert_eq!(
            "redis-list-item-3.bin",
            binary_download_file_name("list-item", 2)
        );
    }

    #[test]
    fn hash_and_stream_pairs_copy_binary_parts_as_base64() {
        assert_eq!(
            "field: C8/b3gEA",
            redis_bytes_pair_copy_text(b"field", &[0x0b, 0xcf, 0xdb, 0xde, 0x01, 0x00])
        );
        assert_eq!(
            "AP8=: value",
            redis_bytes_pair_copy_text(&[0x00, 0xff], b"value")
        );
        assert_eq!(
            "ZmllbGT/: AP8=",
            redis_bytes_pair_copy_text(&[0x66, 0x69, 0x65, 0x6c, 0x64, 0xff], &[0x00, 0xff])
        );
    }

    #[test]
    fn all_collection_views_wire_binary_download_actions() {
        let source = include_str!("key_value_view.rs");

        for action in [
            "download-list",
            "download-set",
            "download-zset",
            "download-hash-field",
            "download-hash-value",
            "download-stream-field",
            "download-stream-value",
        ] {
            assert!(
                source.contains(action),
                "missing binary download action: {action}"
            );
        }
    }

    #[test]
    fn stream_entry_id_uses_readable_foreground_color() {
        let source = include_str!("key_value_view.rs");
        let stream_renderer = source
            .split("fn render_stream_view")
            .nth(1)
            .expect("stream renderer");
        let stream_renderer = stream_renderer
            .split("fn render_hash_view")
            .next()
            .expect("end of stream renderer");

        assert!(stream_renderer.contains("let foreground = cx.theme().foreground;"));
        assert!(stream_renderer.contains(".text_color(foreground)"));
        assert!(!stream_renderer.contains(".text_color(accent)"));
    }

    #[test]
    fn large_text_preview_title_formats_context() {
        assert_eq!(
            "List item #1",
            large_text_preview_title("List item", Some("#1"))
        );
    }

    #[test]
    fn large_text_preview_title_omits_blank_context() {
        assert_eq!("Set member", large_text_preview_title("Set member", None));
    }

    #[test]
    fn should_replace_set_member_detects_changes() {
        assert!(should_replace_set_member("old", "new"));
        assert!(!should_replace_set_member("same", "same"));
    }
}

impl TabContent for KeyValueView {
    fn content_key(&self) -> &'static str {
        "KeyValue"
    }

    fn title(&self, _cx: &App) -> SharedString {
        if self.closeable {
            self.current_key
                .clone()
                .unwrap_or_else(|| t!("KeyValueView.tab_title_default").to_string())
                .into()
        } else {
            "Value".into()
        }
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Key).with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        self.closeable
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<bool> {
        Task::ready(true)
    }
}
