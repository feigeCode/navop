//! 把一个 shell view contribution 变成可运行的 nsh 程序。
//!
//! 两种形态，页面的"入口"都是 manifest 里的 `entry`：
//!
//! - **签名包**：入口旁边的同名 `<stem>.nshpkg`（作者 `nsh publish` 的产物）。
//!   验签 → 解包资产 → 运行；首帧健康后再 `accept_package` 记下 build 数
//!   （两阶段，见迁移方案 §8）。
//! - **开发形态**：入口本身就是 JS 源码（作者 `nsh build` 产出的 `bundle.js`
//!   或手写单文件）。源码随扩展目录发布，没有签名可验，因此能力仍按
//!   `nsh.json` 显式声明限制。

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Context as _, Result, anyhow};
use extension_runtime::RegisteredShellViewContribution;
use nsh_platform_gpui::{App, AssetBase, Bundle, LoadedPackage};

/// 一个 view 的可运行程序。
pub(super) struct ViewProgram {
    pub app: App,
    pub asset_base: Option<AssetBase>,
    /// 落盘数据目录的名字（`nsh.json` 的 `id`）。
    pub app_id: String,
    /// 签名包：首帧健康后要记住的登记。
    pub accepted: Option<LoadedPackage>,
}

/// 装载入口：优先签名包，回落到源码。
pub(super) fn load(
    contribution: &RegisteredShellViewContribution,
    trust: &Rc<RefCell<nsh_platform_gpui::package::TrustStore>>,
) -> Result<ViewProgram> {
    let entry = &contribution.entry_path;
    let app_id = read_app_id(entry).unwrap_or_else(|| default_app_id(contribution));
    let declared = read_capabilities(entry);

    let package_path = entry.with_extension("nshpkg");
    if package_path.is_file() {
        let bytes = std::fs::read(&package_path)
            .with_context(|| format!("读取签名包失败: {}", package_path.display()))?;
        // 验签与解包都碰磁盘：这是嵌入路径上唯一的重活，M4 会挪到后台线程
        // （方案 §7）；M3 先同步做，失败信息足够定位。
        let loaded = nsh_platform_gpui::verify_package(&trust.borrow(), &bytes, &assets_root())
            .map_err(|message| anyhow!("验证 {} 失败: {message}", package_path.display()))?;
        tracing::info!(
            app = %loaded.app.id,
            build = loaded.app.build,
            "nsh 签名包已验证，等首帧健康后接受"
        );
        return Ok(ViewProgram {
            asset_base: Some(AssetBase::Directory(loaded.assets.clone())),
            app: App::Js(loaded.bundle.clone()),
            app_id,
            accepted: Some(loaded),
        });
    }

    let source = std::fs::read_to_string(entry)
        .with_context(|| format!("读取页面入口失败: {}", entry.display()))?;
    let mut bundle = Bundle::source(source);
    bundle.app_id = Some(app_id.clone());
    // 开发形态：保留 eval 便于调试（Policy::development 只关 harden），
    // 能力声明照样强制 —— 缺能力在开发期就该暴露，而不是等发布。
    bundle.policy.harden = false;
    match declared {
        Some(declared) => {
            let (capabilities, dropped) =
                nsh_core::access::Capabilities::from_json_lenient(&declared).map_err(|error| {
                    anyhow!("解析 {} 的 capabilities 失败: {error}", entry.display())
                })?;
            if !dropped.is_empty() {
                tracing::warn!(?dropped, path = %entry.display(), "nsh 能力声明有字段被丢弃");
            }
            bundle.policy.declared = Some(capabilities);
        }
        None => tracing::warn!(
            path = %entry.display(),
            "nsh.json 没有声明 capabilities：页面拿不到任何 host 能力"
        ),
    }
    Ok(ViewProgram {
        asset_base: asset_base_for(entry),
        app: App::Js(bundle),
        app_id,
        accepted: None,
    })
}

/// 两阶段装载的第二阶段：首帧健康，把这个 build 记下来（此后旧包不再通过
/// 验证，防回滚）。
pub(super) fn accept(
    loaded: &LoadedPackage,
    trust: &Rc<RefCell<nsh_platform_gpui::package::TrustStore>>,
) {
    if let Err(error) = nsh_platform_gpui::accept_package(&mut trust.borrow_mut(), loaded) {
        tracing::warn!(%error, "接受 nsh 签名包失败");
    }
}

/// 签名包资产解出的根目录。
pub(crate) fn assets_root() -> PathBuf {
    root().join("packages")
}

/// `<data dir>/extensions/nsh`；没有数据目录时退到配置目录（与 shell 的
/// 存储路径同一套兜底）。
fn root() -> PathBuf {
    one_core::app_dirs::data_dir()
        .map(Ok)
        .unwrap_or_else(one_core::app_dirs::config_dir)
        .unwrap_or_else(|_| std::env::temp_dir().join("navop-nsh"))
        .join("extensions")
        .join("nsh")
}

/// 信任库（根公钥 + 撤销表 + 已接受的 build）落在数据目录里，跨启动保留。
pub(crate) fn trust_store() -> Rc<RefCell<nsh_platform_gpui::package::TrustStore>> {
    let dir = root().join("trust");
    let store = match nsh_platform_gpui::package::TrustStore::open(&dir) {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(%error, path = %dir.display(), "打开 nsh 信任库失败，改用内存库");
            nsh_platform_gpui::package::TrustStore::new()
        }
    };
    Rc::new(RefCell::new(store))
}

/// 页面目录里的 app id。
///
/// 认两种形状：`nsh build` 的产物（`app.json`，只有 id）与作者手写的
/// `nsh.json`（源目录形态）。发布出去的页面目录里不该带 `nsh.json`——
/// 那是构建输入，宿主读产物。
fn read_app_id(entry: &Path) -> Option<String> {
    let dir = entry.parent()?;
    for name in ["app.json", "nsh.json"] {
        let text = std::fs::read_to_string(dir.join(name)).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        if let Some(id) = value.get("id").and_then(|id| id.as_str()) {
            return Some(id.to_owned());
        }
    }
    None
}

/// 页面声明的能力上限。
///
/// `capabilities.json` 是 `nsh build` 的产物；`nsh.json` 里的
/// `capabilities` 是源目录形态。取到哪个用哪个——**没有声明就没有能力**，
/// 由调用方按「不授予」处理（`Policy` 的默认值是 `Capabilities::all`，
/// 所以这里必须显式给出）。
fn read_capabilities(entry: &Path) -> Option<serde_json::Value> {
    let dir = entry.parent()?;
    if let Ok(text) = std::fs::read_to_string(dir.join("capabilities.json"))
        && let Ok(value) = serde_json::from_str(&text)
    {
        return Some(value);
    }
    let text = std::fs::read_to_string(dir.join("nsh.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("capabilities").cloned()
}

fn default_app_id(contribution: &RegisteredShellViewContribution) -> String {
    format!(
        "dev.navop.{}.{}",
        sanitize(&contribution.extension_id),
        sanitize(&contribution.id)
    )
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `asset:/` 的根：入口所在目录下的 `assets/`（作者 `nsh build` 的产物）。
fn asset_base_for(entry: &Path) -> Option<AssetBase> {
    let dir = entry.parent()?.join("assets");
    dir.is_dir().then_some(AssetBase::Directory(dir))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::rc::Rc;

    use extension_runtime::RegisteredShellViewContribution;
    use extension_runtime::extension::manifest::ShellSurface;

    use super::load;

    fn contribution(entry: PathBuf) -> RegisteredShellViewContribution {
        RegisteredShellViewContribution {
            extension_id: "com.navop.docker".into(),
            extension_version: "0.6.3".into(),
            id: "overview".into(),
            view_key: "com.navop.docker::overview".into(),
            title: "Docker Engine Overview".into(),
            description: None,
            icon_path: None,
            entry_path: entry,
            extension_root: PathBuf::from("."),
            surface: ShellSurface::Tab,
            category: None,
            keywords: Vec::new(),
            singleton: false,
            backends: BTreeMap::new(),
            modules: BTreeSet::new(),
            permissions: Vec::new(),
            shell_api_version: "1.0".into(),
            required_gpui_shell_version: String::new(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    #[test]
    fn built_pages_load_from_their_dist_layout() {
        // 试点页的产物就躺在扩展仓里：用它验证「dist/{bundle.js, app.json,
        // capabilities.json}」这套发布布局能被宿主直接装载 —— 页面目录里
        // 没有 nsh.json（那是构建输入），宿主只读产物。
        let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../navop-extensions/extensions/composite/docker/ui/pages/overview/dist");
        if !dist.join("bundle.js").is_file() {
            eprintln!("跳过：扩展仓里的试点页产物不存在（{}）", dist.display());
            return;
        }
        let trust = Rc::new(RefCell::new(nsh_platform_gpui::package::TrustStore::new()));
        let program = load(&contribution(dist.join("bundle.js")), &trust).expect("装载试点页");
        assert_eq!(program.app_id, "com.navop.docker.overview");
        assert!(program.asset_base.is_none(), "开发形态没有资产目录");
        let nsh_platform_gpui::App::Js(bundle) = &program.app else {
            panic!("页面应当是 JS 程序");
        };
        let declared = bundle
            .policy
            .declared
            .as_ref()
            .expect("capabilities.json 里的 host 能力应当被读成声明上限");
        assert!(
            declared.host.iter().any(|host| host == "navop.workbench.*"),
            "试点页要声明工作台能力，实际 {:?}",
            declared.host
        );
    }
}
