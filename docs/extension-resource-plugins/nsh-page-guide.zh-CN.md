# Navop 扩展页面开发指南（nsh / TSX）

本文是扩展作者写 **UI 页面**的指南。扩展机制在这一轮重构成了 nsh 宿主：
页面不再是 gpui-shell 的 builder 链脚本，而是一个 nsh 应用（TSX），由宿主用
`AppView` 装载。能力平面（provider / resource / job / event / blob）没有变，
页面通过 `host.call("navop.*")` 调它。

参考实现（照抄它们最快的）：

```text
navop-extensions/extensions/composite/docker/ui/pages/volumes        # 列表 + 详情 + 写操作
navop-extensions/extensions/composite/docker/ui/pages/log-viewer     # 轮询 + 自动刷新
navop-extensions/extensions/composite/elasticsearch/ui/pages/mapping # 大表 + 过滤 + 复制
navop-extensions/examples/shell-view-example/ui/pages/tool           # 直接调能力平面
```

---

## 1. 一个页面是什么

```text
ui/pages/<page>/
  nsh.json        # nsh 应用身份 + 能力声明（构建输入）
  App.tsx         # 页面本体，结尾 mount(App)
  App.test.tsx    # headless 测试（nsh test）
  tsconfig.json   # extends ./.nsh/tsconfig.json，include 要含 ui/lib
  .gitignore      # 忽略 .nsh/ 与 target/，**dist/ 要入库**
  dist/           # nsh build 产物：宿主直接装它，运行时不跑构建
```

`extension.json` 里声明这个页面：

```json
{
  "engines": { "onetcli": ">=0.17.0", "nsh": "0.3.0" },
  "contributes": {
    "shellViews": [{
      "id": "overview",
      "title": "Docker Engine Overview",
      "entry": "ui/pages/overview/dist/bundle.js",
      "surface": "tab",
      "backends": { "docker": "main" },
      "modules": ["context", "workbench"]
    }]
  }
}
```

- `engines.nsh` 是**必需**的：没有它，`shellViews` 在清单校验阶段就被拒
  （`/engines/nsh`）。`engines.gpui_shell` 已经随旧 UI 平面一起删掉。
- `entry` 指向**构建产物**（`dist/bundle.js`），不是 TSX 源文件。
- `backends` / `modules` 语义与以前一致，见第 4 节。

## 2. 从零建一页

```bash
mkdir -p ui/pages/my-page && cd ui/pages/my-page
nsh init .                 # 生成 tsconfig.json / .gitignore / 示例 App.tsx + nsh.json
# 写 App.tsx、改 nsh.json
nsh check .                # 类型检查（TypeScript 7，不需要 node）
nsh build . --out dist     # 产出 dist/{bundle.js,bundle.js.map,bundle.nshc,app.json,capabilities.json}
nsh test .                 # headless 测试（mock host.call）
```

`nsh.json` 最小形态：

```json
{
  "name": "Docker 存储卷",
  "id": "com.navop.docker.volumes",
  "version": "0.1.0",
  "build": 1,
  "capabilities": { "host": ["navop.workbench.*"] }
}
```

`id` 用反域名，和 `extension.json` 的 `id` 是两回事（前者标识这个 nsh 应用，
后者标识整个扩展包）。

## 3. 页面能用什么宿主能力

页面看到的是 `host.call(name, payload)`（返回 Promise）。名字由页面的
`modules` 决定 —— 没声明的模块，宿主根本不注册对应 handler，调用会失败。

| 模块 | 命名空间 | 说明 |
|---|---|---|
| `context` | `navop.context.current` | 页面上下文：extensionId / viewId / backends / 借来的连接 |
| `log` | `navop.log.{debug,info,warn,error}` | 写进宿主日志（`{ message }`） |
| `i18n` | `navop.i18n.{locale,available}` | 宿主界面语言；变更会以 `navop.i18n.changed` 推送 |
| `theme` | `navop.theme.{snapshot,color}` | 当前主题（明暗 + 30 个 shadcn token）；`navop.theme.changed` 推送 |
| `resource` | `navop.resource.{open,invoke,ping,close}` | 原始资源句柄（tab 页可以用；**嵌入页被禁用**，见第 4 节） |
| `job` | `navop.job.{start,status,cancel,result,close}` | 长任务 |
| `event` | `navop.event.{open,read,close}` | 事件流（嵌入式页面暂时没有推送通道，轮询更实际） |
| `blob` | `navop.blob.{read,close}` | 大块数据分片读 |
| `runtime` | `navop.runtime.info` | provider 运行时信息 |
| `dev` | `navop.dev.*` | 开发者工具页专用（只有 navop 自己的 dev-tools 扩展用） |
| `workbench` | `navop.workbench.{current,dispatch}` | 资源工作台嵌入页：上下文 + 操作派发 |

**错误**：失败时 reject 的 message 形如
`人类可读消息\n__NAVOP_ERROR__<base64url(JSON)>`，解码出
`{ code, message, retryable, details }`，稳定码有
`INVALID_ARGUMENT` / `PERMISSION_DENIED` / `BACKEND_NOT_FOUND` /
`BACKEND_START_FAILED` / `RUNTIME_UNAVAILABLE` / `INVALID_HANDLE` /
`STALE_HANDLE` / `REQUEST_CANCELLED` / `REQUEST_TIMEOUT` 等。页面关闭时在途
调用一律 `REQUEST_CANCELLED`。

**取消**：没有单独的 cancel API。页面被关闭（tab 关掉 / 扩展停用）时，宿主
会取消所有在途调用并在有必要时补一次 close（open 了一半的资源会被回收）。

## 4. 两种页面形态

### 4.1 独立 tab（`surface: "tab"`，模块里没有 `workbench`）

宿主打开即装载，`navop.context.current` 的 `connection` 为 null。声明了
`backends` 的页面只有在连接页里打开时才会带连接（`surface` 不变，
`contributes.connections[].shellViewId` 指过来即可）。

### 4.2 资源工作台嵌入页（模块必须含 `context` + `workbench`）

从资源工作台里嵌进来，主资源由**外层连接会话**开好，页面借用：

- `navop.workbench.current` 给出 `{ pageId, route, selection, paging, connection }`；
- `navop.workbench.dispatch(operationId, input, { confirmed })` 派发
  `resourceWorkbenches[].operations` 里的操作；`confirmed: true` 表示用户已在
  页面上确认过（宿主据此弹/不弹二次确认）。

嵌入页的能力是**黑名单**：`resource` / `job` / `event` / `blob` / `runtime` /
`dev` 一律被拒（页面不该自己开资源），`i18n` / `theme` 放行。

## 5. 设计系统（别自己重造）

共享库在 `extensions/composite/docker/ui/lib/`，其它扩展用一行转发复用：

```ts
// ui/lib/theme.ts
export * from "../../../docker/ui/lib/theme";
```

- `theme.ts`：`c.*()`（bg/panel/text/muted/line/accent/danger/hover/chart(i)，
  返回主题 token）、`r`（圆角）、`t.*`（h1/h2/body/label/meta/mono 排版）、
  `panel()`、`hairline()`。
- `format.ts`：`humanBytes` / `percent` / `ratio` / `shortTime` /
  `splitImageRef` / `errorMessage` / `str` / `arr`。
- `ui.tsx`：`Stack` / `Row` / `Card` / `Badge` / `Button` / `SearchField` /
  `List` / `ListRow` / `Empty` / `Loading` / `LoadError` / `Stat` / `Meter` /
  `KeyValue` / `SectionTitle` / `SplitPane` / `ConfirmBar` / `Notice`。

规矩：颜色只走 `c.*()`（明暗主题自动跟随），数字用 `t.mono`
（tabular-nums，滚动不抖），文案简体中文，三态齐备（加载 / 失败可重试 /
正常；有数据后失败用 `Notice` 保留内容，不要清空）。

## 6. nsh/TSX 的坑（都是实测踩出来的）

1. **组件体只在挂载时执行一次**。所以三态判断不能写成顶层
   `if (loading()) return <Loading/>;` —— 会永久停在加载态。写成
   `const body = () => {...}`，根节点 `<View>{body()}</View>`。
2. **`onChange={sig.set}` 会抛 `not a live signal`**，必须
   `onChange={(text) => sig.set(text)}`。
3. `numberOfLines` / `ellipsizeMode` / `selectable` 是 `<Text>` 的 **prop**，
   写进 style 会报 TS2353。
4. `Input.onChange` 是 `(text: string) => void`；`onSubmit` 才是 `{ text }`。
5. 不要给组件或 `View` 传 `key=`；列表用 `.map()`；`Each` 不从 `"nsh"` 导出。
6. `fontVariant: ["tabular-nums"]` 只能出现在 `satisfies TextStyle` 的对象里
   （`as const` 会变成 readonly 而不兼容）。
7. 百分比宽度写 `` `${Math.round(p*100)}%` ``，`.toFixed()` 出来的是
   `${string}%`，不满足 `DimensionValue`。
8. 主题对象字面量里的颜色**不能**在模块加载时求值（会冻住），要用 getter；
   `theme.ts` 里已经这么做了，页面自己别写 `color: c.text()` 的常量。
9. 相对导入层级要对：`ui/pages/<page>/App.tsx` → `../../lib/ui`；
   `tsconfig.json` 的 include 要含 `../../lib/*.ts`、`../../lib/*.tsx`。
10. 设计系统组件的标量 prop（如 `disabled`、`tone`）在某些组合下会停留在
    构造时的值；需要动态时要么在 JSX 里传 Accessor，要么用函数子重建。

## 7. 测试

`nsh test .` 跑 `*.test.tsx`，宿主能力用 mock 给：

```tsx
import { expect, open, test } from "nsh/test";

test("列表与详情", async () => {
  const app = await open("./App.tsx", {
    width: 1100,
    height: 800,
    host: { handlers: { "navop.workbench.dispatch": (data) => /* 真数据形状 */ ({}) } },
  });
  await app.waitFor("text:存储卷");
  await app.press("text:app-data");
});
```

建议每条页面至少覆盖：列表渲染、选中后详情、写操作的 `confirmed` 载荷、
失败重试。

## 8. 交付清单

- `nsh check .` 零错误（故意插一个类型错误确认它真会报，别信静默成功）；
- `nsh build . --out dist` 退出 0，`dist/` 一并入库；
- `nsh test .` 全绿；
- `extension.json`：`engines.nsh`、`entry` 指到 `dist/bundle.js`、
  `modules` 与页面真实用到的命名空间一致；
- `nsh.json` 的 `capabilities.host` 只列真用到的命名空间（多给等于多授权）。
