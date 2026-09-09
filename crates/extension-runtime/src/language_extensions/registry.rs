use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use anyhow::{Context, Result};
use tree_sitter::{Language, WasmStore, wasmtime};

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
static ENGINE: LazyLock<wasmtime::Engine> = LazyLock::new(wasmtime::Engine::default);
static WASM_STORE: LazyLock<Mutex<WasmStore>> = LazyLock::new(|| {
    let store = WasmStore::new(&ENGINE).expect("init language extension wasm store");
    Mutex::new(store)
});

pub(super) fn load_wasm_language(name: &str, bytes: &[u8]) -> Result<Language> {
    WASM_STORE
        .lock()
        .expect("language extension wasm store mutex poisoned")
        .load_language(name, bytes)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("load wasm language {name}"))
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
