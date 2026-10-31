//! 平台托盘后端：只在这里碰 `tray_icon`，所有回调都只做「入队」。

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod platform {
    use super::super::menu::{
        MENU_ID_NEW_CONNECTION, MENU_ID_OPEN_SETTINGS, MENU_ID_QUIT, MENU_ID_SHOW,
        command_for_menu_id, session_menu_entries,
    };
    use super::super::{config, dispatch};
    use std::cell::RefCell;
    use tray_icon::TrayIconBuilder;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};

    /// 托盘图标边长。源资源是 1024×1024 的应用图标，直接交给平台会被按菜单栏/通知
    /// 区域高度做一次质量不可控的缩放，所以这里先缩到常见托盘尺寸。
    pub(crate) const TRAY_ICON_EDGE: u32 = 32;

    thread_local! {
        /// 托盘句柄必须留在**创建它的线程**上：`TrayIcon` 内部是
        /// `Rc<RefCell<..>>`（全平台 `!Send`），而且 drop 掉最后一个实例图标会
        /// 立刻从托盘区消失。这就是「图标闪一下就没了」的全部原因。
        static TRAY_ICON: RefCell<Option<tray_icon::TrayIcon>> = const { RefCell::new(None) };
    }

    pub(crate) fn install() -> anyhow::Result<()> {
        if TRAY_ICON.with(|slot| slot.borrow().is_some()) {
            return Ok(());
        }

        // 与 `receiver()` 互斥：一旦设置 handler，channel 就永远收不到事件。
        MenuEvent::set_event_handler(Some(handle_menu_event));

        let config = config();
        let (rgba, width, height) = decode_tray_icon(config.icon_png)?;
        let icon = tray_icon::Icon::from_rgba(rgba, width, height)?;
        let menu = build_menu(&[])?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip(config.tooltip.clone())
            // 左键与右键都打开菜单，和 macOS/Windows 常驻托盘应用的惯例一致。
            // 主窗口通过菜单里的「显示 …」恢复，不再占用左键单击。
            .with_menu_on_left_click(true)
            .with_menu_on_right_click(true)
            .build()?;

        TRAY_ICON.with(|slot| *slot.borrow_mut() = Some(tray));
        Ok(())
    }

    /// PNG 字节 → `(RGBA, width, height)`。
    ///
    /// 纯函数，单测覆盖通道数与尺寸；解码失败视为托盘初始化失败——宁可不显示托盘，
    /// 也不做一个没有图标、用户找不到的隐形入口。
    pub(crate) fn decode_tray_icon(png: &[u8]) -> anyhow::Result<(Vec<u8>, u32, u32)> {
        use anyhow::Context as _;

        let decoded = image::load_from_memory(png)
            .context("托盘图标 PNG 解码失败")?
            .into_rgba8();
        let (width, height) = decoded.dimensions();

        let scaled = if width > TRAY_ICON_EDGE || height > TRAY_ICON_EDGE {
            let ratio = f64::from(TRAY_ICON_EDGE) / f64::from(width.max(height));
            let target_width = ((f64::from(width) * ratio).round() as u32).max(1);
            let target_height = ((f64::from(height) * ratio).round() as u32).max(1);
            image::imageops::resize(
                &decoded,
                target_width,
                target_height,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            decoded
        };

        let scaled = match crate::integration::tray::config().badge {
            Some(badge) => add_badge(scaled, badge),
            None => scaled,
        };
        let (width, height) = scaled.dimensions();
        Ok((scaled.into_raw(), width, height))
    }

    /// 在图标右下角画一个带白边的实心圆角标，作为产品区分标记。
    fn add_badge(mut icon: image::RgbaImage, badge: [u8; 4]) -> image::RgbaImage {
        let (width, height) = icon.dimensions();
        let radius = (width.min(height) / 4).max(3);
        let center_x = width.saturating_sub(radius);
        let center_y = height.saturating_sub(radius);
        let ring = radius.saturating_add(1);
        for y in 0..height {
            for x in 0..width {
                let dx = i64::from(x) - i64::from(center_x);
                let dy = i64::from(y) - i64::from(center_y);
                let distance = (dx * dx + dy * dy) as f64;
                if distance <= f64::from(ring * ring) {
                    icon.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
                }
                if distance <= f64::from(radius * radius) {
                    icon.put_pixel(x, y, image::Rgba(badge));
                }
            }
        }
        icon
    }

    /// 会话段 → 分隔线 →（快捷动作段）→ 显示 / 退出。会话段为空时不留下孤立分隔线。
    fn build_menu(sessions: &[(String, String)]) -> anyhow::Result<Menu> {
        let config = config();
        let menu = Menu::new();

        let entries = session_menu_entries(sessions);
        for (id, label) in &entries {
            menu.append(&MenuItem::with_id(id.clone(), label.clone(), true, None))?;
        }
        if !entries.is_empty() {
            menu.append(&PredefinedMenuItem::separator())?;
        }

        if config.show_quick_actions {
            let new_connection = MenuItem::with_id(MENU_ID_NEW_CONNECTION, "新建连接", true, None);
            let open_settings = MenuItem::with_id(MENU_ID_OPEN_SETTINGS, "打开设置", true, None);
            menu.append(&new_connection)?;
            menu.append(&open_settings)?;
            menu.append(&PredefinedMenuItem::separator())?;
        }

        let show = MenuItem::with_id(
            MENU_ID_SHOW,
            format!("显示 {}", config.display_name),
            true,
            None,
        );
        let quit = MenuItem::with_id(
            MENU_ID_QUIT,
            format!("退出 {}", config.display_name),
            true,
            None,
        );
        menu.append(&show)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;
        Ok(menu)
    }

    /// 原地换菜单。`TrayIcon` 是 `!Send`，这里只在创建它的线程上被调用。
    pub(crate) fn sync_sessions(sessions: &[(String, String)]) {
        let menu = match build_menu(sessions) {
            Ok(menu) => menu,
            Err(error) => {
                tracing::warn!(%error, "重建托盘会话菜单失败");
                return;
            }
        };

        TRAY_ICON.with(|slot| {
            if let Some(tray) = slot.borrow().as_ref() {
                tray.set_menu(Some(Box::new(menu)));
            }
        });
    }

    fn handle_menu_event(event: MenuEvent) {
        if let Some(command) = command_for_menu_id(event.id().as_ref()) {
            dispatch(command);
        }
    }

    /// 释放托盘句柄。只在退出漏斗里调用，理由见 [`super::super::shutdown`]。
    ///
    /// 先把句柄 `take` 出闭包再 drop：`TrayIcon::drop` 会阻塞（join 菜单监听线程 +
    /// 等 ksni 的 D-Bus service 收摊），不能让它发生在 `TRAY_ICON.with` 的借用里。
    pub(crate) fn shutdown() {
        let tray = TRAY_ICON.with(|slot| slot.borrow_mut().take());
        drop(tray);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    pub(crate) fn install() -> anyhow::Result<()> {
        anyhow::bail!("当前平台未实现系统托盘")
    }

    pub(crate) fn sync_sessions(_sessions: &[(String, String)]) {}

    pub(crate) fn shutdown() {}
}

pub(super) use platform::{install, shutdown, sync_sessions};

#[cfg(all(
    test,
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
pub(super) use platform::{TRAY_ICON_EDGE, decode_tray_icon};
