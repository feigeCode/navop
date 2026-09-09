# Architecture

```text
extension.json
    |
    v
ExtensionRuntimeCatalog
    |
    v
ActivationManager ---- RuntimeMonitor
    |
    v
ProcessRpcSession
    |
    +---- resource
    +---- job
    +---- event stream
    +---- blob
```

## Ownership

- `ExtensionRuntimeCatalog` owns validated static runtime metadata.
- `ActivationManager` owns runtime activation leases, process generations and shutdown.
- `RuntimeMonitor` owns health polling and bounded restart decisions.
- `UniversalProviderHost` owns reverse host capabilities such as secrets and host blob upload.
- `BlobStore`, `JobActivationManager` and `EventActivationManager` own generation-scoped state.

## Activation contract

`activate_runtime(runtime_id)` is the only process activation entry. Multiple callers may acquire
independent leases for the same runtime; one process session is shared. Releasing a stale lease is
an idempotent no-op. The last live lease shuts the runtime down.

## UI boundary

The runtime layer has no UI model. Declarative panels, ViewSpec/UiNode, WIT open-view and provider
UI RPC have been removed. gpui-shell will be integrated above this layer and will consume runtime
services through a host-owned adapter.
