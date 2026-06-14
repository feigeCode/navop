//! 全局 wasmtime engine 与 WasmStore 管理。
//!
//! Tree-sitter 允许 `Language` 与解析时使用的 `WasmStore` 来自不同实例,
//! 只要二者基于同一个 `wasmtime::Engine`。本模块提供:
//!
//! - 全局 `Engine` 单例(`engine()`),所有 wasm store 都基于它创建
//! - 全局"注册表"`WasmStore`(`with_registry_store`),用于 `LanguageRegistry`
//!   注册时加载 wasm 并产生 `Language` 句柄;该 store 永不释放,保证 Language
//!   引用始终有效
//! - `new_parser_store()`:为每个并发使用的 parser 创建独立 store
//!
//! `WasmStore` 本身实现 `Send + Sync`(tree-sitter 0.25 起),但内部包装了
//! 可变 wasmtime 状态,因此通过 `Mutex` 串行化全局访问。

use std::sync::{LazyLock, Mutex, MutexGuard};

use anyhow::{Context, Result};
use tree_sitter::{WasmStore, wasmtime};

static ENGINE: LazyLock<wasmtime::Engine> = LazyLock::new(wasmtime::Engine::default);

static REGISTRY_STORE: LazyLock<Mutex<WasmStore>> = LazyLock::new(|| {
    let store = WasmStore::new(engine()).expect("init global tree-sitter wasm registry store");
    Mutex::new(store)
});

/// 返回全局 wasmtime engine。
pub fn engine() -> &'static wasmtime::Engine {
    &ENGINE
}

/// 在持有全局注册 store 锁的情况下执行闭包。
///
/// 用于在 `LanguageRegistry::register_wasm` 中将 wasm 字节加载为
/// `tree_sitter::Language`。
pub(crate) fn with_registry_store<R>(f: impl FnOnce(&mut WasmStore) -> R) -> R {
    let mut guard: MutexGuard<'_, WasmStore> = REGISTRY_STORE
        .lock()
        .expect("global wasm registry store mutex poisoned");
    f(&mut guard)
}

/// 创建一个全新的 `WasmStore`(基于全局 engine)。
///
/// 每个并发解析 wasm 语言的 parser 应当持有独立的 store。
pub fn new_parser_store() -> Result<WasmStore> {
    WasmStore::new(engine()).context("create tree-sitter WasmStore")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_returns_same_instance() {
        let a = engine();
        let b = engine();
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn new_parser_store_creates_independent_stores() {
        let s1 = new_parser_store().expect("first store");
        let s2 = new_parser_store().expect("second store");
        assert_eq!(s1.language_count(), 0);
        assert_eq!(s2.language_count(), 0);
    }

    #[test]
    fn registry_store_is_accessible() {
        // 主要验证持锁闭包能正常执行,无 panic
        let _count = with_registry_store(|store| store.language_count());
    }
}
