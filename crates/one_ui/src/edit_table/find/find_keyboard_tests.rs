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

use crate::edit_table::{Column, EditTable, EditTableDelegate, EditTableState, TableKeybindings};

/// 最小 delegate：3 行 × 2 列，够驱动真实表格与查找。
struct FindTestDelegate {
    rows: Vec<Vec<String>>,
}

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
}

impl FindTestHost {
    fn new(forwards_focus_to_table: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        }
    }
}

impl Render for FindTestHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.shell_focus)
            .child(EditTable::new(&self.table))
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

fn find_keystroke() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-f"
    }
}

fn open_host(
    cx: &mut TestAppContext,
    forwards_focus_to_table: bool,
) -> (VisualTestContext, Entity<FindTestHost>) {
    cx.update(gpui_component::init);
    cx.update(|cx| crate::edit_table::init(cx, &TableKeybindings::default()));

    let (window, host) = cx.update(|cx| {
        let mut host = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|cx| FindTestHost::new(forwards_focus_to_table, window, cx));
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
