//! 扩展下载的镜像偏好与 CNB 镜像地址改写。
//!
//! 扩展安装包与各自的 `extension-manifest.json` 同时在三个地方有副本：
//! R2（Cloudflare）、GitHub Releases 与 CNB Releases。镜像偏好复用
//! 「关于 → 更新」里的更新镜像下拉，只决定候选地址的尝试顺序，
//! 选中的源失败后仍会回退到其余镜像。
//!
//! CNB 只镜像 Release 资产，不镜像 marketplace 的 `manifest.json`
//! （仓库文件直链需要登录），所以 CNB 偏好只影响安装包与各自清单的取数顺序，
//! 扩展市场列表仍走 R2 / GitHub。

use std::sync::atomic::{AtomicU8, Ordering};

use one_core::settings::UpdateMirror;

/// CNB 上扩展 Release 资产的下载前缀。
pub(crate) const CNB_EXTENSION_RELEASE_BASE: &str =
    "https://cnb.cool/navop-dev/navop-extensions/-/releases/download/";

/// 只有 navop-extensions 的 Release 资产在 CNB 上有镜像；历史仓库
/// `onetcli-extensions` 没有同步，不能改写过去。
const GITHUB_EXTENSION_RELEASE_PREFIX: &str =
    "github.com/feigeCode/navop-extensions/releases/download/";

/// 进程内的扩展镜像偏好；启动时与设置页切换时由宿主写入。
static EXTENSION_MIRROR: AtomicU8 = AtomicU8::new(0);

fn mirror_code(mirror: UpdateMirror) -> u8 {
    match mirror {
        UpdateMirror::Auto => 0,
        UpdateMirror::Cloudflare => 1,
        UpdateMirror::Cnb => 2,
        UpdateMirror::Github => 3,
    }
}

fn mirror_from_code(code: u8) -> UpdateMirror {
    match code {
        1 => UpdateMirror::Cloudflare,
        2 => UpdateMirror::Cnb,
        3 => UpdateMirror::Github,
        _ => UpdateMirror::Auto,
    }
}

/// 记录用户选择的扩展镜像偏好。
pub fn set_extension_mirror(mirror: UpdateMirror) {
    EXTENSION_MIRROR.store(mirror_code(mirror), Ordering::Relaxed);
}

/// 当前生效的扩展镜像偏好。
pub fn extension_mirror() -> UpdateMirror {
    mirror_from_code(EXTENSION_MIRROR.load(Ordering::Relaxed))
}

/// 把 GitHub Release 资产地址改写成 CNB 上的同名资产。
///
/// 两个远端用同一个 tag 同步同一批资产，因此只需替换前缀。
pub(crate) fn cnb_release_download_url(url: &str) -> Option<String> {
    let url = url.trim();
    let index = url.find(GITHUB_EXTENSION_RELEASE_PREFIX)?;
    let asset_path = url[index + GITHUB_EXTENSION_RELEASE_PREFIX.len()..].trim_start_matches('/');
    if asset_path.is_empty() {
        return None;
    }
    Some(format!("{CNB_EXTENSION_RELEASE_BASE}{asset_path}"))
}

/// 按镜像偏好给下载地址候选排序。
///
/// `candidates` 固定为 `[R2, GitHub, CNB]`；缺哪一项就跳过哪一项，
/// 默认顺序保持历史行为（R2 → GitHub → CNB），CNB 只做兜底。
pub(crate) fn ordered_download_urls(
    preference: UpdateMirror,
    candidates: [Option<String>; 3],
) -> Vec<String> {
    let order: [usize; 3] = match preference {
        UpdateMirror::Github => [1, 0, 2],
        UpdateMirror::Cnb => [2, 0, 1],
        UpdateMirror::Auto | UpdateMirror::Cloudflare => [0, 1, 2],
    };
    let mut urls: Vec<String> = Vec::new();
    for index in order {
        if let Some(url) = candidates[index].clone() {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::{
        CNB_EXTENSION_RELEASE_BASE, cnb_release_download_url, extension_mirror,
        ordered_download_urls, set_extension_mirror,
    };
    use one_core::settings::UpdateMirror;

    #[test]
    fn cnb_mirror_rewrites_github_release_assets() {
        let url = "https://github.com/feigeCode/navop-extensions/releases/download/clickhouse-v0.1.0/clickhouse-driver-aarch64-apple-darwin.tar.gz";
        let rewritten = cnb_release_download_url(url).expect("应能改写成 CNB 资产地址");
        assert_eq!(
            rewritten,
            format!(
                "{CNB_EXTENSION_RELEASE_BASE}clickhouse-v0.1.0/clickhouse-driver-aarch64-apple-darwin.tar.gz"
            )
        );
        assert!(
            cnb_release_download_url("https://navop.dev/zh-CN/extensions").is_none(),
            "非 Release 资产地址不应被改写"
        );
        assert!(
            cnb_release_download_url(
                "https://github.com/feigeCode/onetcli-extensions/releases/download/clickhouse-v0.1.0/a.tar.gz"
            )
            .is_none(),
            "历史仓库没有 CNB 镜像，不应改写出无效地址"
        );
        assert!(
            cnb_release_download_url(
                "https://github.com/feigeCode/navop-extensions/releases/download/"
            )
            .is_none(),
            "缺少资产名时不应生成空地址"
        );
    }

    #[test]
    fn mirror_preference_controls_the_download_order() {
        let r2 = Some("https://cdn.example.test/a.tar.gz".to_string());
        let github = Some("https://github.com/o/r/releases/download/t/a.tar.gz".to_string());
        let cnb = Some("https://cnb.cool/o/r/-/releases/download/t/a.tar.gz".to_string());

        assert_eq!(
            ordered_download_urls(
                UpdateMirror::Auto,
                [r2.clone(), github.clone(), cnb.clone()]
            ),
            vec![
                r2.clone().unwrap(),
                github.clone().unwrap(),
                cnb.clone().unwrap()
            ]
        );
        assert_eq!(
            ordered_download_urls(
                UpdateMirror::Cloudflare,
                [r2.clone(), github.clone(), cnb.clone()]
            ),
            vec![
                r2.clone().unwrap(),
                github.clone().unwrap(),
                cnb.clone().unwrap()
            ]
        );
        assert_eq!(
            ordered_download_urls(
                UpdateMirror::Github,
                [r2.clone(), github.clone(), cnb.clone()]
            ),
            vec![
                github.clone().unwrap(),
                r2.clone().unwrap(),
                cnb.clone().unwrap()
            ]
        );
        assert_eq!(
            ordered_download_urls(UpdateMirror::Cnb, [r2.clone(), github.clone(), cnb.clone()]),
            vec![
                cnb.clone().unwrap(),
                r2.clone().unwrap(),
                github.clone().unwrap()
            ]
        );
        assert!(
            ordered_download_urls(UpdateMirror::Cnb, [None, None, None]).is_empty(),
            "没有候选地址时应返回空列表"
        );
    }

    #[test]
    fn extension_mirror_preference_round_trips() {
        set_extension_mirror(UpdateMirror::Cnb);
        assert_eq!(extension_mirror(), UpdateMirror::Cnb);
        set_extension_mirror(UpdateMirror::Auto);
        assert_eq!(extension_mirror(), UpdateMirror::Auto);
    }
}
