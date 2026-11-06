//! 把一个 shell view contribution 变成可运行的 nsh 程序。
//!
//! 两种形态，页面的"入口"都是 manifest 里的 `entry`：
//!
//! - **签名包**：入口旁边的同名 `<stem>.nshpkg`（作者 `nsh publish` 的产物）。
//!   验签 → 解包资产 → 运行；首帧健康后再 `accept_package` 记下 build 数
//!   （两阶段，见迁移方案 §8）。
//! - **开发形态**：入口本身就是 JS 源码（作者 `nsh build` 产出的 `bundle.js`
//!   或手写单文件）。源码随扩展目录发布，没有签名可验；能力面同样只认
//!   `extension.json`（宿主权威），页面自带的 `nsh.json` / `capabilities.json`
//!   只用来做诊断比对。

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Context as _, Result, anyhow};
use extension_runtime::RegisteredShellAppContribution;
use nsh_core::access::Capabilities;
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
    contribution: &RegisteredShellAppContribution,
    trust: &Rc<RefCell<nsh_platform_gpui::package::TrustStore>>,
) -> Result<ViewProgram> {
    let entry = &contribution.entry_path;
    let app_id = read_app_id(entry).unwrap_or_else(|| default_app_id(contribution));

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
        // 签名包只证明"包完整、发布者可信"，不证明"页面该拿什么能力"。能力面
        // 一律从安装期注册的 extension.json 重建（`contribution` 就是它的注册
        // 结果），包内若有能力声明也不生效。
        let mut bundle = loaded.bundle.clone();
        let granted = apply_host_policy(&mut bundle, contribution);
        log_page_declaration_drift(entry, &granted);
        return Ok(ViewProgram {
            asset_base: Some(AssetBase::Directory(loaded.assets.clone())),
            app: App::Js(bundle),
            app_id,
            accepted: Some(loaded),
        });
    }

    let source = std::fs::read_to_string(entry)
        .with_context(|| format!("读取页面入口失败: {}", entry.display()))?;
    let mut bundle = Bundle::source(source);
    bundle.app_id = Some(app_id.clone());
    let granted = apply_host_policy(&mut bundle, contribution);
    log_page_declaration_drift(entry, &granted);
    Ok(ViewProgram {
        asset_base: asset_base_for(entry),
        app: App::Js(bundle),
        app_id,
        accepted: None,
    })
}

/// 宿主权威的能力面：`declared` 与 `granted` 都由 `extension.json` 派生。
///
/// 这样页面自带的 `capabilities.json` / `nsh.json` 不能自行扩权，也不会因为
/// "没声明" 被 nsh 读成 `Capabilities::all()`（见 `Policy::access`）。返回值是
/// 授予上限，调用方拿去和页面自带声明做诊断比对。
///
/// `harden = true` 两条加载路径一致：开发形态也不能因为"源码没签名"而放宽，
/// 缺能力在开发期就该暴露。
fn apply_host_policy(
    bundle: &mut Bundle,
    contribution: &RegisteredShellAppContribution,
) -> Capabilities {
    let (granted, skipped) = super::grant::granted(contribution);
    if !skipped.is_empty() {
        tracing::warn!(
            ?skipped,
            extension = %contribution.extension_id,
            "这些权限无法翻译成 nsh 能力，已按 fail-closed 丢弃"
        );
    }
    bundle.policy.harden = true;
    bundle.policy.granted = Some(granted.clone());
    bundle.policy.declared = Some(granted.clone());
    granted
}

/// 页面自带的声明只作诊断：与宿主授予不同就记 debug，实际生效的永远是
/// `extension.json`。文件缺失或解析失败都不再是错误，也不再单独告警。
fn log_page_declaration_drift(entry: &Path, granted: &Capabilities) {
    let Some(value) = read_capabilities(entry) else {
        return;
    };
    let Ok((page, dropped)) = Capabilities::from_json_lenient(&value) else {
        tracing::debug!(
            path = %entry.display(),
            "页面自带的能力声明无法解析，忽略（以 extension.json 为准）"
        );
        return;
    };
    if !dropped.is_empty() {
        tracing::debug!(
            ?dropped,
            path = %entry.display(),
            "页面自带的能力声明有字段被丢弃"
        );
    }
    let mut page_hosts = page.host;
    let mut granted_hosts = granted.host.clone();
    page_hosts.sort();
    granted_hosts.sort();
    if page_hosts != granted_hosts {
        tracing::debug!(
            path = %entry.display(),
            ?page_hosts,
            ?granted_hosts,
            "页面自带的能力声明与宿主授予不同，以 extension.json 为准"
        );
    }
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

/// 页面自带的能力声明（只用于诊断）。
///
/// `capabilities.json` 是 `nsh build` 的产物；`nsh.json` 里的
/// `capabilities` 是源目录形态。取到哪个用哪个。v2 起宿主不再信任它：生效的
/// 能力面来自 `extension.json`（见 `apply_host_policy`），这里读到的只用于
/// debug 比对与定位作者 misunderstanding。
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

fn default_app_id(contribution: &RegisteredShellAppContribution) -> String {
    format!("dev.navop.{}", sanitize(&contribution.extension_id))
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

    use extension_runtime::RegisteredShellAppContribution;
    use extension_runtime::extension::manifest::ShellHostModule;

    use super::load;

    fn contribution(entry: PathBuf, modules: &[ShellHostModule]) -> RegisteredShellAppContribution {
        RegisteredShellAppContribution {
            extension_id: "com.navop.docker".into(),
            extension_version: "0.6.3".into(),
            title: "Docker".into(),
            description: None,
            icon_path: None,
            entry_path: entry,
            extension_root: PathBuf::from("."),
            category: None,
            keywords: Vec::new(),
            backends: BTreeMap::new(),
            modules: modules.iter().copied().collect::<BTreeSet<_>>(),
            permissions: Vec::new(),
            shell_api_version: "1.0".into(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    #[test]
    fn built_pages_load_from_their_dist_layout() {
        // 试点 app 的产物就躺在扩展仓里：用它验证「dist/{bundle.js, app.json,
        // capabilities.json}」这套发布布局能被宿主直接装载 —— ui/ 里
        // 没有 nsh.json 的产物副本（那是构建输入），宿主只读产物。
        let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../navop-extensions/extensions/composite/docker/ui/dist");
        if !dist.join("bundle.js").is_file() {
            eprintln!("跳过：扩展仓里的试点页产物不存在（{}）", dist.display());
            return;
        }
        let trust = Rc::new(RefCell::new(nsh_platform_gpui::package::TrustStore::new()));
        let program = load(
            &contribution(dist.join("bundle.js"), &[ShellHostModule::Resource]),
            &trust,
        )
        .expect("装载试点页");
        assert_eq!(program.app_id, "com.navop.docker");
        assert!(program.asset_base.is_none(), "开发形态没有资产目录");
        let nsh_platform_gpui::App::Js(bundle) = &program.app else {
            panic!("页面应当是 JS 程序");
        };
        // v2：两条加载路径都关掉 harden 放宽，declared 与 granted 都由宿主
        // 按 extension.json 给出，不再读页面自带的 capabilities.json。
        assert!(bundle.policy.harden, "harden 必须始终为 true");
        assert_eq!(
            bundle.policy.declared.as_ref(),
            bundle.policy.granted.as_ref(),
            "declared 必须等于宿主授予上限"
        );
        let granted = bundle
            .policy
            .granted
            .as_ref()
            .expect("宿主必须显式给出 granted");
        assert!(
            granted.host.iter().any(|host| host == "navop.resource.*"),
            "资源模块要派生资源命名空间，实际 {:?}",
            granted.host
        );
        assert!(
            !granted
                .host
                .iter()
                .any(|host| host.starts_with("navop.workbench")),
            "页面不该再依赖已删除的工作台通道，实际 {:?}",
            granted.host
        );
    }

    /// 扩展仓里每个已发布 app 的产物都要能被宿主装载。
    ///
    /// 这条测试故意做成"扫目录"：新扩一个 app 就自动多覆盖一个，不依赖谁记得
    /// 往清单里加一行。`extensions/` 与 `examples/` 都扫；产物不在（没初始化
    /// 子模块）时跳过。
    #[test]
    fn every_built_page_in_the_extension_repo_loads() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../navop-extensions");
        // CI 的 checkout 不初始化子模块：目录在、内容是空壳，这时要整条跳过，
        // 而不是拿「一个产物都没有」去断言失败。
        if !root.join("extensions").is_dir() {
            eprintln!("跳过：扩展仓子模块没有检出（{}）", root.display());
            return;
        }
        let trust = Rc::new(RefCell::new(nsh_platform_gpui::package::TrustStore::new()));
        let mut loaded = Vec::new();
        for bundle in dist_bundles(&root) {
            let program = load(
                &contribution(bundle.clone(), &[ShellHostModule::Resource]),
                &trust,
            )
            .unwrap_or_else(|error| panic!("装载 {} 失败：{error:#}", bundle.display()));
            assert!(
                !program.app_id.is_empty(),
                "{} 的 app_id 来自产物里的 app.json，不能为空",
                bundle.display()
            );
            // 一个扩展一个包：产物里的 app id 必须是扩展 id，否则 UI 与
            // manifest 会各说各话（旧模型一页一个 app id 就是这么漂的）。
            let extension_dir = bundle
                .ancestors()
                .nth(3)
                .unwrap_or_else(|| panic!("{} 不在 <extension>/ui/dist 下", bundle.display()));
            let manifest_text = std::fs::read_to_string(extension_dir.join("extension.json"))
                .unwrap_or_else(|error| {
                    panic!("读 {} 的清单失败：{error}", extension_dir.display())
                });
            let manifest: serde_json::Value =
                serde_json::from_str(&manifest_text).unwrap_or_else(|error| {
                    panic!("解析 {} 的清单失败：{error}", extension_dir.display())
                });
            let extension_id = manifest["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{} 的清单缺 id", extension_dir.display()));
            assert_eq!(
                program.app_id,
                extension_id,
                "{} 的 app id 必须等于扩展 id",
                bundle.display()
            );
            let nsh_platform_gpui::App::Js(js) = &program.app else {
                panic!("{} 应当是 JS 程序", bundle.display());
            };
            assert!(
                js.policy.harden,
                "{} 的 harden 必须为 true（开发形态也不放宽）",
                bundle.display()
            );
            assert_eq!(
                js.policy.declared.as_ref(),
                js.policy.granted.as_ref(),
                "{} 的 declared 必须等于宿主授予上限",
                bundle.display()
            );
            let grants_resource =
                js.policy.granted.as_ref().is_some_and(|granted| {
                    granted.host.iter().any(|host| host == "navop.resource.*")
                });
            assert!(
                grants_resource,
                "{} 声明的 resource 模块没有派生到宿主能力",
                bundle.display()
            );
            loaded.push(program.app_id);
        }
        eprintln!("已装载 {} 个 app：{loaded:?}", loaded.len());
        assert!(!loaded.is_empty(), "扩展仓里应当至少有一个 app 产物");
        let mut unique = loaded.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            loaded.len(),
            "同一扩展只能产出一个 app，实际 {loaded:?}"
        );
    }

    /// 递归找 `<extension>/ui/dist/bundle.js`；不引入 walkdir。
    ///
    /// `extensions/composite/<ext>` 与 `examples/<ext>` 深度不同，所以按目录递归，
    /// 但不进 `ui/`、`dist/`、`node_modules/`、`target/`。
    fn dist_bundles(root: &std::path::Path) -> Vec<PathBuf> {
        fn walk(dir: &std::path::Path, depth: usize, found: &mut Vec<PathBuf>) {
            if depth > 3 {
                return;
            }
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let bundle = path.join("ui/dist/bundle.js");
                if bundle.is_file() {
                    found.push(bundle);
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if matches!(
                    name,
                    "node_modules" | "target" | "ui" | "dist" | ".nsh" | "src"
                ) {
                    continue;
                }
                walk(&path, depth + 1, found);
            }
        }

        let mut found = Vec::new();
        walk(root, 0, &mut found);
        found.sort();
        found
    }
}
