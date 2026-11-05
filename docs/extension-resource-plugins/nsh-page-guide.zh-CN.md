# Navop 扩展页面开发指南（nsh / TSX）

本文是扩展作者写 **UI 页面**的指南。扩展机制在这一轮重构成了 nsh 宿主：
页面不再是 gpui-shell 的 builder 链脚本，而是一个 nsh 应用（TSX），由宿主用
`AppView` 装载。能力平面（provider / resource / job / event / blob）没有变，
但页面与宿主的四类通道各归其位（ABI v2，契约见 `host-abi-v2.md`）：

| 通道 | 用途 | 典型调用 |
|---|---|---|
| `host.params` | **启动事实**，首帧前写死，同步读 | `host.params.connection` |
| `host.on(name, fn)` | **事实变化**，宿主推送 | `host.on("navop.event", fn)` |
| `host.call(name, data, {timeout})` | **命令**，有应答、可失败、带 `e.code` | `host.call("navop.resource.invoke", …)` |
| `host.post(name, data)` | **通知**，单向、无应答 | `host.post("navop.log", …)` |

v1 的 `navop.context.current` 启动 RPC、`navop.theme.*`、`navop.i18n.*`、
`navop.log.*`（call 形态）、`navop.event.read` 轮询、以及 `__NAVOP_ERROR__`
base64 信封**全部删除**，没有兼容层。

参考实现（照抄它们最快的；每一项都是某个 app 内的一个路由组件）：

```text
navop-extensions/extensions/composite/docker/ui/pages/volumes        # 列表 + 详情 + 写操作
navop-extensions/extensions/composite/docker/ui/pages/log-viewer     # 轮询 + 自动刷新
navop-extensions/extensions/composite/elasticsearch/ui/pages/mapping # 大表 + 过滤 + 复制
navop-extensions/examples/shell-view-example/ui/pages/tool           # 直接调能力平面
```

---

## 1. 一个扩展 app 是什么

宿主装载的单位是**扩展 app**，不是一个页面：一个扩展 = 一个 nsh app = 一个
`dist/` 包。页面是 app 内部的路由组件，不再各自有 `nsh.json` / `dist/`。

```text
ui/
  App.tsx        # 应用入口：外壳（侧栏/导航）+ 路由表，结尾 mount(App)
  App.test.tsx   # 外壳测试（可选，但推荐）
  nsh.json       # nsh app 身份 + capabilities.host（构建输入）
  tsconfig.json  # extends ./.nsh/tsconfig.json，include 要含 lib 与 pages
  .gitignore     # 忽略 .nsh/ 与 target/，**dist/ 要入库**
  dist/          # nsh build 产物：宿主直接装它，运行时不跑构建
  lib/           # 共享库（host 调用封装、组件、格式化）
  pages/<page>/  # 普通 TSX 模块：export default 页面组件，不 mount、没有自己的 nsh.json
```

`extension.json` 里声明这个 app：

```json
{
  "engines": { "onetcli": ">=0.17.0", "nsh": "0.3.0" },
  "contributes": {
    "shellApp": {
      "title": "Docker",
      "entry": "ui/dist/bundle.js",
      "backends": { "docker": "main" },
      "modules": ["resource"],
      "category": "container",
      "keywords": ["docker"]
    },
    "connections": [{
      "id": "docker-local",
      "label": "Docker",
      "runtimeId": "main",
      "resourceType": "docker",
      "route": "containers",
      "form": { "tabs": [] }
    }]
  }
}
```

- `engines.nsh` 是**必需**的：没有它，`shellApp` 在清单校验阶段就被拒
  （`/engines/nsh`）。`engines.gpui_shell` 已经随旧 UI 平面一起删掉。
- `entry` 指向**构建产物**（`dist/bundle.js`），不是 TSX 源文件。
- `backends` / `modules` 语义见第 4 节：`modules` 是**唯一的授权源**。
- `connections[].route` 是这个连接打开 app 时的**初始路由**；工具箱打开时
  `route` 为 `null`，由 app 自己决定默认页。
- 一个扩展只能有一个 `shellApp`、一个 `build` 号；页面增减不再改版本号。

## 2. 从零建一个 app

```bash
mkdir -p ui/pages/my-page && cd ui
nsh init .                 # 生成 tsconfig.json / .gitignore / 示例入口 + nsh.json
# 写 App.tsx（外壳 + 路由）、pages/my-page/App.tsx（页面组件）
nsh check .                # 类型检查（TypeScript 7，不需要 node）
nsh build . --out dist     # 产出 dist/{bundle.js,bundle.js.map,bundle.nshc,app.json,capabilities.json}
nsh test .                 # headless 测试（mock host.call）
```

`ui/nsh.json` 最小形态：

```json
{
  "name": "Docker",
  "id": "com.navop.docker",
  "version": "0.6.3",
  "build": 1,
  "capabilities": { "host": true }
}
```

`"capabilities": { "host": true }` 的意思是「这个 app 的 host 调用交给宿主裁决」，
不是「什么都能调」：宿主安装时把 declared 覆盖成 `extension.json` 的
`modules` + `permissions` 派生出来的那一份，app 能调到什么由 `modules` 决定。

这么写有两个实际原因：一是 nsh build 会因为「导入了 `host` 却没声明
`capabilities.host`」报一条 warn（它假设页面自己声明能调什么），对 navop
app 是误报；二是没有宿主的地方（`nsh test`、`nsh preview`）会按「没声明」
把调用全部拒掉，app 就没法独立跑。

**不要在这里列具体名字**（比如 `["navop.resource.*"]`）：那会和 `extension.json`
的 `modules` 形成第二份需要人工同步的清单，v2 就是要消掉它。
`id` 是 nsh app id，建议**直接等于扩展 id**（一插件一包）；它和
`extension.json` 的 `id` 此时相同，`build` 每次发布递增即可。

### 路由

`App.tsx` 读 `host.params.route` 选根页面（一般用侧栏 + 内容区），未知或
`null` 回落默认路由；页面之间的切换只改 app 内状态，不再经过宿主。需要深链
时把 route 设计成 `"containers/<id>"` 这类可解析的字符串即可——宿主只负责把
它原样透传。

### nsh 只读入口目录的 `nsh.json`

`nsh check/build/test` 只会读**入口所在目录**的 `nsh.json`，不向上找。所以
不要为了测试把入口放到别的目录再补一份 `nsh.json`；直接 `open("../../App.tsx")`
打开 app 入口（见第 8 节）。

## 3. 页面能用什么宿主能力

| 通道 | 名字 | 说明 |
|---|---|---|
| `host.params` | （对象） | 启动事实：`apiVersion` / `extension{id,viewId,viewKey,title}` / `backends` / `connection` |
| `host.call` | `navop.resource.{open,invoke,ping,close}` | 资源句柄。`invoke` 是页面读写数据的主要方式 |
| `host.call` | `navop.job.{start,status,cancel,result,close}` | 长任务 |
| `host.call` | `navop.blob.read` / `navop.blob.close` | 大块数据分片读 |
| `host.call` | `navop.event.{subscribe,unsubscribe}` | 订阅事件流；随后由 `host.on("navop.event")` 推送 |
| `host.call` | `navop.runtime.info` | provider 运行时信息 |
| `host.call` | `navop.dev.*` | 只有 navop 内置的 dev-tools 扩展能用，第三方扩展调不到 |
| `host.on` | `navop.event` | 事件批次 `{ handle, cursor, events, done?, error? }`，单批最多 64 条，溢出时带 `EVENT_OVERFLOW` 并可用 `since` 续订 |
| `host.post` | `navop.log` | `{ level, message, fields }`，写进宿主日志，无应答 |

没在 `modules` 里声明的命名空间，宿主**也注册了** handler，只是在调用时回
`PERMISSION_DENIED`，`details.module` 指出缺的是哪个模块（`resource` / `job` …）。
调用名本身写错才会拿到 nsh 的 `ERR_NSH_HOST_ERROR`（「宿主没有这个 handler」）。
两者因此可区分：`PERMISSION_DENIED` → 去 `extension.json` 补 `modules`；
`ERR_NSH_HOST_ERROR` → 名字拼错了。

**错误**：reject 的对象是 nsh 的 `JsError`，直接读字段：

```ts
try {
  await invoke("docker/volume/list", {});
} catch (e) {
  e.code;       // "INVALID_HANDLE" / "STALE_HANDLE" / "REQUEST_TIMEOUT" / …
  e.retryable;  // boolean
  e.details;    // 可选，结构化补充（如 { limitBytes })
  e.message;    // 人类可读，不要用来分支
}
```

稳定码：`INVALID_ARGUMENT` / `PERMISSION_DENIED` / `BACKEND_NOT_FOUND` /
`BACKEND_START_FAILED` / `RUNTIME_UNAVAILABLE` / `INVALID_HANDLE` /
`STALE_HANDLE` / `REQUEST_CANCELLED` / `REQUEST_TIMEOUT` / `RESULT_TOO_LARGE` /
`PROTOCOL_ERROR` / `EXTENSION_UNLOADED`。页面关闭时在途调用一律
`REQUEST_CANCELLED`。

**超时**：`host.call` 第三参数 `{ timeout }`（毫秒），不传时 nsh 默认 30 秒，
`0` 表示不限。跑得久的 `invoke`（建连、跑查询）自己给一个明确的超时，别让
页面在 30 秒被 `ETIMEDOUT` 打断而宿主还在跑。

**语言与明暗**：都走 nsh 原生，不需要宿主推送。
- 语言：`import { locale } from "nsh"`（`locale()` 是 accessor，随 nsh 的 surface
  环境变化）。宿主在首帧之前就 `set_locale` 成 navop 的语言设置，所以
  `locale()` 就是用户在设置里选的那个，冷启动也一样。
- 明暗：`import { colorScheme } from "nsh"`，以及 `theme.ts` 的 `c.*()` /
  `token()` 返回的主题 token —— 它们自动跟随系统/宿主的 appearance，页面不
  需要订阅任何 `theme.changed` 事件，也不要自己存一份颜色常量。

**取消**：仍然没有单独的 cancel API。页面被关闭（tab 关掉 / 扩展停用）时，
宿主会取消所有在途调用并在有必要时补一次 close（open 了一半的资源会被回收）。

## 4. 页面怎么拿数据

宿主只给页面两条通道：

1. `host.params`（同步，首帧就有）：
   ```ts
   {
     apiVersion: 2,
     extension: { id, viewId, viewKey, title },
     backends: ["main"],
     connection: null | {
       connectionId, name, contributionId, resourceType,
       resourceHandle,        // 借来的资源句柄
       alias,                 // 这个句柄在页面 backends 里的别名
       config,                // 连接的非敏感配置（secrets 不会出现）
     },
   }
   ```
   不是从连接打开的页面（例如工具箱里直接打开的页面）`connection` 为 null。
   读不到 `host.params` 属于宿主 bug，不是页面要去兜底的情况。
2. `navop.resource.invoke` —— 按 provider 的方法名调一次：
   ```ts
   await host.call("navop.resource.invoke", {
     handle: connection?.resourceHandle,
     method: "docker/volume/list",
     params: {},
   }, { timeout: 60_000 });
   ```

共享库 `ui/lib/navop.ts` 把这两步包成一行：

```ts
import { invoke } from "../../lib/navop";

const volumes = await invoke<{ volumes: Volume[] }>("docker/volume/list", {});
```

`modules` 的合法值只有 `resource` / `job` / `event` / `blob` / `runtime` /
`log` / `dev`；声明 `resource` / `job` / `event` / `blob` 的页面必须有
`backends`。**声明了就代表你授权了这个命名空间**，页面没用到的不声明。

**没有第二套"声明式 UI"机制了**：没有工作台描述、没有 operation 派发、
没有宿主注入的 route/selection。页面要选容器/选索引/选主题，就自己拉列表、
自己放选择器——宿主不再替页面决定"当前对象是什么"。

方法名去哪找：provider 的 `src/main.rs`（或扩展的 README）里有
`match method { "docker/volume/list" => ... }` 这样的大表；旧清单里
`resourceWorkbenches[].operations` 也写着 operationId → method 的对应关系，
但那份声明已经不再参与运行。

## 5. 原生组件（`nativeComponent`）

TSX 画不了、但必须是 Rust 实体的东西（终端、编辑器、图表面板）走原生组件：
宿主注册一个名字，页面用 `nativeComponent` 声明，然后像普通元素一样摆位置。

```tsx
import { nativeComponent } from "nsh";

const Terminal = nativeComponent<{ command: string; args: string[] }>("navop.terminal", {
  props: ["command", "args", "env", "title"],
});

<Terminal command="docker" args={["exec", "-it", container.name, "/bin/sh"]} />
```

第二个参数是**声明面**：`props` 里列出的名字才会被转发给 Rust；
`events` 里列出的名字才会回调页面（`onXxx`）。不列 = 不传。

### 目前可用的原生组件

| 名字 | props | 说明 |
|---|---|---|
| `navop.terminal` | `command`（必填）、`args`、`env`、`cwd`、`title` | 宿主本地终端（PTY）。改 `command`/`args`/`env`/`cwd` 会**重开**一个 pty；只改 `title` 不会 |

### 权限与边界

- `navop.terminal` 会起**宿主本地进程**，因此扩展清单（`extension.json` 的
  `permissions`）必须声明 `shell:exec`；没声明时组件渲染一行错误，不会静默失败。
- 终端进程不受 provider 沙箱约束：**命令与参数必须由页面显式给出**，不要做成
  "用户随手输入一条命令"的形态（那就等于给了扩展一个任意命令执行入口）。
- 组件的键盘焦点由宿主接管，Tab 会走进终端；页面不需要也不想管焦点。
- 页面关掉时宿主动手回收 pty（组件 drop 后主线程补 `shutdown_embedded`）。

### 什么时候**不**该用

- 需要"发请求拿数据"的：那是能力平面（`host.call("navop.resource.*")`）。
- 需要 provider 侧执行的：扩展协议只有请求-响应与 job 两种形态，没有流式 pty，
  所以 `navop.terminal` 的 `operation` 参数目前直接报错，别用。

## 6. 设计系统（别自己重造）

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

## 7. nsh/TSX 的坑（都是实测踩出来的）

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
10. **不要在组件体里把 props 算成常量**。组件体只跑一次，算出来的就是挂载
    值；只有 JSX 属性表达式与子节点绑定按帧求值。反例（会让按钮永久点不动、
    进度条不动、空态不消失）：
    ```tsx
    const disabled = props.disabled === true;
    return <View onPress={disabled ? undefined : props.onPress} style={{ opacity: disabled ? 0.5 : 1 }} />;
    ```
    正解是全部内联：`onPress={props.disabled === true ? undefined : props.onPress}`、
    `style={{ opacity: props.disabled ? 0.5 : 1 }}`。需要动态能力时让 prop 接受
    取值函数（`disabled?: boolean | (() => boolean)`）。空态这类**结构**分支也别
    用体级 `if (...) return ...`，写成 `<If when={...}>` 或渲染函数。

## 8. 测试

`nsh test .` 跑 `*.test.tsx`，启动事实用 `params` 给，宿主能力用 mock 给：

```tsx
import { expect, open, test } from "nsh/test";
import { TEST_PARAMS, hostHandlers } from "../../../lib/test-host";

// 从 ui/pages/<page>/App.test.tsx 出发：打开 app 入口，用 route 选中本页。
const PARAMS = { ...TEST_PARAMS, route: "volumes" };

test("列表与详情", async () => {
  const app = await open("../../App.tsx", {
    width: 1200,
    height: 800,                       // nsh 默认视口 390，带侧栏的 app 要显式给宽高
    host: { params: PARAMS, handlers: hostHandlers({ "docker/volume/list": () => /* 真数据形状 */ ({}) }) },
  });
  await app.waitFor("text:存储卷");
  await app.press("text:app-data");
});
```

`TEST_PARAMS` / `TEST_PARAMS.route` 与真实宿主写入的 `host.params` 同形（含
`apiVersion: 2`、`extension`、`route`、`backends`、`connection`），测试里不要
另造一套字段名：打开 app 入口、传 route，是唯一一条同时覆盖「路由 → 页面」的路径。

建议每条页面至少覆盖：列表渲染、选中后详情、写操作（是否带 `id`）、失败重试。

## 9. 交付清单

- `nsh check .` 零错误（故意插一个类型错误确认它真会报，别信静默成功）；
- `nsh build . --out dist` 退出 0，`dist/` 一并入库；
- `nsh test .` 全绿（页面测试从页面目录 `open("../../App.tsx")` 并传 `route`）；
- `extension.json`：`engines.nsh`、`shellApp.entry` 指到 `dist/bundle.js`、
  `modules` 与 app 真实用到的命名空间一致（`resource`/`job`/`event`/`blob`/
  `runtime`/`log`/`dev`，没有 `context`/`theme`/`i18n` 了）、连接用 `route` 而不是旧 `shellViewId`；
- 页面目录里没有 `nsh.json` / `tsconfig.json` / `dist/`（它们只属于 app 根）；
- `ui/nsh.json` 的 `capabilities.host` 是 `true`（交给宿主裁决，不要列具体名字）。
