use std::collections::HashMap;

use crate::home_tab::HomePage;
use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, Hsla, InteractiveElement, IntoElement,
    ParentElement, Pixels, Styled, UniformListScrollHandle, Window, deferred, div, hsla, px,
};
use gpui_component::input::{InputEvent, InputState};

mod batch_toolbar;
mod connection_command;
mod connection_context_menu;
mod connection_copy;
mod connection_copy_menu;
mod connection_share;
mod context_menu;
mod drag;
mod filter_bar;
mod header_actions;
mod resize;
#[cfg(test)]
mod resize_contract_tests;
mod row_parts;
mod rows;
mod selection;
mod state;
mod tree;
mod tree_model;
mod workspace_context_menu;

pub(crate) use connection_context_menu::build_connection_context_menu;
pub(crate) use connection_share::connection_full_info_text;

#[derive(Clone, Copy)]
pub(super) struct SidebarPalette {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
}

impl SidebarPalette {
    fn app(sidebar: &PersistentConnectionSidebar, cx: &gpui::App) -> Self {
        use gpui_component::ActiveTheme as _;

        let dark_mode = cx.theme().is_dark();
        // 外壳托管的连接列表面板比图标栏/Tab 栏（外壳 token）亮一档：
        // 和右边白色内容拉开层次，又不至于和左侧 52px 图标栏糊成同一块灰。
        let background = if sidebar.home_navigation_layout {
            cx.theme().sidebar
        } else if sidebar.home_embedded {
            // 嵌入主页内容里的树仍是内容表面，跟随主页画布。
            crate::chrome_palette::chrome_palette(cx).content
        } else {
            cx.theme().sidebar
        };
        Self {
            background,
            foreground: cx.theme().sidebar_foreground,
            // sidebar_accent 已变为主页导航选中浅蓝；连接树行的 hover/徽标底保持中性，
            // 不随主页侧栏主题补丁变色（refinement §5.3）。
            muted: cx.theme().muted,
            // 行悬停落在面板底色上：外壳灰下内容表面的中性 hover 几乎看不出来，
            // 所以壳里改用「贴着底色抬一层」的色调。
            hover: if sidebar.home_navigation_layout {
                hover_tint(background, dark_mode)
            } else {
                cx.theme().list_hover
            },
            selected: cx.theme().list_active,
            muted_foreground: cx.theme().muted_foreground,
            border: cx.theme().sidebar_border,
        }
    }
}

impl From<domain_api::SidebarColors> for SidebarPalette {
    fn from(colors: domain_api::SidebarColors) -> Self {
        Self {
            background: colors.background,
            foreground: colors.foreground,
            muted: colors.muted,
            hover: colors.muted,
            selected: hsla(colors.accent.h, colors.accent.s, colors.accent.l, 0.18),
            muted_foreground: colors.muted_foreground,
            border: colors.border,
        }
    }
}

/// 行悬停底色：贴着面板底色抬一层——浅色壳上加深、深色壳上提亮。
/// 内容表面的中性 `list_hover` 落在外壳灰上几乎看不出变化，需要单独推导。
fn hover_tint(background: Hsla, dark_mode: bool) -> Hsla {
    let amount = if dark_mode { 0.035 } else { -0.028 };
    hsla(
        background.h,
        background.s,
        (background.l + amount).clamp(0.0, 1.0),
        background.a,
    )
}

/// 浮动连接树卡片与窗口边缘的间距（像素）。
const FLOATING_CARD_MARGIN: f32 = 8.0;

pub(crate) struct PersistentConnectionSidebar {
    pub(super) home_page: Entity<HomePage>,
    pub(super) tree_expanded: bool,
    pub(super) selected_filter: crate::home_tab::ConnectionFilter,
    pub(super) hide_empty_workspaces: bool,
    pub(super) auto_hide_tree: bool,
    pub(super) search_input: Entity<InputState>,
    tree_width: Pixels,
    /// 最近一次落盘的宽度，用于拖拽过程中的增量持久化判断。
    persisted_tree_width: Pixels,
    terminal_palette: Option<SidebarPalette>,
    pub(super) tree_scroll_handle: UniformListScrollHandle,
    /// Tree 布局把侧栏作为主页内容渲染时为 true；渲染主体仍由 Render 承担，
    /// 避免在 HomePage 自身 render 租约内 read(home_page) 造成重入。
    home_embedded: bool,
    home_navigation_layout: bool,
    /// 连接级 AI 访问策略缓存（issue #437）：侧栏行徽标用。
    /// 只缓存「非默认」的级别，缺省即 `Allowed`。
    pub(super) ai_access_levels: HashMap<i64, tool_runtime::AiAccessLevel>,
}

pub(crate) enum PersistentConnectionSidebarEvent {
    TreeVisibilityChanged { expanded: bool },
}

impl PersistentConnectionSidebar {
    /// 批量读取连接级 AI 访问策略（只含「非默认」的项）。
    fn refresh_ai_access_levels(&mut self, cx: &mut Context<Self>) {
        self.ai_access_levels = load_ai_access_levels(cx);
    }
}

/// 批量读取所有「非默认」的 AI 访问策略；存储不可用时返回空表（等价全部允许）。
fn load_ai_access_levels(cx: &gpui::App) -> HashMap<i64, tool_runtime::AiAccessLevel> {
    let Some(storage) = cx
        .try_global::<one_core::storage::GlobalStorageState>()
        .map(|state| state.storage.clone())
    else {
        return HashMap::new();
    };
    storage
        .get::<one_core::storage::ConnectionRepository>()
        .and_then(|repo| repo.ai_access_levels().ok())
        .unwrap_or_default()
}

/// 读取单条连接的 AI 访问级别（右键菜单用，顺带避开可能过期的缓存）。
pub(super) fn load_ai_access_level(
    connection_id: i64,
    cx: &gpui::App,
) -> tool_runtime::AiAccessLevel {
    let Some(storage) = cx
        .try_global::<one_core::storage::GlobalStorageState>()
        .map(|state| state.storage.clone())
    else {
        return tool_runtime::AiAccessLevel::default();
    };
    storage
        .get::<one_core::storage::ConnectionRepository>()
        .and_then(|repo| repo.ai_access_level(connection_id).ok())
        .unwrap_or_default()
}

impl EventEmitter<PersistentConnectionSidebarEvent> for PersistentConnectionSidebar {}

impl gpui::Render for PersistentConnectionSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.home_navigation_layout {
            if self.tree_expanded {
                self.render_home_navigation_tree(px(248.0), cx)
            } else {
                div().into_any_element()
            }
        } else if self.home_embedded {
            self.render_home_tree(cx)
        } else {
            // 侧栏由 App 外壳以停靠/浮动形式渲染；空占位避免误作窗口根。
            div().into_any_element()
        }
    }
}

impl PersistentConnectionSidebar {
    /// Render the connection tree as a floating card that overlays the main
    /// content instead of occupying flex space, so expanding it no longer
    /// squeezes the terminal. 调用方（外壳）用 `left` 把它摆到应用菜单右缘，
    /// 并负责在点击内容区或打开连接后收起。
    pub(crate) fn render_floating_tree(
        &mut self,
        left: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette(cx);
        let layout = one_ui::theme_geometry().layout;
        let top = layout.tab_bar;
        let pad = px(FLOATING_CARD_MARGIN);
        // 必须把**绘制**放进 gpui 的 deferred 层（overlay surface），否则盖不住原生
        // Windows RDP 会话：那个会话的光栅化被合成成一个 portal visual，位于
        // 「普通元素树之上、deferred 层之下」。也就是说这个 `div().absolute()` 浮层
        // 作为普通元素树内容，必然被远端画面盖住 —— 只有 deferred 内容才在它上面。
        // gpui `has_deferred_content` 的文档把这点写死了：「普通元素树内容无法表达
        // 『盖住原生子窗口』」。dialog / 菜单能盖住，正是因为 gpui-kit 用
        // `deferred(anchored(...))` 画它们。
        //
        // `deferred` 只推迟**绘制**，布局仍留在原处（`Deferred::request_layout` 直接
        // 转交 child），所以侧栏的尺寸、位置、命中测试、occlude 与滚动都不受影响；
        // 它同时是 `Window::has_deferred_content()` 的输入，侧栏可见期间原生会话会
        // 照旧「按需 cloak 让位」，让 visual 不再被原始子窗口压住。
        deferred(
            div()
                .absolute()
                .top(top)
                .bottom_0()
                .left(left)
                .w(self.tree_width + pad)
                // 浮动侧边栏覆盖在内容区之上：occlude 让命中测试在侧边栏处终止，
                // 避免滚轮/鼠标事件穿透到下方的 tab 内容区（否则终端等会跟着滚动）。
                .occlude()
                .pt(pad)
                .pl(pad)
                .pb(pad)
                .child(
                    div()
                        .size_full()
                        .overflow_hidden()
                        .rounded_lg()
                        .border_1()
                        .border_color(palette.border.opacity(0.6))
                        .shadow_lg()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(self.render_connection_tree(palette, window, cx)),
                ),
        )
        .into_any_element()
    }

    pub(crate) fn new(
        home_page: Entity<HomePage>,
        tree_expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 主页任何状态变化（搜索词、类型筛选、批量选择、数据刷新）都需要重绘树；
        // 批量选择的失效裁剪由 HomePage::prune_connection_selection 在数据加载时完成。
        // 保存/右键切换后都靠这次重读刷新徽标（连接列表数据本身已带策略）。
        cx.observe(&home_page, |this, _, cx| {
            this.refresh_ai_access_levels(cx);
            cx.notify();
        })
        .detach();
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("Connection.search_placeholder").to_string())
                .clean_on_escape()
        });
        cx.subscribe_in(&search_input, window, |_, _, event, _, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })
        .detach();
        let tree_state = one_core::settings::AppSettings::current(cx).connection_sidebar_tree_state;
        let layout = one_ui::theme_geometry().layout;
        let tree_width = px(tree_state.tree_width as f32)
            .clamp(layout.context_sidebar_min, layout.context_sidebar_max);
        Self {
            home_page,
            tree_expanded,
            selected_filter: crate::home_tab::ConnectionFilter::All,
            hide_empty_workspaces: tree_state.hide_empty_workspaces,
            auto_hide_tree: tree_state.auto_hide_tree,
            search_input,
            tree_width,
            persisted_tree_width: tree_width,
            terminal_palette: None,
            tree_scroll_handle: UniformListScrollHandle::new(),
            home_embedded: false,
            home_navigation_layout: false,
            ai_access_levels: load_ai_access_levels(cx),
        }
    }

    pub(crate) fn set_home_embedded(&mut self, embedded: bool, cx: &mut Context<Self>) {
        if self.home_embedded != embedded || self.home_navigation_layout {
            self.home_embedded = embedded;
            self.home_navigation_layout = false;
            cx.notify();
        }
    }

    pub(crate) fn set_tree_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.tree_expanded != expanded {
            self.tree_expanded = expanded;
            cx.notify();
        }
    }

    pub(crate) fn is_expanded(&self) -> bool {
        self.tree_expanded
    }

    pub(crate) fn is_auto_hide_tree(&self) -> bool {
        self.auto_hide_tree
    }

    /// 同一个连接树 Entity 由外壳按资源计划换宿主；不创建第二棵树或重新加载连接。
    pub(crate) fn render_application_resource(
        &mut self,
        width: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if one_core::settings::AppSettings::global(cx).app_layout
            == one_core::settings::AppLayout::GlobalNavigation
        {
            self.home_embedded = true;
            self.home_navigation_layout = true;
            self.render_tree_impl(Some(width), cx)
        } else {
            self.render_home_navigation_tree(width, cx)
        }
    }

    #[cfg(feature = "shell")]
    pub(crate) fn set_terminal_palette(
        &mut self,
        palette: Option<SidebarPalette>,
        cx: &mut Context<Self>,
    ) {
        self.terminal_palette = palette;
        cx.notify();
    }

    pub(super) fn palette(&self, cx: &gpui::App) -> SidebarPalette {
        if let Some(palette) = self.terminal_palette {
            return palette;
        }
        SidebarPalette::app(self, cx)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn floating_tree_renders_as_card_overlay_and_keeps_auto_collapse_paths() {
        // mod.rs 顶部声明了 #[cfg(test)] 子模块，不能按该标记截断实现部分
        let implementation = include_str!("mod.rs");
        let state = include_str!("state.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();

        // 浮动模式保留：overlay 覆盖 + occlude 阻断事件穿透
        assert!(implementation.contains("fn render_floating_tree"));
        assert!(implementation.contains(".occlude()"));
        // 浮层卡片样式：圆角、阴影、与窗口边缘的间距
        assert!(implementation.contains(".rounded_lg()"));
        assert!(implementation.contains(".shadow_lg()"));
        assert!(implementation.contains("FLOATING_CARD_MARGIN"));
        // 浮动侧边栏必须画在 gpui 的 deferred 层，否则会被原生 Windows RDP 会话的
        // 合成 visual 盖住：普通元素树内容在它下面，deferred 内容才在它上面。
        // 断言按函数体切片，不用整文件 contains —— 免得被别处的 `deferred(` 满足。
        let floating = implementation
            .split("fn render_floating_tree")
            .nth(1)
            .and_then(|rest| rest.split("pub(crate) fn new(").next())
            .expect("render_floating_tree source");
        assert!(floating.contains("deferred("));
        assert!(floating.contains(".into_any_element()"));
        // 旧的「零尺寸 deferred 标记」写法不够：布局在原处、绘制也在原处，侧栏仍然
        // 留在普通内容层，盖不住 native 子窗口。
        assert!(!floating.contains(".child(deferred("));
        // 自动收起路径保留：打开连接后
        assert!(state.contains("fn collapse_after_open"));
    }

    #[test]
    fn connection_tree_separates_home_layers_without_stacked_borders() {
        let implementation = include_str!("mod.rs");
        let tree = include_str!("tree.rs");
        let tree_implementation = tree.split("#[cfg(test)]").next().unwrap();

        // 外壳托管的连接列表面板比壳亮一档（sidebar token），右侧内容才保持白底；
        // 嵌入主页内容里的树才跟随内容表面。
        assert!(implementation.contains("if sidebar.home_navigation_layout {"));
        assert!(implementation.contains("cx.theme().sidebar"));
        assert!(implementation.contains("crate::chrome_palette::chrome_palette(cx).content"));
        assert!(implementation.contains("SidebarPalette::app(self, cx)"));
        // 外壳灰上的行悬停要贴着底色抬一层，内容表面的 list_hover 在这里看不见。
        assert!(implementation.contains("hover_tint(background, dark_mode)"));
        assert!(!tree_implementation.contains(".border_r_1()"));
        assert!(!tree_implementation.contains(".border_b_1()"));
        // 外壳托管时左侧是应用菜单、同属一个壳，不画内部接缝；嵌进画布的树才用左边线。
        assert!(tree_implementation.contains("self.home_embedded && !self.home_navigation_layout"));
        assert!(tree_implementation.contains("tree.border_l_1().border_color(palette.border)"));
    }

    #[test]
    fn home_tree_renders_via_own_render_lease_not_inline_home_render() {
        // 回归：HomePage::render 内联调用 render_home_tree 会在 HomePage 租约内
        // 再 read(home_page)，触发 "cannot read while being updated" panic。
        // 正确路径：侧栏作为子实体渲染，Render 内按 home_embedded 输出主页树。
        let content = include_str!("../home_tab/content.rs");
        let implementation = include_str!("mod.rs");

        assert!(!content.contains("render_home_tree(cx)"));
        // Tree 布局经 set_home_embedded 让侧栏以子实体输出主页树；
        // 全局导航布局由 home_navigation_layout 单独驱动，不走该入口。
        assert!(content.contains("sidebar.set_home_embedded("));
        assert!(content.contains("self.connection_layout == ConnectionLayout::Tree"));
        assert!(content.contains("!HomePage::uses_global_navigation_layout(cx)"));
        assert!(implementation.contains("impl gpui::Render for PersistentConnectionSidebar"));
        assert!(implementation.contains("self.home_embedded"));
    }
}
