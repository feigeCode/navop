//! 用真实 GPUI 窗口和鼠标事件验证外壳，而不是匹配实现字符串。
//!
//! 这是完整 NavopApp 的集成测试，默认不注册：HomePage::new 会恢复主密钥并加载业务状态，
//! 不能把跨 crate 的 cfg(test) 当成真实用户数据隔离保证。需要专门的隔离运行环境后，
//! 再显式启用 `NAVOP_ENABLE_FULL_SHELL_LAYOUT_TESTS=1` 编译运行。
#![cfg(feature = "full-shell-layout-tests")]

use crate::navop_app::NavopApp;
use gpui::{
    AppContext, Bounds, Entity, MouseButton, Size, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, px,
};
use gpui_component::Root;
use one_core::settings::{AppLayout, AppSettings};
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, StorageManager, WorkspaceRepository,
    connection::SqliteConnection, migration::run_migrations,
};

fn open_shell(
    cx: &mut TestAppContext,
    size: Size<gpui::Pixels>,
) -> (VisualTestContext, Entity<NavopApp>) {
    let temp = tempfile::tempdir().expect("临时测试数据库目录");
    let connection = SqliteConnection::open(temp.path().join("layout.db")).expect("打开测试数据库");
    connection
        .with_connection(|connection| run_migrations(connection))
        .expect("初始化测试数据库");
    let storage = StorageManager::new_with_connection(connection.clone());
    storage.register(ConnectionRepository::new(connection.clone()));
    storage.register(WorkspaceRepository::new(connection));

    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(GlobalStorageState {
            storage: storage.clone(),
        });
        one_core::llm::init(cx);
        one_core::themes::init(cx);
        one_core::tab_container::init(cx);
        one_core::gpui_tokio::init(cx);
        one_ui::init(cx, Default::default());
        crate::auth::init(cx);
        cx.set_global(GlobalStorageState {
            storage: storage.clone(),
        });
        cx.set_global(AppSettings {
            app_layout: AppLayout::GlobalNavigation,
            connection_sidebar_expanded: true,
            ..AppSettings::default()
        });
    });

    let (window, app) = cx.update(|cx| {
        let window_bounds = Bounds::centered(None, size, cx);
        let mut app = None;
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(window_bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let entity = cx.new(|cx| {
                        cx.set_global(GlobalStorageState {
                            storage: storage.clone(),
                        });
                        NavopApp::new(window, cx)
                    });
                    app = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                },
            )
            .expect("打开真实 Navop 外壳");
        (window, app.expect("外壳 Entity"))
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    (visual, app)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} 必须在真实窗口中渲染"));
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "{selector} 必须可点击: {bounds:?}"
    );
    let point = bounds.center();
    cx.simulate_mouse_down(point, MouseButton::Left, Default::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Default::default());
    cx.run_until_parked();
}

fn set_layout(cx: &mut VisualTestContext, app: &Entity<NavopApp>, layout: AppLayout) {
    cx.update(|_, cx| {
        AppSettings::global_mut(cx).app_layout = layout;
        app.update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();
}

#[gpui::test]
fn switching_layout_preserves_connection_tree_and_home_navigation(cx: &mut TestAppContext) {
    let (mut cx, app) = open_shell(cx, gpui::size(px(1440.), px(900.)));
    let sidebar_id = cx.update(|_, cx| app.read(cx).connection_sidebar.entity_id());
    assert_eq!(
        cx.debug_bounds("persistent-connection-resource")
            .expect("全局连接树")
            .size
            .width,
        px(260.)
    );
    let rail = cx
        .debug_bounds("application-navigation")
        .expect("全局图标导航");
    assert_eq!(rail.size.width, px(52.));
    let rail_content = cx
        .debug_bounds("application-navigation-content")
        .expect("全局图标导航内容");
    let inset = cx
        .debug_bounds("macos-navigation-traffic-light-inset")
        .expect("macOS 红绿灯避让区");
    assert_eq!(inset.origin.x, rail.origin.x);
    assert_eq!(inset.origin.y, rail.origin.y);
    assert_eq!(inset.size.width, px(52.));
    assert_eq!(rail_content.origin.y, inset.bottom());

    set_layout(&mut cx, &app, AppLayout::HomeNavigation);
    let menu = cx
        .debug_bounds("application-navigation")
        .expect("主页应用菜单");
    assert_eq!(menu.size.width, px(184.));
    assert!(
        cx.debug_bounds("persistent-connection-resource").is_none(),
        "普通主页不能重复画外壳连接树"
    );

    set_layout(&mut cx, &app, AppLayout::GlobalNavigation);
    assert_eq!(
        cx.update(|_, cx| app.read(cx).connection_sidebar.entity_id()),
        sidebar_id
    );
    assert!(cx.debug_bounds("persistent-connection-resource").is_some());
}

#[gpui::test]
fn collapsed_connection_sidebar_can_be_reopened_with_the_mouse(cx: &mut TestAppContext) {
    let (mut cx, app) = open_shell(cx, gpui::size(px(1440.), px(900.)));
    click(&mut cx, "navigation-sidebar-toggle");
    assert!(
        cx.debug_bounds("persistent-connection-resource").is_none(),
        "收起后连接树应隐藏"
    );
    assert!(
        cx.debug_bounds("navigation-sidebar-toggle").is_some(),
        "收起后按钮必须保留"
    );
    assert!(!cx.update(|_, cx| app.read(cx).connection_sidebar.read(cx).is_expanded()));

    click(&mut cx, "navigation-sidebar-toggle");
    assert!(
        cx.debug_bounds("persistent-connection-resource").is_some(),
        "同一按钮必须能重新展开连接树"
    );
    assert!(cx.update(|_, cx| app.read(cx).connection_sidebar.read(cx).is_expanded()));
}

#[gpui::test]
fn global_home_and_connection_management_do_not_render_two_connection_trees(
    cx: &mut TestAppContext,
) {
    let (mut cx, app) = open_shell(cx, gpui::size(px(1440.), px(900.)));
    click(&mut cx, "home-manage-all-connections");
    let managing = cx.update(|_, cx| {
        app.read(cx)
            .connection_sidebar
            .read(cx)
            .home_page
            .read(cx)
            .navigation_connection_manager_open
    });
    assert!(managing, "真实鼠标点击必须打开管理页，而不是只改变命中区");
    assert!(
        cx.debug_bounds("home-navigation-connection-manager")
            .is_some()
    );
    assert!(
        cx.debug_bounds("persistent-connection-resource").is_none(),
        "中央全集管理打开时不重复显示左侧连接树"
    );
    assert!(!cx.update(|_, cx| app.read(cx).connection_sidebar.read(cx).is_expanded()));
    click(&mut cx, "navigation-sidebar-toggle");
    assert!(
        cx.update(|_, cx| app.read(cx).connection_sidebar.read(cx).is_expanded()),
        "切回主页后应恢复之前展开的连接树"
    );
    click(&mut cx, "home-navigation-manager-back");
    let restored = cx
        .debug_bounds("persistent-connection-resource")
        .expect("返回主页后原连接树恢复且偏好没有被管理页覆盖");
    assert_eq!(restored.size.width, px(260.));
}

#[gpui::test]
fn narrow_global_navigation_keeps_a_positive_workspace(cx: &mut TestAppContext) {
    let (mut cx, _app) = open_shell(cx, gpui::size(px(800.), px(600.)));
    let content = cx
        .debug_bounds("home-navigation-content")
        .expect("主页内容");
    assert!(
        content.size.width > px(0.) && content.size.height > px(0.),
        "窄窗口内容必须有实际尺寸: {content:?}"
    );
    assert!(cx.debug_bounds("application-navigation").is_some());
}
