# Universal Resource Plugins

- [扩展页面开发指南（nsh / TSX）](nsh-page-guide.zh-CN.md) —— **当前**，写 UI 页面看这篇
- [nsh 迁移方案](nsh-migration-plan.md) —— 这轮重构的方案与实施记录
- [通用资源扩展开发指南](development-guide.zh-CN.md) —— provider / runtime / 清单机制（仍然有效）
- ~~[gpui-shell 扩展设计](gpui-shell-extension-design.md)~~ —— 已废弃：UI 平面整体换成 nsh

当前底层机制保持 headless；UI 平面已从 gpui-shell 换成 **nsh**（`AppView` +
TSX 页面，签名包/能力模型沿用 nsh 的）：


- extension.json 声明 native IPC runtime；
- 宿主按 namespaced runtime id 惰性启动和监督 provider；
- provider 通过稳定 RPC 提供 resource、job、event stream、blob 能力；
- 权限、进程生命周期、重启代数和 host blob 均由宿主管理；
- shell view 是一个 nsh 应用（`ui/pages/<page>/dist/bundle.js`），宿主用
  `AppView` 装载；页面通过 `host.call("navop.*")` 打能力平面；provider 仍不返回
  UI tree。

扩展连接复用标准 `StoredConnection`、工作区、首页和侧边栏生命周期。
`contributes.connections` 声明宿主表单、IPC runtime 和可选 `shellViewId`：有 UI 时打开
connection-scoped shell tab，无 UI 时打开宿主 headless 状态 tab。job/event/blob、热更新
drain 和 permission facade 均复用同一 provider runtime。

UI 统一由 nsh 承载：宿主侧是 `crates/universal-plugins/src/nsh_host/`（页面装载、
租约、host ABI、嵌入页、运行监视），扩展侧是 TSX 页面 + 共享设计系统。

## Runtime 生命周期

1. `ExtensionRuntimeCatalog` 解析 `runtime.ipc`。
2. `ActivationManager::activate_runtime` 启动或复用 runtime，并返回 activation lease。
3. 调用方通过 `ManagedUniversalPluginClient` 使用 resource/job/event/blob 方法。
4. 调用方释放 lease；最后一个 lease 释放后 provider 关闭。
5. `RuntimeMonitor` 检查健康状态并按 manifest 策略执行有限重启。

## 当前边界

- 不解析 `contributes.declarativePanels`。
- 不提供 `ui/action`、`ui/dialog`、`ui/window`。
- 不提供 WASM `open-view` 或 `handle-view-action`。
- DB tree extension action 仍可执行无 UI WASM action。
- `extension_view` 继续作为 Navop 的扩展安装/管理页面，与插件 UI 无关。

## 页面接入点

页面怎么写在 [`nsh-page-guide.zh-CN.md`](nsh-page-guide.zh-CN.md)；宿主怎么装、
怎么派发能力的实现在 `crates/universal-plugins/src/nsh_host/`。旧文档
[`gpui-shell-extension-design.md`](gpui-shell-extension-design.md) 仅供追溯。

后续 `ShellPluginHost` 负责：

- 根据 extension.json 中的新 shell contribution 定位脚本入口；
- 先注册 `navop.*` HostModule，再实例化脚本视图；
- 通过 `UniversalPluginService::activate_runtime` 获取 provider lease；
- 把 resource/job/event/blob/db capability 暴露给脚本；
- unload 时释放 host module、脚本 view 和 activation lease。
