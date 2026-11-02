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
    let manifest = read_manifest(entry);
    let declared = manifest
        .as_ref()
        .and_then(|manifest| manifest.get("capabilities"))
        .cloned();
    let app_id = manifest
        .as_ref()
        .and_then(|manifest| manifest.get("id"))
        .and_then(|id| id.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| default_app_id(contribution));

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

fn read_manifest(entry: &Path) -> Option<serde_json::Value> {
    let path = entry.parent()?.join("nsh.json");
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
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
