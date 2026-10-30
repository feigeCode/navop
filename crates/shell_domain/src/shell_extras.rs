use std::sync::{Mutex, OnceLock};

use domain_api::{ShellExtras, WslDistributionInfo};
use gpui::App;

static WSL_DISTRIBUTIONS: OnceLock<Mutex<Option<Vec<WslDistributionInfo>>>> = OnceLock::new();

fn distributions_cache() -> &'static Mutex<Option<Vec<WslDistributionInfo>>> {
    WSL_DISTRIBUTIONS.get_or_init(|| Mutex::new(None))
}

pub struct ShellExtrasImpl;

impl ShellExtras for ShellExtrasImpl {
    fn wsl_distributions(&self, _cx: &App) -> Option<Vec<WslDistributionInfo>> {
        distributions_cache()
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    fn refresh_wsl_distributions(&self, cx: &mut App) {
        detect(cx);
    }
}

pub(crate) fn init(cx: &mut App) {
    detect(cx);
}

/// 后台识别 WSL 发行版，结果写入进程内缓存；识别完成前菜单不显示该区段。
fn detect(cx: &mut App) {
    if !cfg!(target_os = "windows") {
        return;
    }
    cx.spawn(async move |_cx| {
        let detected = smol::unblock(terminal::list_wsl_distributions).await;
        let Ok(distributions) = detected else {
            return;
        };
        let infos = distributions
            .into_iter()
            .map(|distribution| WslDistributionInfo {
                name: distribution.name.into(),
                is_default: distribution.is_default,
            })
            .collect::<Vec<_>>();
        if let Ok(mut guard) = distributions_cache().lock() {
            *guard = Some(infos);
        }
    })
    .detach();
}
