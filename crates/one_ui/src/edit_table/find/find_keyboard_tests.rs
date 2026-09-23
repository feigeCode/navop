//! 表内查找的键盘派发测试
//!
//! 查找键位绑在 `EditTable` 键盘上下文上，而这个上下文挂在表格自身节点。
//! 宿主（页签、面板）必须把焦点交给 [`EditTableState::table_focus_handle`]，
//! 否则焦点停在宿主外壳上时，`EditTable` 不在窗口的焦点路径里，
//! Cmd/Ctrl+F 不会有任何反应——这正是「浏览表时按 Cmd/Ctrl+F 没反应」的机制。

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, Styled as _, TestAppContext,
    VisualTestContext, Window, WindowOptions, div,
};
use gpui_component::Root;

use crate::edit_table::{
    Column, EditTable, EditTableDelegate, EditTableState, FindNext, HOSTED_FIND_CONTEXT,
    TableKeybindings,
};

/// 最小 delegate：3 行 × 2 列，够驱动真实表格与查找。
struct FindTestDelegate {
    rows: Vec<Vec<String>>,
}

/// 最小 delegate：命中只落在最后一列，列宽之和远超窗口宽度。
///
/// 「切换下一个命中时只滚行不滚列」的回归就靠它：命中在视口右侧以外的
/// 列上时，横向偏移必须跟着动。
struct WideFindTestDelegate {
    rows: Vec<Vec<String>>,
    columns: usize,
}

impl EditTableDelegate for WideFindTestDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        let name = SharedString::from(format!("col-{col_ix}"));
        Column::new(name.clone(), name).width(gpui::px(WIDE_COLUMN_WIDTH))
    }

    fn get_cell_value(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        self.rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
            .unwrap_or_default()
    }

    fn find_in_table_enabled(&self, _cx: &App) -> bool {
        true
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<EditTableState<Self>>,
    ) -> impl IntoElement {
        let text = self.get_cell_value(row_ix, col_ix, cx);
        div().child(text)
    }
}

/// 单列宽度：8 列合计远大于默认测试窗口宽度，保证最后一列不在视口内。
const WIDE_COLUMN_WIDTH: f32 = 400.;

impl EditTableDelegate for FindTestDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        2
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        let name = SharedString::from(format!("col-{col_ix}"));
        Column::new(name.clone(), name)
    }

    fn get_cell_value(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        self.rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
            .unwrap_or_default()
    }

    fn find_in_table_enabled(&self, _cx: &App) -> bool {
        true
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<EditTableState<Self>>,
    ) -> impl IntoElement {
        let text = self.get_cell_value(row_ix, col_ix, cx);
        div().child(text)
    }
}

/// 复刻宿主页签：外壳自己可聚焦，表格挂在内部。
///
/// `forwards_focus_to_table` 为 `true` 时行为和修好后的
/// `TableDataTabContent` 一致——`focus_handle` 直接返回表格内部的句柄。
struct FindTestHost {
    table: Entity<EditTableState<FindTestDelegate>>,
    shell_focus: FocusHandle,
    forwards_focus_to_table: bool,
    /// 复刻数据网格工具栏：搜索框在表格之外，靠宿主上下文拿上下跳键位。
    hosts_find_bar: bool,
}

impl FindTestHost {
    fn new(
        forwards_focus_to_table: bool,
        hosts_find_bar: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| {
            EditTableState::new(
                FindTestDelegate {
                    rows: vec![
                        vec!["alice".into(), "1".into()],
                        vec!["bob".into(), "2".into()],
                        vec!["carol".into(), "3".into()],
                    ],
                },
                window,
                cx,
            )
        });

        Self {
            table,
            shell_focus: cx.focus_handle(),
            forwards_focus_to_table,
            hosts_find_bar,
        }
    }
}

impl Render for FindTestHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div().size_full().track_focus(&self.shell_focus);
        if self.hosts_find_bar {
            root = root.key_context(HOSTED_FIND_CONTEXT).on_action(cx.listener(
                |this, _: &FindNext, _window, cx| {
                    this.table.update(cx, |table, cx| table.find_next(cx));
                },
            ));
        }
        root.child(EditTable::new(&self.table))
    }
}

impl Focusable for FindTestHost {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.forwards_focus_to_table {
            self.table.read(cx).table_focus_handle()
        } else {
            self.shell_focus.clone()
        }
    }
}

/// 宽表宿主：只需要一个能把 `WideFindTestDelegate` 渲染出来的容器。
struct WideFindTestHost {
    table: Entity<EditTableState<WideFindTestDelegate>>,
}

impl Render for WideFindTestHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(EditTable::new(&self.table))
    }
}

fn find_keystroke() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-f"
    }
}

fn find_next_keystroke() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-g"
    } else {
        "ctrl-g"
    }
}

fn open_host(
    cx: &mut TestAppContext,
    forwards_focus_to_table: bool,
) -> (VisualTestContext, Entity<FindTestHost>) {
    open_host_with(cx, forwards_focus_to_table, false)
}

fn open_host_with(
    cx: &mut TestAppContext,
    forwards_focus_to_table: bool,
    hosts_find_bar: bool,
) -> (VisualTestContext, Entity<FindTestHost>) {
    cx.update(gpui_component::init);
    cx.update(|cx| crate::edit_table::init(cx, &TableKeybindings::default()));

    let (window, host) = cx.update(|cx| {
        let mut host = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|cx| {
                    FindTestHost::new(forwards_focus_to_table, hosts_find_bar, window, cx)
                });
                host = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .expect("open table find test window");
        (window, host.expect("host view"))
    });

    let cx = VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    (cx, host)
}

/// 复刻页签容器激活页签时的 `focus(content.focus_handle())`。
fn activate_host(cx: &mut VisualTestContext, host: &Entity<FindTestHost>) {
    let handle = cx.read(|cx| host.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.run_until_parked();
}

#[gpui::test]
fn find_panel_stays_closed_when_focus_only_sits_on_the_host_shell(cx: &mut TestAppContext) {
    let (mut cx, host) = open_host(cx, false);
    activate_host(&mut cx, &host);

    cx.simulate_keystrokes(find_keystroke());
    cx.run_until_parked();

    // 这就是「浏览表时按 Cmd/Ctrl+F 没反应」的机制：焦点只在宿主外壳上时
    // `EditTable` 不在焦点路径里，按键落空。宿主必须把焦点交给表格。
    assert!(
        !cx.read(|cx| host.read(cx).table.read(cx).find_panel_open()),
        "焦点不在表格上时不应打开查找面板"
    );
}

/// 打开一个 8 列 × 400px 的宽表窗口，命中只放在最后一列。
fn open_wide_host(cx: &mut TestAppContext) -> (VisualTestContext, Entity<WideFindTestHost>) {
    const COLUMNS: usize = 8;
    cx.update(gpui_component::init);
    cx.update(|cx| crate::edit_table::init(cx, &TableKeybindings::default()));

    let (window, host) = cx.update(|cx| {
        let mut host = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|cx| {
                    let mut hit_row = vec!["x".to_string(); COLUMNS];
                    hit_row[COLUMNS - 1] = "needle".to_string();
                    let rows = vec![vec!["x".to_string(); COLUMNS], hit_row];
                    let table = cx.new(|cx| {
                        EditTableState::new(
                            WideFindTestDelegate {
                                rows,
                                columns: COLUMNS,
                            },
                            window,
                            cx,
                        )
                    });
                    WideFindTestHost { table }
                });
                host = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .expect("open wide table find test window");
        (window, host.expect("wide host view"))
    });

    let cx = VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    (cx, host)
}

#[gpui::test]
fn hosted_find_runs_on_the_query_the_host_sends(cx: &mut TestAppContext) {
    let (mut cx, host) = open_host(cx, true);

    let (panel_open, total, current) = cx.update(|_window, cx| {
        host.update(cx, |host, cx| {
            host.table.update(cx, |table, cx| {
                // 宿主自己渲染输入框（数据网格工具栏的搜索框）。
                table.set_find_hosted(true, cx);
                assert!(!table.find_panel_open(), "宿主渲染输入框时不应浮出面板");

                table.set_find_query("bob", cx);
                (
                    table.find_panel_open(),
                    table.find_total(),
                    table.find_current(),
                )
            })
        })
    });

    assert!(!panel_open, "查询进行中也不该出现第二个输入框");
    assert_eq!(1, total, "命中应由宿主送入的查询词算出来");
    assert_eq!(1, current, "有命中时应直接停在第一个命中上");
}

#[gpui::test]
fn hosted_find_clears_the_matches_when_the_query_disappears(cx: &mut TestAppContext) {
    let (mut cx, host) = open_host(cx, true);

    let total = cx.update(|_window, cx| {
        host.update(cx, |host, cx| {
            host.table.update(cx, |table, cx| {
                table.set_find_hosted(true, cx);
                table.set_find_query("bob", cx);
                // 清空输入框（输入框右侧的 × 或 Escape）即结束查找。
                table.set_find_query("", cx);
                table.find_total()
            })
        })
    });

    assert_eq!(0, total, "清空查询词后不应残留高亮");
}

#[gpui::test]
fn cmd_g_still_advances_the_match_while_focus_sits_in_the_hosts_find_bar(cx: &mut TestAppContext) {
    // 搜索框搬到工具栏后，焦点在框里——那已经不在 `EditTable` 节点下了，
    // 表格那套 Cmd+G 绑定落不到它头上。宿主必须挂上 `HOSTED_FIND_CONTEXT`。
    let (mut cx, host) = open_host_with(cx, false, true);
    activate_host(&mut cx, &host);

    cx.update(|_window, cx| {
        host.update(cx, |host, cx| {
            host.table.update(cx, |table, cx| {
                table.set_find_hosted(true, cx);
                table.set_find_query("a", cx);
                assert_eq!(2, table.find_total(), "alice 与 carol 各命中一次");
                assert_eq!(1, table.find_current());
            })
        })
    });
    cx.run_until_parked();

    cx.simulate_keystrokes(find_next_keystroke());
    cx.run_until_parked();

    assert_eq!(
        2,
        cx.read(|cx| host.read(cx).table.read(cx).find_current()),
        "在搜索框里按 Cmd/Ctrl+G 应跳到下一个命中"
    );
}

#[gpui::test]
fn navigating_to_a_match_outside_the_viewport_scrolls_the_columns_too(cx: &mut TestAppContext) {
    let (mut cx, host) = open_wide_host(cx);

    let offset_before = cx.read(|cx| {
        host.read(cx)
            .table
            .read(cx)
            .horizontal_scroll_handle
            .offset()
            .x
    });

    // 命中只在最后一列：这一列远在视口右侧之外。
    cx.update(|_window, cx| {
        host.update(cx, |host, cx| {
            host.table.update(cx, |table, cx| {
                table.set_find_hosted(true, cx);
                table.set_find_query("needle", cx);
                assert_eq!(1, table.find_total());
            })
        })
    });
    cx.run_until_parked();

    let offset_after = cx.read(|cx| {
        host.read(cx)
            .table
            .read(cx)
            .horizontal_scroll_handle
            .offset()
            .x
    });

    // 只滚行不滚列时这里会一动不动：用户只看到行在动、高亮始终在屏幕之外。
    assert!(
        offset_after < offset_before,
        "命中在视口右侧以外时视口应当横向移动（{offset_before:?} -> {offset_after:?}）"
    );
}

#[gpui::test]
fn find_panel_opens_when_the_host_hands_focus_to_the_table(cx: &mut TestAppContext) {
    let (mut cx, host) = open_host(cx, true);
    activate_host(&mut cx, &host);

    cx.simulate_keystrokes(find_keystroke());
    cx.run_until_parked();

    assert!(
        cx.read(|cx| host.read(cx).table.read(cx).find_panel_open()),
        "宿主把焦点交给 table_focus_handle 后，Cmd/Ctrl+F 应打开表格内查找"
    );
}
