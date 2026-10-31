//! 独立产品的 Tab 会话快照：存盘 + 重启后恢复。
//!
//! 快照直接用 `one_core::tab_container::TabContainerState`（与
//! `TabContainer::dump` 对称），文件放在本产品自己的配置目录下；恢复时按
//! `TabItemState::key` 把每条快照分派给领域注册的 [`TabRestorer`]。
//!
//! 产品侧只需要在 `Home::new` 末尾调用一次 [`install`]。

use domain_api::TabRestorer;
use gpui::{App, Entity, SharedString, Subscription, Window};
use one_core::storage::get_config_dir;
use one_core::tab_container::{
    TabContainer, TabContainerState, TabContentRegistry, TabItem, TabItemState, TabOpenMode,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// 快照文件名；沿用 `one_core::tab_persistence` 的历史命名。
const STATE_FILE: &str = "tab_state.json";
/// 快照格式版本；读到更高的版本说明是别的构建写的，直接放弃恢复。
const STATE_VERSION: usize = 1;
/// 标签变化后的落盘延迟：连续开关/拖拽多个标签时合并成一次写。
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// 每次排期自增；只有最新一次排期才真正落盘。
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
/// 最近一次成功写盘的状态，用于跳过无变化的写。
static LAST_SAVED: OnceLock<Mutex<Option<TabContainerState>>> = OnceLock::new();

/// 接入 Tab 会话：开始观察容器并（下一个窗口周期）恢复上次的标签。
///
/// `Home::new` 期间实体还没插入实体表，重建视图要用 `window.defer` 推到
/// 安全时点；调用方只需在 `Home::new` 末尾调用本函数一次。
pub fn install(tab_container: Entity<TabContainer>, window: &mut Window, cx: &mut App) {
    observe(tab_container.clone(), cx).detach();
    window.defer(cx, move |window, cx| {
        // 恢复失败的标签会被静默跳过，这里只需保证容器已经接手。
        let _ = restore_pending(&tab_container, window, cx);
    });
}

/// 监听容器变化；返回的订阅需要由调用方保活（[`install`] 里直接 detach）。
pub fn observe(tab_container: Entity<TabContainer>, cx: &mut App) -> Subscription {
    cx.observe(&tab_container, |tab_container, cx| {
        schedule_save(tab_container, cx);
    })
}

/// 读快照并重建标签，返回恢复出来的标签数量。
///
/// 快照缺失、损坏或版本更高时视作「没有上次会话」，容器保持原样。
///
/// 逐条用 `add_tab_with_mode(_, Background, ..)` 加回去而不是
/// `TabContainer::load`：公开的加法会顺手订阅内容事件，load 走的是不订阅的
/// 私有路径。全部加完后再激活快照里的当前标签。
pub fn restore_pending(
    tab_container: &Entity<TabContainer>,
    window: &mut Window,
    cx: &mut App,
) -> usize {
    let Some(state) = read_state() else {
        return 0;
    };
    let registry = content_registry(crate::registry().tab_restorers());
    let mut restored: Vec<SharedString> = Vec::new();
    let mut active_id = None;
    for (index, tab_state) in state.tabs.iter().enumerate() {
        let Some(content) = registry.build(tab_state, window, cx) else {
            continue;
        };
        if index == state.active_index {
            active_id = Some(tab_state.id.clone());
        }
        let item = TabItem::from_dyn(tab_state.id.clone(), tab_state.from.clone(), content)
            .with_metadata(tab_state.metadata.clone());
        restored.push(tab_state.id.clone());
        tab_container.update(cx, |container, cx| {
            container.add_tab_with_mode(item, TabOpenMode::Background, window, cx);
        });
    }
    if restored.is_empty() {
        return 0;
    }
    let target = active_id.unwrap_or_else(|| restored[0].clone());
    activate_restored(tab_container, &target, window, cx);
    restored.len()
}

/// 把恢复出来的标签设为当前标签，并保证它收到一次 `on_activate`。
fn activate_restored(
    tab_container: &Entity<TabContainer>,
    id: &SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(index) = tab_container
        .read(cx)
        .tabs()
        .iter()
        .position(|tab| tab.id() == *id)
    else {
        return;
    };
    // 目标恰好已经是当前索引时 set_active_index 不会调 on_activate，这里补一次。
    let needs_manual_activate = tab_container.read(cx).active_index() == index;
    tab_container.update(cx, |container, cx| {
        container.set_active_index(index, window, cx);
    });
    if !needs_manual_activate {
        return;
    }
    let content = tab_container
        .read(cx)
        .tabs()
        .get(index)
        .map(|tab| tab.content().clone());
    if let Some(content) = content {
        content.on_activate(window, cx);
    }
}

/// 立刻把当前标签状态写盘（退出前可主动调用）。
pub fn save(tab_container: &Entity<TabContainer>, cx: &App) -> anyhow::Result<()> {
    write_state(&tab_container.read(cx).dump(cx))
}

/// 防抖落盘：`SAVE_DELAY` 内的连续变化只写一次。
pub fn schedule_save(tab_container: Entity<TabContainer>, cx: &mut App) {
    let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DELAY).await;
        // 期间又有新的变化排期，交给更新的一次处理。
        if SAVE_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        cx.update(|cx| save_if_changed(&tab_container, cx));
    })
    .detach();
}

/// 状态没有变化就不写盘，避免只因为切换焦点而反复 IO。
fn save_if_changed(tab_container: &Entity<TabContainer>, cx: &App) {
    let state = tab_container.read(cx).dump(cx);
    let mut last_saved = LAST_SAVED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if last_saved.as_ref() == Some(&state) {
        return;
    }
    if write_state(&state).is_ok() {
        *last_saved = Some(state);
    }
}

/// 用各领域注册的恢复器装配 `TabContentRegistry`。
fn content_registry(restorers: &[Arc<dyn TabRestorer>]) -> TabContentRegistry {
    let mut registry = TabContentRegistry::new();
    for restorer in restorers {
        for key in restorer.content_keys() {
            let restorer = restorer.clone();
            registry.register_fn(
                key.into(),
                move |state: &TabItemState, window: &mut Window, cx: &mut App| {
                    restorer.restore(state, window, cx)
                },
            );
        }
    }
    registry
}

/// 快照文件路径；配置目录还不存在时由 [`write_state`] 负责创建。
pub fn state_path() -> anyhow::Result<PathBuf> {
    Ok(get_config_dir()?.join(STATE_FILE))
}

/// 读快照；文件缺失、损坏或版本不支持都按「没有上次会话」处理。
fn read_state() -> Option<TabContainerState> {
    read_state_from(&state_path().ok()?)
}

fn write_state(state: &TabContainerState) -> anyhow::Result<()> {
    let path = state_path()?;
    write_state_to(&path, state)
}

fn read_state_from(path: &Path) -> Option<TabContainerState> {
    parse_state(&std::fs::read_to_string(path).ok()?)
}

fn parse_state(text: &str) -> Option<TabContainerState> {
    let state: TabContainerState = serde_json::from_str(text).ok()?;
    // 更高版本可能含本产品不认识的字段语义，宁可不恢复也不误建标签。
    if state.version.is_some_and(|version| version > STATE_VERSION) {
        return None;
    }
    Some(state)
}

fn write_state_to(path: &Path, state: &TabContainerState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

#[cfg(test)]
mod tests;
