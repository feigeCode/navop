use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use anyhow::{Context, Result};
use tree_sitter::{Language, Parser, WasmStore, wasmtime};

use super::LanguageManifest;

#[derive(Clone)]
struct RegisteredManifest {
    manifest: LanguageManifest,
    source_path: PathBuf,
    loaded: bool,
}

#[derive(Default)]
struct RuntimeState {
    manifests: HashMap<String, RegisteredManifest>,
}

static STATE: LazyLock<Mutex<RuntimeState>> = LazyLock::new(Mutex::default);
static LOAD_LOCK: LazyLock<Mutex<()>> = LazyLock::new(Mutex::default);
static ENGINE: LazyLock<wasmtime::Engine> = LazyLock::new(wasmtime::Engine::default);
static WASM_STORE: LazyLock<Mutex<WasmStore>> = LazyLock::new(|| {
    let store =
        with_big_stack(|| WasmStore::new(&ENGINE).expect("init language extension wasm store"));
    Mutex::new(store)
});

/// cranelift 编译 tree-sitter wasm 语法时递归极深，会击穿普通工作线程的栈
/// （实测在 GPUI 后台任务线程上首次触发编译时 SIGBUS：guard page 命中）。
/// 语法编译只在进程内发生少数几次，这里统一放到大栈专用线程执行，
/// 编译结果由 tree_sitter/wasmtime 内部缓存，后续调用代价很低。
fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name("ts-wasm-compile".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn tree-sitter wasm compile thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

pub(super) fn load_wasm_language(name: &str, bytes: &[u8]) -> Result<Language> {
    let name = name.to_string();
    let bytes = bytes.to_vec();
    with_big_stack(move || {
        WASM_STORE
            .lock()
            .expect("language extension wasm store mutex poisoned")
            .load_language(&name, &bytes)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("load wasm language {name}"))
    })
}

pub(super) fn parser_factory(
    name: &str,
    bytes: &[u8],
) -> gpui_component::highlighter::LanguageParserFactory {
    let name = name.to_string();
    let bytes = bytes.to_vec();
    Arc::new(move || {
        // 语法高亮的 parser 可能在任意栈深的工作线程上创建，
        // 同样经大栈线程编译，避免首个使用者所在线程的栈不够。
        let name = name.clone();
        let bytes = bytes.clone();
        with_big_stack(move || {
            let mut store = WasmStore::new(&ENGINE).map_err(anyhow::Error::msg)?;
            let language = store
                .load_language(&name, &bytes)
                .map_err(anyhow::Error::msg)?;
            let mut parser = Parser::new();
            parser.set_wasm_store(store)?;
            Ok((parser, language))
        })
    })
}

pub(crate) fn register_manifest(manifest: LanguageManifest, source_path: PathBuf, loaded: bool) {
    STATE.lock().unwrap().manifests.insert(
        manifest.name.clone(),
        RegisteredManifest {
            manifest,
            source_path,
            loaded,
        },
    );
}

pub(crate) fn replace_manifests(root: &Path, manifests: Vec<(LanguageManifest, PathBuf)>) {
    let mut state = STATE.lock().unwrap();
    let loaded = state
        .manifests
        .iter()
        .filter(|(_, registered)| registered.loaded)
        .map(|(name, _)| name.clone())
        .collect::<std::collections::HashSet<_>>();
    state
        .manifests
        .retain(|_, registered| registered.source_path.parent() != Some(root));
    for (manifest, source_path) in manifests {
        let was_loaded = loaded.contains(&manifest.name);
        state.manifests.insert(
            manifest.name.clone(),
            RegisteredManifest {
                manifest,
                source_path,
                loaded: was_loaded,
            },
        );
    }
}

pub(crate) fn forget(name: &str) {
    STATE.lock().unwrap().manifests.remove(name);
}

pub(super) fn load_registered(identifier: &str) -> Result<bool> {
    let _load_guard = LOAD_LOCK.lock().unwrap();
    let registered = {
        let state = STATE.lock().unwrap();
        state
            .manifests
            .get(identifier)
            .or_else(|| {
                state.manifests.values().find(|registered| {
                    registered.manifest.name.eq_ignore_ascii_case(identifier)
                        || registered
                            .manifest
                            .file_extensions
                            .iter()
                            .any(|extension| extension.eq_ignore_ascii_case(identifier))
                })
            })
            .cloned()
    };
    let Some(registered) = registered else {
        return Ok(false);
    };
    if registered.loaded
        && gpui_component::highlighter::LanguageRegistry::singleton()
            .language(&registered.manifest.name)
            .is_some()
    {
        return Ok(true);
    }
    super::InstalledExtension::load_from_dir(&registered.source_path)?
        .register(gpui_component::highlighter::LanguageRegistry::singleton())?;
    Ok(true)
}

pub(super) fn registered_language_name(identifier: &str) -> Option<String> {
    let identifier = identifier.trim().trim_start_matches('.');
    let state = STATE.lock().unwrap();
    state
        .manifests
        .get(identifier)
        .or_else(|| {
            state.manifests.values().find(|registered| {
                registered.manifest.name.eq_ignore_ascii_case(identifier)
                    || registered
                        .manifest
                        .file_extensions
                        .iter()
                        .any(|extension| extension.eq_ignore_ascii_case(identifier))
            })
        })
        .map(|registered| registered.manifest.name.clone())
}

#[cfg(test)]
mod tests {
    use super::with_big_stack;

    #[test]
    fn big_stack_runner_propagates_value_and_panic() {
        assert_eq!(with_big_stack(|| 40 + 2), 42);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = with_big_stack(|| -> () { panic!("boom") });
        }));
        assert!(panicked.is_err(), "worker panic must propagate to caller");
    }
}
