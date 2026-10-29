#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `navop-data`：数据库 / Redis / MongoDB 产品。
//!
//! 与全量产品共用同一套应用外壳与主页，只换产品身份（数据目录、密钥链、MCP
//! 发现、更新 feed）与领域装配；入口显隐由领域注册表与 `ProductFeatures` 决定。

fn main() {
    navop_app::run_product_with_version(
        navop_app::ProductKind::Data,
        env!("CARGO_PKG_VERSION"),
    );
}
