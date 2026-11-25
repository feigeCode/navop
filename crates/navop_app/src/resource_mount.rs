//! 纯资源挂载契约：只决定资源的宿主、可见状态与有效宽度，不访问业务实体。
//!
//! `desired_visible` 和持久宽度由调用方持有；窗口变窄产生的浮层状态不写回偏好。

use one_core::settings::AppLayout;

/// 并排布局优先保留的内容宽度。
pub const CONTENT_BUDGET: f32 = 560.0;
/// 内容宽度的绝对硬底线；不允许靠继续压缩内容来维持多栏并排。
pub const MIN_SIDE_BY_SIDE_CONTENT_WIDTH: f32 = 420.0;

const GLOBAL_NAVIGATION_WIDTH: f32 = 52.0;
const HOME_NAVIGATION_WIDTH: f32 = 184.0;

/// 当前页面声明的真实资源，不代表外壳应凭空创建资源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Connections,
    Notes,
    AiSessions,
    SettingsCategories,
    DbObjects,
    RedisKeys,
    MongoCollections,
    RemoteFiles,
    None,
}

/// 活动页面的资源及其工作区语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceDeclaration {
    pub resource: ResourceKind,
    /// 是否为 SSH 工作区；主页导航只在这里挂全集连接树。
    pub is_ssh_workspace: bool,
    /// 是否为本地或非 SSH 会话；即使声明连接资源也不套用 SSH 专属规则。
    pub is_local: bool,
}

impl ResourceDeclaration {
    pub const fn new(resource: ResourceKind) -> Self {
        Self {
            resource,
            is_ssh_workspace: false,
            is_local: false,
        }
    }

    pub const fn ssh_workspace() -> Self {
        Self {
            resource: ResourceKind::Connections,
            is_ssh_workspace: true,
            is_local: false,
        }
    }

    pub const fn local_terminal() -> Self {
        Self {
            resource: ResourceKind::Connections,
            is_ssh_workspace: false,
            is_local: true,
        }
    }
}

/// 资源在窗口中的挂载槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MountSlot {
    PrimarySidebar,
    WorkspaceSidebar,
    AuxRight,
    Overlay,
}

/// 实际可见状态；`Overlay` 是临时呈现，不覆盖用户的 `desired_visible`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MountVisibility {
    Visible,
    Collapsed,
    Overlay,
    Hidden,
}

/// 单个资源的持久呈现偏好。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MountPreferences {
    pub desired_visible: bool,
    /// 持久宽度；`None` 使用资源类型的默认宽度，非法值按宽度策略安全回退。
    pub width: Option<f32>,
}

impl Default for MountPreferences {
    fn default() -> Self {
        Self {
            desired_visible: true,
            width: None,
        }
    }
}

/// 一次纯计算的挂载结果；每次调用至多产生一个宿主计划。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResourceMountPlan {
    pub resource: ResourceKind,
    pub slot: MountSlot,
    pub visibility: MountVisibility,
    pub width: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WidthPolicy {
    min: f32,
    default: f32,
    max: f32,
}

impl WidthPolicy {
    fn sanitize(self, persisted: Option<f32>) -> f32 {
        match persisted {
            Some(width) if width.is_finite() => width.clamp(self.min, self.max),
            _ => self.default,
        }
    }
}

/// 外壳拖动和业务树共用同一个宽度边界，非法输入使用默认值。
pub fn sanitize_resource_width(resource: ResourceKind, width: f32) -> f32 {
    resource_width_policy(resource)
        .map(|policy| policy.sanitize(Some(width)))
        .unwrap_or(260.0)
}

fn resource_width_policy(resource: ResourceKind) -> Option<WidthPolicy> {
    let policy = match resource {
        ResourceKind::Connections
        | ResourceKind::DbObjects
        | ResourceKind::RedisKeys
        | ResourceKind::MongoCollections => WidthPolicy {
            min: 220.0,
            default: 260.0,
            max: 420.0,
        },
        ResourceKind::Notes | ResourceKind::AiSessions => WidthPolicy {
            min: 220.0,
            default: 260.0,
            max: 360.0,
        },
        ResourceKind::SettingsCategories => WidthPolicy {
            min: 184.0,
            default: 220.0,
            max: 300.0,
        },
        ResourceKind::RemoteFiles => WidthPolicy {
            min: 280.0,
            default: 320.0,
            max: 480.0,
        },
        ResourceKind::None => return None,
    };
    Some(policy)
}

fn base_slot(layout: AppLayout, active: &ResourceDeclaration) -> Option<MountSlot> {
    if active.resource == ResourceKind::RemoteFiles {
        // 远程文件是当前会话的辅助工具，永不替代主对象树。
        return Some(MountSlot::AuxRight);
    }
    if active.resource == ResourceKind::None {
        return None;
    }

    match layout {
        AppLayout::GlobalNavigation => (active.resource != ResourceKind::SettingsCategories)
            .then_some(MountSlot::PrimarySidebar),
        AppLayout::HomeNavigation => {
            // 展开布局只保留应用菜单。连接树与数据库类对象树由外壳托管：
            // 它们的表面颜色、拖动分隔线和折叠开关才能和连接面板保持一致；
            // 其余业务资源（笔记、AI、设置）仍留在各自页面内部。
            let hosted = match active.resource {
                ResourceKind::Connections => active.is_ssh_workspace && !active.is_local,
                ResourceKind::DbObjects
                | ResourceKind::RedisKeys
                | ResourceKind::MongoCollections => true,
                _ => false,
            };
            hosted.then_some(MountSlot::WorkspaceSidebar)
        }
    }
}

/// 纯函数：无 IO、无实体访问、无窗口副作用，也不修改调用方偏好。
pub fn plan_mount_with_navigation_width(
    layout: AppLayout,
    active: &ResourceDeclaration,
    window_width: f32,
    navigation_width: f32,
    prefs: &MountPreferences,
) -> ResourceMountPlan {
    let resource = active.resource;
    let fallback_slot = match layout {
        AppLayout::GlobalNavigation => MountSlot::PrimarySidebar,
        AppLayout::HomeNavigation => MountSlot::WorkspaceSidebar,
    };
    let requested_slot = base_slot(layout, active);
    let slot = requested_slot.unwrap_or(fallback_slot);
    let policy = resource_width_policy(resource);

    // 无资源和不应外移的业务资源都必须隐藏；即使窗口很窄也不能制造浮层空树。
    let Some(policy) = policy.filter(|_| requested_slot.is_some()) else {
        return ResourceMountPlan {
            resource,
            slot,
            visibility: MountVisibility::Hidden,
            width: None,
        };
    };

    // 用户主动折叠优先于空间求解，且跨布局、扩窗时保持。
    if !prefs.desired_visible {
        return ResourceMountPlan {
            resource,
            slot,
            visibility: MountVisibility::Collapsed,
            width: None,
        };
    }

    let persisted_width = policy.sanitize(prefs.width);
    let available_width = if window_width.is_finite() {
        (window_width - navigation_width).max(0.0)
    } else {
        0.0
    };
    let width_budget = available_width - CONTENT_BUDGET;

    // 先按 560 内容预算缩资源宽度；最小资源宽度仍放不下时转临时浮层。
    // 因而不存在内容低于 420 还继续并排的中间状态。
    let parallel_content_at_minimum = available_width - policy.min;
    let can_mount_side_by_side =
        width_budget >= policy.min && parallel_content_at_minimum >= MIN_SIDE_BY_SIDE_CONTENT_WIDTH;

    if can_mount_side_by_side {
        ResourceMountPlan {
            resource,
            slot,
            visibility: MountVisibility::Visible,
            width: Some(persisted_width.min(width_budget).max(policy.min)),
        }
    } else {
        ResourceMountPlan {
            resource,
            // 右侧辅助工具即使浮层化也保持 AuxRight 身份，避免替代主对象树。
            slot: if slot == MountSlot::AuxRight {
                MountSlot::AuxRight
            } else {
                MountSlot::Overlay
            },
            visibility: MountVisibility::Overlay,
            width: Some(persisted_width),
        }
    }
}

/// 默认兼容调用；窗口外壳应传入真实菜单宽度（包括折叠后的 58px）。
pub fn plan_mount(
    layout: AppLayout,
    active: &ResourceDeclaration,
    window_width: f32,
    prefs: &MountPreferences,
) -> ResourceMountPlan {
    plan_mount_with_navigation_width(
        layout,
        active,
        window_width,
        match layout {
            AppLayout::GlobalNavigation => GLOBAL_NAVIGATION_WIDTH,
            AppLayout::HomeNavigation => HOME_NAVIGATION_WIDTH,
        },
        prefs,
    )
}

/// GPUI 拖动回调给出相对手柄原点的偏移，不能直接当窗口横坐标。
pub fn drag_resource_width(initial_width: f32, cursor_offset_x: f32) -> f32 {
    (initial_width + cursor_offset_x).max(0.0)
}

/// 一次拖动的鼠标起点和面板初始宽度。状态由 NavopApp 持有，不能放在 render 闭包里。
pub type ResourceResizeDrag = (gpui::Point<gpui::Pixels>, gpui::Pixels);

/// 更新一次连续拖动；重渲染后仍使用同一个起点，收到 `None` 时清理会话。
pub fn update_resource_resize_drag(
    drag: &mut Option<ResourceResizeDrag>,
    position: Option<gpui::Point<gpui::Pixels>>,
    initial_width: gpui::Pixels,
) -> Option<gpui::Pixels> {
    let position = match position {
        Some(position) => position,
        None => {
            *drag = None;
            return None;
        }
    };
    let (origin, initial_width) = *drag.get_or_insert((position, initial_width));
    Some(gpui::px(drag_resource_width(
        f32::from(initial_width),
        f32::from(position.x - origin.x),
    )))
}

/// 稳定的资源持久键片段，不使用进程内实体标识。
pub const fn stable_resource_key(resource: ResourceKind) -> &'static str {
    match resource {
        ResourceKind::Connections => "connections",
        ResourceKind::Notes => "notes",
        ResourceKind::AiSessions => "ai_sessions",
        ResourceKind::SettingsCategories => "settings_categories",
        ResourceKind::DbObjects => "db_objects",
        ResourceKind::RedisKeys => "redis_keys",
        ResourceKind::MongoCollections => "mongo_collections",
        ResourceKind::RemoteFiles => "remote_files",
        ResourceKind::None => "none",
    }
}

/// 按布局与资源隔离宽度偏好，避免切换布局相互覆盖。
pub fn stable_width_key(layout: AppLayout, resource: ResourceKind) -> String {
    let layout = match layout {
        AppLayout::GlobalNavigation => "global_navigation",
        AppLayout::HomeNavigation => "home_navigation",
    };
    format!(
        "resource_mount.{layout}.{}.width",
        stable_resource_key(resource)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ShellResizeHost {
        width: gpui::Pixels,
        persisted: std::rc::Rc<std::cell::Cell<gpui::Pixels>>,
    }

    impl gpui::Render for ShellResizeHost {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{AppContext as _, InteractiveElement as _, ParentElement as _, Styled as _};
            let width = self.width;
            let persisted = self.persisted.clone();
            let host = cx.entity();
            let resize = one_ui::resize_handle::resize_handle::<
                one_ui::resize_handle::ResizePanel,
                Self,
            >("shell-resize-test", gpui::Axis::Horizontal)
            .inline()
            .on_drag(
                one_ui::resize_handle::ResizePanel,
                move |_, offset, _, cx| {
                    let next = gpui::px(drag_resource_width(f32::from(width), f32::from(offset.x)));
                    persisted.set(next);
                    host.update(cx, |host, cx| {
                        host.width = next;
                        cx.notify();
                    });
                    cx.new(|_| ShellResizeHost {
                        width: next,
                        persisted: persisted.clone(),
                    })
                },
            );
            gpui_component::h_flex()
                .items_stretch()
                .size_full()
                .child(
                    gpui::div()
                        .debug_selector(|| "shell-resize-panel".to_owned())
                        .w(width)
                        .h_full(),
                )
                .child(resize)
                .child(gpui::div().flex_1().min_w_0().h_full())
        }
    }

    #[gpui::test]
    fn the_shell_handle_actually_resizes_and_persists_the_panel(cx: &mut gpui::TestAppContext) {
        use gpui::{
            AppContext as _, Bounds, Context as _, Modifiers, MouseButton, Point,
            VisualTestContext, WindowBounds, WindowOptions, px, size,
        };
        cx.update(gpui_component::init);
        let (window, host) = cx.update(|cx| {
            let bounds = Bounds::centered(None, size(px(900.), px(600.)), cx);
            let mut host = None;
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    |window, cx| {
                        let entity = cx.new(|_| ShellResizeHost {
                            width: px(260.),
                            persisted: std::rc::Rc::new(std::cell::Cell::new(px(260.))),
                        });
                        host = Some(entity.clone());
                        cx.new(|cx| gpui_component::Root::new(entity, window, cx))
                    },
                )
                .expect("open safe resize fixture");
            (window, host.expect("fixture host"))
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let panel = cx.debug_bounds("shell-resize-panel").expect("panel");
        let band = cx
            .debug_bounds("resize-handle-band-horizontal-none")
            .expect("inline handle");
        assert_eq!(band.left(), panel.right());
        assert_eq!(band.size.height, panel.size.height);
        let start = band.center();
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            Point {
                x: start.x + px(40.),
                y: start.y,
            },
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            Point {
                x: start.x + px(40.),
                y: start.y,
            },
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.run_until_parked();
        let width = cx.read(|cx| host.read(cx).persisted.get());
        assert_eq!(width, px(260. + 40. + 6.5));
        let resized = cx
            .debug_bounds("shell-resize-panel")
            .expect("resized panel");
        assert_eq!(resized.size.width, width, "拖动后原宿主必须实际重排");
    }

    fn declaration(resource: ResourceKind) -> ResourceDeclaration {
        ResourceDeclaration::new(resource)
    }

    fn ssh() -> ResourceDeclaration {
        ResourceDeclaration::ssh_workspace()
    }

    fn local_terminal() -> ResourceDeclaration {
        ResourceDeclaration::local_terminal()
    }

    fn prefs() -> MountPreferences {
        MountPreferences::default()
    }

    fn plan(
        layout: AppLayout,
        active: &ResourceDeclaration,
        window_width: f32,
        prefs: &MountPreferences,
    ) -> ResourceMountPlan {
        plan_mount(layout, active, window_width, prefs)
    }

    fn assert_mount(
        plan: &ResourceMountPlan,
        resource: ResourceKind,
        slot: MountSlot,
        visibility: MountVisibility,
    ) {
        assert_eq!(resource, plan.resource);
        assert_eq!(slot, plan.slot);
        assert_eq!(visibility, plan.visibility);
    }

    #[test]
    fn 全局导航按页面挂载唯一资源() {
        let cases = [
            (
                declaration(ResourceKind::Connections),
                ResourceKind::Connections,
            ),
            (ssh(), ResourceKind::Connections),
            (
                declaration(ResourceKind::DbObjects),
                ResourceKind::DbObjects,
            ),
            (
                declaration(ResourceKind::RedisKeys),
                ResourceKind::RedisKeys,
            ),
            (
                declaration(ResourceKind::MongoCollections),
                ResourceKind::MongoCollections,
            ),
            (declaration(ResourceKind::Notes), ResourceKind::Notes),
            (
                declaration(ResourceKind::AiSessions),
                ResourceKind::AiSessions,
            ),
        ];

        for (active, resource) in cases {
            let result = plan(AppLayout::GlobalNavigation, &active, 1440.0, &prefs());
            assert_mount(
                &result,
                resource,
                MountSlot::PrimarySidebar,
                MountVisibility::Visible,
            );
            assert!(result.width.is_some());
        }
    }

    #[test]
    fn 设置分类始终留在设置页内部() {
        for layout in [AppLayout::GlobalNavigation, AppLayout::HomeNavigation] {
            let result = plan(
                layout,
                &declaration(ResourceKind::SettingsCategories),
                1440.0,
                &prefs(),
            );
            assert_eq!(MountVisibility::Hidden, result.visibility);
            assert_eq!(None, result.width);
        }
    }

    #[test]
    fn 主页导航只保留应用菜单并在_ssh_工作区挂载连接树() {
        let ssh_plan = plan(AppLayout::HomeNavigation, &ssh(), 1440.0, &prefs());
        assert_mount(
            &ssh_plan,
            ResourceKind::Connections,
            MountSlot::WorkspaceSidebar,
            MountVisibility::Visible,
        );

        let home_plan = plan(
            AppLayout::HomeNavigation,
            &declaration(ResourceKind::Connections),
            1440.0,
            &prefs(),
        );
        assert_mount(
            &home_plan,
            ResourceKind::Connections,
            MountSlot::WorkspaceSidebar,
            MountVisibility::Hidden,
        );
        assert_eq!(None, home_plan.width);

        let non_ssh_plan = plan(
            AppLayout::HomeNavigation,
            &ResourceDeclaration {
                resource: ResourceKind::Connections,
                is_ssh_workspace: false,
                is_local: false,
            },
            1440.0,
            &prefs(),
        );
        assert_eq!(MountVisibility::Hidden, non_ssh_plan.visibility);
    }

    #[test]
    fn 主页导航本地终端不显示_ssh_全集树() {
        let result = plan(
            AppLayout::HomeNavigation,
            &local_terminal(),
            1440.0,
            &prefs(),
        );
        assert_mount(
            &result,
            ResourceKind::Connections,
            MountSlot::WorkspaceSidebar,
            MountVisibility::Hidden,
        );

        let inconsistent_declaration = ResourceDeclaration {
            is_ssh_workspace: true,
            ..local_terminal()
        };
        let result = plan(
            AppLayout::HomeNavigation,
            &inconsistent_declaration,
            1440.0,
            &prefs(),
        );
        assert_eq!(MountVisibility::Hidden, result.visibility);
    }

    #[test]
    fn 主页导航业务资源保留在功能内部() {
        for resource in [
            ResourceKind::Notes,
            ResourceKind::AiSessions,
            ResourceKind::SettingsCategories,
        ] {
            let result = plan(
                AppLayout::HomeNavigation,
                &declaration(resource),
                1440.0,
                &prefs(),
            );
            assert_mount(
                &result,
                resource,
                MountSlot::WorkspaceSidebar,
                MountVisibility::Hidden,
            );
            assert_eq!(None, result.width);
        }
    }

    #[test]
    fn 主页导航数据库类对象树交给外壳托管() {
        for (resource, active) in [
            (
                ResourceKind::DbObjects,
                declaration(ResourceKind::DbObjects),
            ),
            (
                ResourceKind::RedisKeys,
                declaration(ResourceKind::RedisKeys),
            ),
            (
                ResourceKind::MongoCollections,
                declaration(ResourceKind::MongoCollections),
            ),
        ] {
            let result = plan(AppLayout::HomeNavigation, &active, 1440.0, &prefs());
            assert_mount(
                &result,
                resource,
                MountSlot::WorkspaceSidebar,
                MountVisibility::Visible,
            );
            assert!(result.width.is_some());

            // 折叠与经典图标布局的空闲窗口同样由外壳决定显隐，业务页不再自己画树。
            let collapsed = plan(
                AppLayout::HomeNavigation,
                &active,
                1440.0,
                &MountPreferences {
                    desired_visible: false,
                    width: None,
                },
            );
            assert_eq!(MountVisibility::Collapsed, collapsed.visibility);
        }
    }

    #[test]
    fn 窄窗口使用真实折叠菜单宽度计算() {
        let result = plan_mount_with_navigation_width(
            AppLayout::HomeNavigation,
            &ssh(),
            900.0,
            58.0,
            &prefs(),
        );
        assert_eq!(MountVisibility::Visible, result.visibility);
        assert_eq!(900.0 - 58.0 - 260.0, 582.0);
    }

    #[test]
    fn 远程文件始终使用右侧辅助槽() {
        for layout in [AppLayout::GlobalNavigation, AppLayout::HomeNavigation] {
            let result = plan(
                layout,
                &declaration(ResourceKind::RemoteFiles),
                1440.0,
                &prefs(),
            );
            assert_mount(
                &result,
                ResourceKind::RemoteFiles,
                MountSlot::AuxRight,
                MountVisibility::Visible,
            );
        }

        let narrow = plan(
            AppLayout::GlobalNavigation,
            &declaration(ResourceKind::RemoteFiles),
            800.0,
            &prefs(),
        );
        assert_mount(
            &narrow,
            ResourceKind::RemoteFiles,
            MountSlot::AuxRight,
            MountVisibility::Overlay,
        );
    }

    #[test]
    fn 无资源始终隐藏且不产生宽度() {
        for layout in [AppLayout::GlobalNavigation, AppLayout::HomeNavigation] {
            for desired_visible in [false, true] {
                let result = plan(
                    layout,
                    &declaration(ResourceKind::None),
                    1440.0,
                    &MountPreferences {
                        desired_visible,
                        width: Some(400.0),
                    },
                );
                assert_eq!(ResourceKind::None, result.resource);
                assert_eq!(MountVisibility::Hidden, result.visibility);
                assert_eq!(None, result.width);
            }
        }
    }

    #[test]
    fn 切换布局保持_ssh_资源与期望可见状态() {
        let preferences = MountPreferences {
            desired_visible: true,
            width: Some(288.0),
        };
        let global = plan(AppLayout::GlobalNavigation, &ssh(), 1440.0, &preferences);
        let home = plan(AppLayout::HomeNavigation, &ssh(), 1440.0, &preferences);

        assert_eq!(global.resource, home.resource);
        assert_eq!(global.visibility, home.visibility);
        assert_eq!(global.width, home.width);
        assert_eq!(MountSlot::PrimarySidebar, global.slot);
        assert_eq!(MountSlot::WorkspaceSidebar, home.slot);
    }

    #[test]
    fn 用户折叠偏好在切换布局和扩窗时不被重置() {
        let preferences = MountPreferences {
            desired_visible: false,
            width: Some(288.0),
        };
        for layout in [AppLayout::GlobalNavigation, AppLayout::HomeNavigation] {
            for width in [640.0, 1440.0, 2400.0] {
                let result = plan(layout, &ssh(), width, &preferences);
                assert_eq!(MountVisibility::Collapsed, result.visibility);
                assert_eq!(None, result.width);
            }
        }
    }

    #[test]
    fn 八百像素宽窗口转为临时浮层() {
        for layout in [AppLayout::GlobalNavigation, AppLayout::HomeNavigation] {
            let result = plan(layout, &ssh(), 800.0, &prefs());
            assert_eq!(MountVisibility::Overlay, result.visibility);
            assert_eq!(MountSlot::Overlay, result.slot);
            assert_eq!(Some(260.0), result.width);
        }
    }

    #[test]
    fn 空间不足先缩到最小宽度再转浮层且扩窗恢复() {
        let preferences = MountPreferences {
            desired_visible: true,
            width: Some(400.0),
        };
        let shrunk = plan(AppLayout::GlobalNavigation, &ssh(), 920.0, &preferences);
        assert_eq!(MountVisibility::Visible, shrunk.visibility);
        assert_eq!(Some(308.0), shrunk.width);

        let minimum = plan(AppLayout::GlobalNavigation, &ssh(), 832.0, &preferences);
        assert_eq!(MountVisibility::Visible, minimum.visibility);
        assert_eq!(Some(220.0), minimum.width);

        let overlay = plan(AppLayout::GlobalNavigation, &ssh(), 831.0, &preferences);
        assert_eq!(MountVisibility::Overlay, overlay.visibility);
        assert_eq!(Some(400.0), overlay.width);

        let restored = plan(AppLayout::GlobalNavigation, &ssh(), 1440.0, &preferences);
        assert_eq!(MountVisibility::Visible, restored.visibility);
        assert_eq!(Some(400.0), restored.width);
        assert_eq!(preferences.width, Some(400.0));
    }

    #[test]
    fn 任何并排结果都保留内容预算和硬底线() {
        for layout in [AppLayout::GlobalNavigation, AppLayout::HomeNavigation] {
            let navigation_width = match layout {
                AppLayout::GlobalNavigation => 52.0,
                AppLayout::HomeNavigation => 184.0,
            };
            for window_width in [600.0, 800.0, 900.0, 1024.0, 1280.0, 1440.0] {
                let result = plan(layout, &ssh(), window_width, &prefs());
                if result.visibility == MountVisibility::Visible {
                    let content = window_width - navigation_width - result.width.unwrap();
                    assert!(content >= CONTENT_BUDGET);
                    assert!(content >= MIN_SIDE_BY_SIDE_CONTENT_WIDTH);
                }
            }
        }
    }

    #[test]
    fn 各资源使用独立默认宽度() {
        let cases = [
            (ResourceKind::Connections, 260.0),
            (ResourceKind::Notes, 260.0),
            (ResourceKind::AiSessions, 260.0),
            (ResourceKind::DbObjects, 260.0),
            (ResourceKind::RedisKeys, 260.0),
            (ResourceKind::MongoCollections, 260.0),
            (ResourceKind::RemoteFiles, 320.0),
        ];
        for (resource, expected) in cases {
            let result = plan(
                AppLayout::GlobalNavigation,
                &declaration(resource),
                2400.0,
                &prefs(),
            );
            assert_eq!(Some(expected), result.width, "资源：{resource:?}");
        }
    }

    #[test]
    fn 持久宽度按资源边界截断() {
        for resource in [
            ResourceKind::Connections,
            ResourceKind::Notes,
            ResourceKind::AiSessions,
            ResourceKind::DbObjects,
            ResourceKind::RedisKeys,
            ResourceKind::MongoCollections,
            ResourceKind::RemoteFiles,
        ] {
            let policy = resource_width_policy(resource).unwrap();
            for (candidate, expected) in [
                (Some(1.0), policy.min),
                (Some(10_000.0), policy.max),
                (Some(f32::NAN), policy.default),
            ] {
                let result = plan(
                    AppLayout::GlobalNavigation,
                    &declaration(resource),
                    4000.0,
                    &MountPreferences {
                        desired_visible: true,
                        width: candidate,
                    },
                );
                assert_eq!(Some(expected), result.width, "资源：{resource:?}");
            }
        }
    }

    #[test]
    fn 默认偏好使用资源默认宽度() {
        let preferences = MountPreferences::default();
        assert!(preferences.desired_visible);
        assert_eq!(None, preferences.width);
        assert_eq!(
            260.0,
            resource_width_policy(ResourceKind::Connections)
                .unwrap()
                .default
        );
        assert_eq!(
            220.0,
            resource_width_policy(ResourceKind::Connections)
                .unwrap()
                .min
        );
        assert_eq!(
            420.0,
            resource_width_policy(ResourceKind::Connections)
                .unwrap()
                .max
        );
        assert!(resource_width_policy(ResourceKind::None).is_none());
    }

    #[test]
    fn 拖动宽度使用手柄偏移而不是窗口横坐标() {
        assert_eq!(drag_resource_width(260.0, 40.0), 300.0);
        assert_eq!(drag_resource_width(260.0, -40.0), 220.0);
        assert_eq!(drag_resource_width(220.0, -300.0), 0.0);
    }

    #[test]
    fn 连续拖动在多次重渲染后仍使用同一起点() {
        use gpui::{point, px};

        let mut drag = None;
        assert_eq!(
            update_resource_resize_drag(&mut drag, Some(point(px(100.), px(50.))), px(260.)),
            Some(px(260.))
        );
        // 每次回调都带新的初始宽度，模拟 resize 后重新 render；会话不得被重置。
        for (x, width) in [(120., 280.), (160., 320.), (140., 300.)] {
            assert_eq!(
                update_resource_resize_drag(&mut drag, Some(point(px(x), px(50.))), px(width)),
                Some(px(width))
            );
        }
        assert_eq!(update_resource_resize_drag(&mut drag, None, px(300.)), None);
        assert_eq!(
            update_resource_resize_drag(&mut drag, Some(point(px(500.), px(50.))), px(300.)),
            Some(px(300.))
        );
    }

    #[test]
    fn 稳定持久键包含布局和资源且不依赖实体() {
        let keys = [
            ResourceKind::Connections,
            ResourceKind::Notes,
            ResourceKind::AiSessions,
            ResourceKind::SettingsCategories,
            ResourceKind::DbObjects,
            ResourceKind::RedisKeys,
            ResourceKind::MongoCollections,
            ResourceKind::RemoteFiles,
            ResourceKind::None,
        ]
        .map(stable_resource_key);
        for (index, key) in keys.iter().enumerate() {
            assert!(!key.is_empty());
            assert!(!keys[..index].contains(key));
        }

        assert_eq!(
            "connections",
            stable_resource_key(ResourceKind::Connections)
        );
        assert_ne!(
            stable_width_key(AppLayout::GlobalNavigation, ResourceKind::Connections),
            stable_width_key(AppLayout::HomeNavigation, ResourceKind::Connections)
        );
        assert_eq!(
            stable_width_key(AppLayout::GlobalNavigation, ResourceKind::Connections),
            stable_width_key(AppLayout::GlobalNavigation, ResourceKind::Connections)
        );
    }
}
