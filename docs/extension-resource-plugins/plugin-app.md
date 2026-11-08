# 一个插件一个 nsh app（plugin app 契约）

> 2026-11-04。取代「一个页面一个 nsh 包」的过渡形态；与
> [`nsh-migration-plan.md`](nsh-migration-plan.md) §5（一包一 app）和 §15（一个插件
> 一个 tab 视图）对齐。页面作者看 [`nsh-page-guide.zh-CN.md`](nsh-page-guide.zh-CN.md)，
> 宿主 ABI 看 [`host-abi-v2.md`](host-abi-v2.md)。

## 1. 决定

- **一个扩展 = 一个 nsh app = 一个包**：一个扩展 id、一份 `ui/dist/bundle.js`、
  一个 `build`、一个版本号；不再一页一个 app id。
- **app 内用路由组织页面**：页面是 app 里的普通 TSX 组件，由 app 自己的
  `App.tsx` 画外壳（侧栏/tab 栏）并按 route 渲染；宿主不解释页面结构。
- **宿主一个 app 一个 tab**：连接型按 `extension-connection:{connectionId}`，
  非连接型按 `nsh:app:{extensionId}`（单例）。app 内部切路由不新建宿主 tab。
- **宿主只传启动事实**：`host.params` 里多一个 `route`（连接清单声明的初始路由）；
  之后的路由变化完全在 app 内。

这样做的收益：一个包/一个版本/一次构建/一个引擎 VM；页面间共享连接与状态；
插件作者可以自由决定导航形态（侧栏、tab、NavStack 都行）。

## 2. 清单

nsh 扩展的 `extension.json` 用 `contributes.shellApp` 声明 app，不再用
`shellViews`：

```json
{
  "contributes": {
    "shellApp": {
      "title": "Docker",
      "description": "Docker 容器管理",
      "icon": "icons/docker-color.svg",
      "entry": "ui/dist/bundle.js",
      "category": "container",
      "keywords": ["docker", "container"],
      "backends": { "docker": "main" },
      "modules": ["resource"]
    },
    "connections": [
      {
        "id": "docker-local",
        "label": "Docker",
        "runtimeId": "main",
        "resourceType": "docker",
        "route": "containers",
        "form": { "...": "..." }
      }
    ]
  }
}
```

规则：

- `shellApp` 最多一个；有 nsh 扩展就必须有（有 `engines.nsh` 的扩展 = nsh 扩展）。
- `title` 缺省取 `manifest.name`；`entry`、`backends`、`modules` 语义与旧
  `shellViews` 相同（backends 引用 `runtime.ipc[].id`；别名 `host`/`navop`/`default`
  禁用；`shell:exec` 权限仍是 nsh 扩展的硬要求）。
- 没有 `surface`、`singleton`、`id`：一个扩展只有一个 app，工具箱入口按
  「有连接 → 由连接打开；无连接 → 工具箱」决定，不再有 toolbox 卡片页。
- `connections[].route` 取代 `shellViewId`：可选、非空字符串、app 自己解释。
  连接打开时宿主把它放进 `host.params.route`。非 terminal 连接仍要求
  `runtimeId`/`resourceType`，且 app 必须声明 `resource` 模块。
- `shellViews` 键删除（旧清单出现该键会被 `deny_unknown_fields` 拒绝，报错文案
  指向本节）。

## 3. host.params

```ts
{
  apiVersion: 2,
  extension: { id: "com.navop.docker", title: "Docker" },
  route: "containers" | null,      // 连接清单声明的初始路由；工具箱打开为 null
  backends: ["docker"],            // 已激活的 runtime alias
  connection: { ... } | null       // 非连接型为 null
}
```

- 同步可读、首帧前写入、生命周期内不变（沿用 ABI v2）。
- `route` 只在打开时给一次；宿主不提供「切换路由」的推送。app 内导航自己维护。
- 删除 `extension.viewId` / `extension.viewKey`（页面不再有 view 身份）。

## 4. 宿主行为

- **注册**：`register_shell_apps` 按 extension id 注册 `RegisteredShellAppContribution`；
  `catalog.shell_apps()`；`toolbox_apps()` 只列没有连接声明的 app。
- **打开**：`open_shell_app(extension_id, route, window, cx)`；连接打开时
  `route = connection.route`，没有就 `null`。已存在的 tab 直接激活（新 route 不
  重新下发）。
- **tab**：标题 = 连接名（连接型）或 app title；`content_key` 不变；runtime 变更
  的处理沿用 `decision.rs`（连接型 fail、非连接型 remount）。
- **权限**：`granted` 由 app 的 `modules` + `permissions` 派生；`allow_process`
  仍是清单里有 `shell:exec`。
- **原生组件**：`NativeViewContext` 的页面身份从 `view_id` 变为
  `route: Option<String>`，组件名注册方式不变。
- **开发工具**：`navop.dev.openView` 改为 `navop.dev.openApp(extensionId, route?)`；
  内置 devtools 扩展同样声明 `shellApp`（入口 `ui/dist/bundle.js`）。
- **扩展管理器**：`ExtensionSummary.shell_views` 改为 `shell_app: Option<ShellAppSummary>`。

## 5. 工程结构

```
extensions/composite/<ext>/
  extension.json                 # shellApp + connections[].route
  ui/
    App.tsx                      # 入口：路由表 + 外壳，结尾 mount(App)
    nsh.json                     # id = 扩展 id，name/version/build
    tsconfig.json                # extends ./.nsh/tsconfig.json，include 页面与 lib
    lib/                         # @navop/extension-kit 的转发层（navop/theme/format/ui）+ 测试启动事实
    package.json                 # 依赖 @navop/extension-kit（workspace 或公共 npm）
    pages/<page>/App.tsx         # 页面组件（export default，不再 mount）
    pages/<page>/App.test.tsx    # 页面测试（打开外壳入口 + params.route）
    dist/                        # nsh build 产物，入库
```

- 页面目录里不再有 `nsh.json`/`tsconfig.json`/`dist/`/`.nsh/`。
- 构建：`cd ui && nsh build . --out dist`（一个 bundle，含所有被 import 的页面）。
- 测试：`cd ui && nsh test pages/<page>`（逐页，headless），或按页循环。
  页面测试打开**外壳入口**并指定路由：
  `open("../../App.tsx", { host: { params: { ...TEST_PARAMS, route: "<page>" }, handlers }, clock: {} })`。
  这样零新增入口文件，且顺带覆盖「`params.route` → 首屏」这一段。
- nsh 0.3.0 的两个硬约束：`open(entry)` 不会自动挂载默认导出，入口模块必须
  自己 `mount()`（否则报 `the app rendered nothing in 5 s`）；nsh 只读**入口
  所在目录**的 `nsh.json`（不向上查找），所以 `ui/nsh.json` 必须带
  `capabilities.host: true`，测试目录不要再放 nsh.json。
- `params.backends` 是 `shellApp.backends` 的别名 keys（如 `["docker"]`、`["resource"]`），
  不是 `runtime.ipc` 的 runtime id。
- `dist/app.json` 的 id 必须等于扩展 id（宿主按它认 app）。
- 新扩展用 `npx @navop/create-extension new --id … --name …`（仓库内用
  `node scripts/create-extension.mjs`，模板 `templates/extension-basic`）；
  离线 / 内网用 `bash scripts/create-extension-offline.sh --id … --name …`：
  它把 kit 源码复制进 `ui/lib`，**不需要 npm install**；整套离线包用
  `bash scripts/pack-offline.sh` 生成。

## 6. app 外壳约定（各插件保持一致）

- 左侧 208px 侧栏：顶部 app 标题，下面是路由项（图标 + 标题，当前项高亮），
  键盘 Tab 可聚焦、Enter 切换；内容区 `flex_1 min_w_0 min_h_0`。
- 路由表里的 route id 沿用旧 view id（`containers`/`images`/`log-viewer`/…），
  迁移期老入口和连接 `route` 不用改语义。
- 启动：`route = params.route ?? 默认路由`；未知 route 回落默认路由。
- 没有连接时（声明了连接但 `params.connection` 为 null）：内容区显示可读空态
  「请先打开一个 <连接名> 连接」，不渲染页面、不发起 invoke。
- 只有一条路由的 app（dev-tools）不画侧栏，直接渲染页面。
- 路由切换保留外壳与连接；页面组件按需挂载（未访问的路由不执行初始化）。

## 7. 迁移步骤

1. 宿主：清单类型/校验/注册/catalog、extension_view 打开接口、nsh_host 的
   tab/params/原生组件、navop_app 的 toolbox/dev/setting/连接表单调用点。
2. 插件：`extension.json` 改 `shellApp` + `connections[].route`；`ui/` 提为 app
   工程（App.tsx/nsh.json/tsconfig.json/dist）；页面去掉 `mount` 并默认导出；
   外壳按 §6 实现；删除每页 `nsh.json`/`tsconfig.json`/`dist`/`.nsh`。
3. 文档：本文件、`nsh-page-guide`、`host-abi-v2`、`nsh-migration-plan` §18；
   `navop-extensions/packages/navop-extension-types` 的清单类型与 JSON schema。

## 8. 验证

- 宿主：`cargo test -p extension-runtime`、`cargo test -p universal-plugins --features nsh-host`、
  `cargo test -p extension-view`、`cargo test -p navop_app`；`cargo clippy -- -D warnings`。
- 插件：每个 app `nsh build . --out dist` 成功且 `dist/app.json` id 正确；
  逐页 `nsh test pages/<page>` 全绿；`nsh check .` 无错。
- 端到端：本地安装 extensions，打开连接 tab / 工具箱卡片，确认只出现一个
  app tab、路由可切换、资源调用与原生终端正常。
