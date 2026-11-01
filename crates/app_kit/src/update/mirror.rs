//! 更新镜像源：Cloudflare（R2 上的 JSON 清单）、CNB 镜像、GitHub Releases。
//!
//! 三个源的信息形态并不一样，但最终都要落到同一份 [`super::UpdateDialogInfo`]：
//!
//! - Cloudflare：`{base}/updates/…/latest.json`，一份 JSON 清单，最直接；
//! - GitHub：Releases API 的 `releases/latest`，API 语义天然排除预发布，所以只在
//!   稳定通道上使用；
//! - CNB：既没有 JSON 清单的公开直链（仓库文件 raw 直链要登录），也没有 `latest`
//!   别名，只能先读 git smart HTTP 的 refs 广告拿到 tag，再去 Release 资产里取
//!   `feed-<product>.json`。
//!
//! 「镜像源」只决定「先问谁」：选中的源排第一，失败后仍按内置顺序依次回退，避免
//! 用户手动选错一次就再也查不到更新。

use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use one_core::settings::UpdateMirror;
use rust_i18n::t;
use semver::Version;

use super::UpdateChannel;
use super::config::is_legacy_full;
use super::util::parse_version;

/// CNB 仓库（发行包与 feed 资产都挂在这里的 Release 上）。
pub(crate) const CNB_REPOSITORY: &str = "navop-dev/navop";
/// CNB 网页域名，Release 资产下载走 `{base}/-/releases/download/<tag>/<asset>`。
pub(crate) const CNB_WEB_BASE: &str = "https://cnb.cool/navop-dev/navop";
const USER_AGENT: &str = "onetcli-updater";
/// 自动模式的尝试顺序：R2 是主站；CNB 在国内通常最快；GitHub 兜底。
pub(crate) const AUTO_MIRROR_ORDER: [UpdateMirror; 3] = [
    UpdateMirror::Cloudflare,
    UpdateMirror::Cnb,
    UpdateMirror::Github,
];

/// 把用户偏好的源排到第一位，其余保持内置相对顺序。
pub(crate) fn mirror_order(preferred: UpdateMirror) -> [UpdateMirror; 3] {
    let mut order = AUTO_MIRROR_ORDER;
    if let Some(index) = order.iter().position(|mirror| *mirror == preferred) {
        // `rotate_right(1)` 把选中的源整体挪到最前，后面的源顺序不变。
        order[..=index].rotate_right(1);
    }
    order
}

/// CNB 仓库的 git refs 广告地址；匿名可读，是唯一可靠的 tag 发现手段。
pub(crate) fn cnb_refs_url() -> String {
    format!("https://cnb.cool/{CNB_REPOSITORY}.git/info/refs?service=git-upload-pack")
}

/// CNB Release 资产地址。
pub(crate) fn cnb_release_asset_url(tag: &str, asset: &str) -> String {
    format!("{CNB_WEB_BASE}/-/releases/download/{tag}/{asset}")
}

/// feed 资产名：与 `upload-r2.yml` 上传到 Release 的扁平清单同名。
///
/// 之所以用扁平资产而不是 `updates/**` 目录结构：CNB 只同步 Release 资产，
/// 资产名里不能带路径分隔符。
pub(crate) fn feed_asset_name(product: &str, channel: UpdateChannel) -> String {
    match channel {
        UpdateChannel::Stable => format!("feed-{product}.json"),
        UpdateChannel::Beta => format!("feed-{product}-beta.json"),
    }
}

/// 当前产品在 feed 资产名里的产品段，与 `config::feed_url` 的产品段保持一致。
///
/// 身份未配置（例如刚启动的单测或开发构建）时退回全量产品 `navop`：CNB 镜像是一层
/// 兜底，宁可去问主产品的 feed，也不要因为认不出身份直接放弃。
pub(crate) fn feed_product_slug() -> &'static str {
    match crate::identity() {
        Some(identity) if !is_legacy_full(identity) => identity.binary_name,
        _ => "navop",
    }
}

/// 拉取 CNB 上的全部 tag（形如 `v0.23.0-beta.2`）。
pub(crate) async fn fetch_cnb_tags(
    http_client: Arc<dyn HttpClient>,
) -> Result<Vec<String>, String> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(cnb_refs_url())
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/x-git-upload-pack-advertisement")
        .body(AsyncBody::empty())
        .map_err(|err| format!("构建 CNB tag 请求失败: {err}"))?;

    let mut response = http_client
        .send(request)
        .await
        .map_err(|err| format!("{}: {err}", t!("Update.network_unreachable")))?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .map_err(|err| format!("读取 CNB tag 列表失败: {err}"))?;

    if !status.is_success() {
        return Err(format!("CNB tag 列表返回异常状态码: {status}"));
    }

    Ok(parse_advertised_tags(&body))
}

/// 解析 git smart HTTP 的 refs 广告（pkt-line），只保留 `vX.Y.Z[-pre]` 形式的 tag。
///
/// 广告体开头是一行 `# service=git-upload-pack`，随后每个 pkt-line 是
/// `<oid> <ref>\0<capabilities>`；带注释的 tag 还会多一条 `<ref>^{}` 的剥离行，
/// 两者要去重成同一个 tag。
pub(crate) fn parse_advertised_tags(body: &[u8]) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    while cursor + 4 <= body.len() {
        let Ok(header) = std::str::from_utf8(&body[cursor..cursor + 4]) else {
            break;
        };
        let Ok(length) = usize::from_str_radix(header, 16) else {
            break;
        };
        cursor += 4;
        if length == 0 {
            // flush-pkt
            continue;
        }
        if length < 4 || cursor + length - 4 > body.len() {
            break;
        }
        let payload = String::from_utf8_lossy(&body[cursor..cursor + length - 4]).to_string();
        cursor += length - 4;

        // 第一行可能是服务声明，ref 行也可能和 capabilities 挤在同一个 pkt-line 里。
        for line in payload.split(['\n', '\0']) {
            let mut parts = line.split(' ');
            let _oid = parts.next();
            let Some(reference) = parts.next() else {
                continue;
            };
            let Some(tag) = reference.strip_prefix("refs/tags/") else {
                continue;
            };
            let tag = tag.trim_end_matches("^{}");
            if parse_version(tag).is_none() {
                continue;
            }
            if !tags.iter().any(|existing| existing == tag) {
                tags.push(tag.to_string());
            }
        }
    }
    tags
}

/// 指定通道上最新的 tag。
///
/// 稳定通道只看正式版；测试通道也接受正式版（切回稳定版后仍能收到下个正式版）。
pub(crate) fn latest_tag_for_channel(tags: &[String], channel: UpdateChannel) -> Option<String> {
    let mut best: Option<(Version, &String)> = None;
    for tag in tags {
        let Some(version) = parse_version(tag) else {
            continue;
        };
        if channel == UpdateChannel::Stable && !version.pre.is_empty() {
            continue;
        }
        let newer = match &best {
            Some((current, _)) => version > *current,
            None => true,
        };
        if newer {
            best = Some((version, tag));
        }
    }
    best.map(|(_, tag)| tag.clone())
}

/// 把清单里的下载地址改写到 CNB 的 Release 资产上。
///
/// CNB 资产与 GitHub 资产文件名一一对应，所以只换前缀和 tag、文件名保持不变；
/// 取不到文件名时返回 `None`，让调用方保留原地址。
pub(crate) fn cnb_download_url(tag: &str, url: &str) -> Option<String> {
    let trimmed = url.split(['?', '#']).next().unwrap_or(url);
    let asset = trimmed.rsplit('/').next()?.trim();
    if asset.is_empty() {
        None
    } else {
        Some(cnb_release_asset_url(tag, asset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把一个 payload 编码成 git pkt-line。
    fn pkt(payload: &str) -> String {
        format!("{:04x}{}", payload.len() + 4, payload)
    }

    #[test]
    fn mirror_order_puts_the_chosen_source_first_without_dropping_fallbacks() {
        assert_eq!(AUTO_MIRROR_ORDER, mirror_order(UpdateMirror::Auto));
        assert_eq!(
            [
                UpdateMirror::Cloudflare,
                UpdateMirror::Cnb,
                UpdateMirror::Github
            ],
            mirror_order(UpdateMirror::Cloudflare)
        );
        assert_eq!(
            [
                UpdateMirror::Cnb,
                UpdateMirror::Cloudflare,
                UpdateMirror::Github
            ],
            mirror_order(UpdateMirror::Cnb)
        );
        assert_eq!(
            [
                UpdateMirror::Github,
                UpdateMirror::Cloudflare,
                UpdateMirror::Cnb
            ],
            mirror_order(UpdateMirror::Github)
        );
    }

    #[test]
    fn cnb_top_level_products_share_one_repository_url() {
        assert_eq!(
            "https://cnb.cool/navop-dev/navop.git/info/refs?service=git-upload-pack",
            cnb_refs_url()
        );
        assert_eq!(
            "https://cnb.cool/navop-dev/navop/-/releases/download/v0.22.0/feed-navop.json",
            cnb_release_asset_url("v0.22.0", "feed-navop.json")
        );
    }

    #[test]
    fn parse_advertised_tags_keeps_version_tags_and_dedupes_peeled_refs() {
        let mut body = String::new();
        body.push_str(&pkt("# service=git-upload-pack\n"));
        body.push_str("0000");
        body.push_str(&pkt(
            "f939102e54acaaf02037735225892766e0f6bf52 HEAD\0multi_ack thin-pack side-band\n",
        ));
        body.push_str(&pkt(
            "1111111111111111111111111111111111111111 refs/heads/main\n",
        ));
        body.push_str(&pkt(
            "2222222222222222222222222222222222222222 refs/tags/v0.22.0\n",
        ));
        body.push_str(&pkt(
            "3333333333333333333333333333333333333333 refs/tags/v0.23.0-beta.2\n",
        ));
        body.push_str(&pkt(
            "4444444444444444444444444444444444444444 refs/tags/v0.23.0-beta.2^{}\n",
        ));

        let tags = parse_advertised_tags(body.as_bytes());
        assert_eq!(vec!["v0.22.0", "v0.23.0-beta.2"], tags);
    }

    #[test]
    fn parse_advertised_tags_survives_a_truncated_advertisement() {
        let mut body = String::new();
        body.push_str(&pkt(
            "2222222222222222222222222222222222222222 refs/tags/v0.22.0\n",
        ));
        body.push_str("0020abc");

        let tags = parse_advertised_tags(body.as_bytes());
        assert_eq!(vec!["v0.22.0"], tags);
    }

    #[test]
    fn latest_tag_for_channel_follows_semver_and_the_channel_rule() {
        let tags = vec![
            "v0.22.0".to_string(),
            "v0.23.0-beta.2".to_string(),
            "v0.23.0-beta.10".to_string(),
        ];
        assert_eq!(
            Some("v0.22.0".to_string()),
            latest_tag_for_channel(&tags, UpdateChannel::Stable)
        );
        assert_eq!(
            Some("v0.23.0-beta.10".to_string()),
            latest_tag_for_channel(&tags, UpdateChannel::Beta)
        );
    }

    #[test]
    fn feed_asset_names_are_flat_and_channel_scoped() {
        assert_eq!(
            "feed-navop.json",
            feed_asset_name("navop", UpdateChannel::Stable)
        );
        assert_eq!(
            "feed-navop-beta.json",
            feed_asset_name("navop", UpdateChannel::Beta)
        );
        assert_eq!(
            "feed-navop-shell.json",
            feed_asset_name("navop-shell", UpdateChannel::Stable)
        );
    }

    #[test]
    fn cnb_download_url_keeps_the_asset_file_name() {
        assert_eq!(
            Some(
                "https://cnb.cool/navop-dev/navop/-/releases/download/v0.22.0/navop-0.22.0-macos-arm64.tar.gz"
                    .to_string()
            ),
            cnb_download_url(
                "v0.22.0",
                "https://r2.example.test/releases/v0.22.0/navop-0.22.0-macos-arm64.tar.gz?x=1#frag"
            )
        );
        assert_eq!(None, cnb_download_url("v0.22.0", ""));
    }
}
