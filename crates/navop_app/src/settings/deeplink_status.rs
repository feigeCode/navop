//! 链接协议「系统有没有把 `ssh://` / `sftp://` / `jms://` 交给 Navop」的异步探测。
//!
//! 答案只能去系统里查：macOS 读 LaunchServices，Windows 读注册表，Linux 走
//! `xdg-mime`。后两者要起子进程，所以探测一律放在后台线程，UI 线程只读一份
//! 快照；探测完成后再刷新窗口，设置页据此显示「已接管 / 被别的应用占用 / 未设置」。

use std::collections::BTreeMap;

use gpui::{App, AppContext as _, AsyncApp, Global};
use one_core::settings::AppSettings;

/// 设置页需要展示接管状态的 scheme，顺序与界面上出现顺序一致。
pub const SCHEMES: [&str; 3] = ["ssh", "sftp", "jms"];

/// 某个 scheme 当前在系统里的归属。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemeTakeover {
    /// 系统默认处理程序就是 Navop。
    Taken,
    /// 系统默认处理程序是别的应用，值为它的标识（bundle id / 可执行文件 / desktop 名）。
    Other(String),
    /// 系统里没有应用认领这个 scheme。
    Unset,
    /// 探测失败（子系统缺失、权限不足等），值为错误信息。
    Unknown(String),
    /// 当前不是安装形态（开发态 / 便携版），不写系统关联，也就无所谓「接管」。
    NotApplicable,
}

struct GlobalDeeplinkTakeover {
    /// 是否已有一次探测在跑；避免设置页反复渲染时重复起子进程。
    probing: bool,
    entries: BTreeMap<&'static str, SchemeTakeover>,
}

impl Global for GlobalDeeplinkTakeover {}

impl GlobalDeeplinkTakeover {
    fn new() -> Self {
        Self {
            probing: false,
            entries: BTreeMap::new(),
        }
    }
}

/// 读取快照里的接管状态；`None` 表示还在探测（或还没开始探测）。
pub fn takeover(cx: &App, scheme: &str) -> Option<SchemeTakeover> {
    cx.try_global::<GlobalDeeplinkTakeover>()
        .and_then(|snapshot| snapshot.entries.get(scheme).cloned())
}

/// 是否已有探测在后台跑。
pub fn is_probing(cx: &App) -> bool {
    cx.try_global::<GlobalDeeplinkTakeover>()
        .is_none_or(|snapshot| snapshot.probing)
}

/// 设置页渲染时调用：没有快照就延后一帧起一次探测，绝不阻塞当前帧。
pub fn ensure_probed(cx: &mut App) {
    let has_snapshot = cx
        .try_global::<GlobalDeeplinkTakeover>()
        .is_some_and(|snapshot| snapshot.probing || !snapshot.entries.is_empty());
    if has_snapshot {
        return;
    }
    cx.defer(refresh);
}

/// 重新探测，且仅当设置页已经展示过状态（快照存在）时才做。
///
/// 用于「注册写完之后」这条路径：启动阶段的注册不该为一个没打开的设置页去查
/// 注册表、起 `xdg-mime`。
pub fn refresh_if_active(cx: &mut App) {
    let active = cx
        .try_global::<GlobalDeeplinkTakeover>()
        .is_some_and(|snapshot| snapshot.probing || !snapshot.entries.is_empty());
    if active {
        refresh(cx);
    }
}

/// 重新探测（开关切换、注册写完之后调用）。已有探测在跑时直接忽略。
pub fn refresh(cx: &mut App) {
    // 先建快照：`is_probing` 在全局缺失时返回 true（还没探测过），不能靠它判重。
    if !cx.has_global::<GlobalDeeplinkTakeover>() {
        cx.set_global(GlobalDeeplinkTakeover::new());
    }
    if is_probing(cx) {
        return;
    }
    cx.set_global(GlobalDeeplinkTakeover {
        probing: true,
        entries: cx
            .try_global::<GlobalDeeplinkTakeover>()
            .map(|snapshot| snapshot.entries.clone())
            .unwrap_or_default(),
    });

    let schemes: Vec<&str> = SCHEMES.to_vec();
    let task = cx.background_spawn(async move {
        app_kit::file_association::probe_url_scheme_handlers(&schemes)
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let probes = task.await;
        cx.update(|cx| apply(cx, probes));
    })
    .detach();
}

fn apply(cx: &mut App, probes: Vec<app_kit::file_association::UrlSchemeHandlerProbe>) {
    let mut entries = BTreeMap::new();
    for probe in probes {
        // 只收认识的 scheme：探测结果按字符串回传，这里映射回静态字面量，
        // 免得快照 key 变成泄漏出去的临时字符串。
        let Some(scheme) = SCHEMES
            .iter()
            .copied()
            .find(|scheme| scheme.eq_ignore_ascii_case(&probe.scheme))
        else {
            continue;
        };
        let takeover = if let Some(error) = probe.error {
            SchemeTakeover::Unknown(error)
        } else if probe.is_claimed_by_self() {
            SchemeTakeover::Taken
        } else if probe.expected.is_none() {
            SchemeTakeover::NotApplicable
        } else if let Some(handler) = probe.handler {
            SchemeTakeover::Other(handler)
        } else {
            SchemeTakeover::Unset
        };
        entries.insert(scheme, takeover);
    }
    cx.set_global(GlobalDeeplinkTakeover {
        probing: false,
        entries,
    });
    cx.refresh_windows();
}

/// scheme 在设置页里的显示名（`jms://` 单列出来，避免和公共 scheme 混淆）。
pub fn scheme_label(scheme: &str) -> String {
    format!("{scheme}://")
}

/// 该 scheme 的开关是否打开（关闭时界面只提示「未接管」，不展示系统归属）。
pub fn switch_enabled(cx: &App, scheme: &str) -> bool {
    AppSettings::global(cx).deeplink_scheme_enabled(scheme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_cover_every_deeplink_the_product_declares() {
        for declared in domain_api::AppIdentity::FULL.url_schemes {
            assert!(
                SCHEMES.contains(declared),
                "scheme {declared} 有设置项却没有接管状态探测"
            );
        }
    }

    #[test]
    fn scheme_labels_are_rendered_as_url_prefixes() {
        assert_eq!("jms://", scheme_label("jms"));
        assert_eq!("ssh://", scheme_label("ssh"));
    }
}
