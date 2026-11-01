//! 扩展下载的镜像偏好与 CNB 镜像地址改写。
//!
//! 扩展安装包与各自的 `extension-manifest.json` 同时在三个地方有副本：
//! R2（Cloudflare）、GitHub Releases 与 CNB Releases。镜像偏好复用
//! 「关于 → 更新」里的更新镜像下拉，只决定候选地址的尝试顺序，
//! 选中的源失败后仍会回退到其余镜像。
//!
//! CNB 只镜像 Release 资产（仓库文件直链需要登录），所以扩展市场清单也发布成
//! navop-extensions 固定 tag `manifest` 的 Release 资产：CNB 同步后即可国内直读，
//! 客户端不需要做 tag 发现。取不到时回退 R2（`extensions/manifest.json`）与
//! GitHub raw。

use std::sync::atomic::{AtomicU8, Ordering};

use one_core::settings::UpdateMirror;

/// CNB 上扩展 Release 资产的下载前缀。
pub(crate) const CNB_EXTENSION_RELEASE_BASE: &str =
    "https://cnb.cool/navop-dev/navop-extensions/-/releases/download/";

/// 只有 navop-extensions 的 Release 资产在 CNB 上有镜像；历史仓库
/// `onetcli-extensions` 没有同步，不能改写过去。
const GITHUB_EXTENSION_RELEASE_PREFIX: &str =
    "github.com/feigeCode/navop-extensions/releases/download/";

/// 扩展市场清单在 GitHub Release 上的地址。
///
/// navop-extensions 每次发布都会把 `manifest.json` 覆盖发到固定 tag `manifest`
/// 的 Release 下，客户端不需要做 tag 发现，GitHub 与 CNB 用同一段路径。
pub(crate) const GITHUB_MARKETPLACE_MANIFEST_RELEASE_URL: &str =
    "https://github.com/feigeCode/navop-extensions/releases/download/manifest/manifest.json";

/// 同一份扩展市场清单在 CNB 上的镜像地址（由 navop-extensions 的 CI 同步）。
///
/// `github-marketplace` 特性构建只会走 GitHub，这条候选不会被引用。
#[cfg_attr(feature = "github-marketplace", allow(dead_code))]
pub(crate) const CNB_MARKETPLACE_MANIFEST_URL: &str =
    "https://cnb.cool/navop-dev/navop-extensions/-/releases/download/manifest/manifest.json";

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

/// 扩展市场清单在 CNB 上的地址。
///
/// 市场清单以固定 tag `manifest` 的 Release 资产发布，因此不需要 tag 发现；
/// 该 Release 还没同步到 CNB 时只会 404，由调用方回退下一个源。
#[cfg_attr(feature = "github-marketplace", allow(dead_code))]
pub(crate) fn cnb_marketplace_manifest_url() -> &'static str {
    CNB_MARKETPLACE_MANIFEST_URL
}

/// 按镜像偏好给候选地址排序。
///
/// `candidates` 固定为 `[R2, GitHub, CNB]`；缺哪一项就跳过哪一项，
/// 默认顺序保持历史行为（R2 → GitHub → CNB），CNB 只做兜底。
/// 安装包、单扩展清单与市场清单共用这套排序。
pub(crate) fn ordered_mirror_candidates(
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
        CNB_EXTENSION_RELEASE_BASE, GITHUB_MARKETPLACE_MANIFEST_RELEASE_URL,
        cnb_marketplace_manifest_url, cnb_release_download_url, extension_mirror,
        ordered_mirror_candidates, set_extension_mirror,
    };
    use one_core::settings::UpdateMirror;

    #[test]
    fn marketplace_manifest_uses_the_fixed_release_tag() {
        assert_eq!(
            GITHUB_MARKETPLACE_MANIFEST_RELEASE_URL,
            "https://github.com/feigeCode/navop-extensions/releases/download/manifest/manifest.json"
        );
        assert_eq!(
            cnb_marketplace_manifest_url(),
            "https://cnb.cool/navop-dev/navop-extensions/-/releases/download/manifest/manifest.json"
        );
        // CNB 地址必须是 GitHub 地址同名改写来的，两边资产名与 tag 一致。
        assert_eq!(
            cnb_release_download_url(GITHUB_MARKETPLACE_MANIFEST_RELEASE_URL).as_deref(),
            Some(cnb_marketplace_manifest_url())
        );
    }

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
            ordered_mirror_candidates(
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
            ordered_mirror_candidates(
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
            ordered_mirror_candidates(
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
            ordered_mirror_candidates(UpdateMirror::Cnb, [r2.clone(), github.clone(), cnb.clone()]),
            vec![
                cnb.clone().unwrap(),
                r2.clone().unwrap(),
                github.clone().unwrap()
            ]
        );
        assert!(
            ordered_mirror_candidates(UpdateMirror::Cnb, [None, None, None]).is_empty(),
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
