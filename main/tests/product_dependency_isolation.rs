//! 产品依赖隔离契约：独立产品的依赖图里不能出现对方领域的 crate。
//!
//! 三个产品共用 `main`(navop_app)，领域代码靠 `database`/`shell` Cargo feature
//! 裁剪。feature 一旦写漏（比如某个 cfg 反了、或者新代码直接引用领域 crate），
//! 产物会悄悄把整条领域依赖链带回来——编译期不会报错，只有称重和依赖树能发现。
//! 这里把「依赖树」变成可回归的断言，替代人工 `cargo tree | grep`。

use std::process::Command;

/// 终端产品（`navop-shell`）不该出现的数据库领域 crate。
const DATABASE_CRATES: &[&str] = &["db_view", "redis_view", "mongodb_view"];

/// 数据库产品（`navop-data`）不该出现的终端领域 crate。
const SHELL_CRATES: &[&str] = &[
    "terminal_view",
    "sftp_view",
    "remote_desktop_view",
    "known_hosts_view",
    "port_forwarding_view",
];

#[test]
fn standalone_products_do_not_depend_on_the_other_domain() {
    assert_dependency_tree_excludes("navop-shell", DATABASE_CRATES);
    assert_dependency_tree_excludes("navop-data", SHELL_CRATES);
}

fn assert_dependency_tree_excludes(product: &str, forbidden: &[&str]) {
    let tree = cargo_tree(product);
    let leaked: Vec<&str> = forbidden
        .iter()
        .copied()
        .filter(|name| tree.iter().any(|package| package == name))
        .collect();

    assert!(
        leaked.is_empty(),
        "{product} 的依赖树不应包含领域 crate {leaked:?}（共 {} 个包）",
        tree.len()
    );
}

/// 列出 `cargo tree -p <product> -e normal` 里的包名（去掉版本号）。
fn cargo_tree(product: &str) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["tree", "-p", product, "-e", "normal", "--prefix", "none"])
        .output()
        .expect("cargo tree 应该能执行");

    assert!(
        output.status.success(),
        "cargo tree -p {product} 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            line.split_once(" v")
                .map(|(name, _)| name.trim().to_string())
        })
        .collect()
}
