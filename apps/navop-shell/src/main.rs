#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `navop-shell`：Shell。
//!
//! 应用 crate 只负责**装配**：选定产品身份、领域包与初始化流程；主页、设置、
//! 标签、AI、Notes、团队、扩展、凭据库、云同步、托盘、更新等能力全部来自共享
//! 外壳 `navop_app`（`main` 包）。

use std::sync::Arc;

use domain_api::{AppIdentity, Domain, Product, ProductFeatures};

struct ShellProduct;

impl Product for ShellProduct {
    type Assets = navop_app::AppAssets;

    fn identity(&self) -> &'static AppIdentity {
        &AppIdentity::SHELL
    }

    fn features(&self) -> ProductFeatures {
        ProductFeatures::SHELL
    }

    fn assets(&self) -> Self::Assets {
        navop_app::AppAssets::new()
    }

    fn domains(&self) -> Vec<Arc<dyn Domain>> {
        vec![Arc::new(shell_domain::ShellDomain)]
    }

    fn mcp_managed_by_app_kit(&self) -> bool {
        true
    }

    fn init(&self, cx: &mut gpui::App) -> anyhow::Result<()> {
        navop_app::init_app(cx)
    }
}

fn main() {
    navop_app::run_with(ShellProduct, env!("CARGO_PKG_VERSION"));
}
