# Declarative UI Standalone v1

`declarative-ui-demo` 是一个独立于 Navop 扩展系统的 **Rust + GPUI
声明式 UI 运行时**。当前版本已经从“验证 HTML 能否渲染”的 PoC 收敛为一套有明确
契约、错误边界和资源边界的 standalone v1：

```text
受限 HTML
  ↓ html5ever / markup5ever_rcdom
VNode（与输入格式无关的中间层）
  ↓ 编译期校验 + typed diagnostics
CompiledTemplate
  ↓ Runtime state binding
Resolved VNode
  ↓ VNode diff + transactional patch
Stored VNode
  ↓ Component Registry + Tailwind utility
GPUI Element / stateful Entity
```

它不依赖 Navop extension catalog、WASM host、权限系统或插件生命周期，可以单独
运行和测试。这样可以先把 DSL、状态、事件、组件和渲染机制稳定下来，再设计正式的
扩展 ABI。

## v1 已提供的机制

- 使用 `html5ever` 解析 HTML fragment，并转换为自定义 `VNode`；
- 只接受 Tailwind utility class，不解析 `style` 或 CSS；
- strict / permissive 两种模板编译模式；
- 编译期受限 HTML `<input type>` adapter：文本类映射原生 `Input`，并把
  `checkbox` / `radio` / `range` / button-family 规范化到已有原生 DSL 组件；
- source、节点、深度、属性和 class token 的硬资源限制；
- 分阶段、分严重级别、带 VNode path 的 typed diagnostics；
- 默认 HTML 标签和可扩展的 `ComponentRegistry`；
- 自定义组件的 `Result` 错误边界和 panic boundary；
- 基于 `Entity<Runtime>` 的响应式状态、Action 和事件；
- Action transaction、失败回滚和 panic 回滚；
- 文本绑定、attribute binding、`input` / `textarea` 文本双向绑定、
  `checkbox` / `switch` / `radio` 布尔双向绑定，以及单值 `Slider` 数值双向绑定；
- 基于 `key`、`id` 或 VNode path 的有状态 `InputState` / `SliderState`
  identity 和 cache 生命周期；
- old/new resolved VNode diff 和事务式 patch；
- 原生 `gpui-component` Form、Table、ListItem、表单控件和反馈组件映射；
- 原生 `Avatar` / `AvatarGroup` / `DescriptionList` display 映射，以及
  `Breadcrumb` / `Pagination` / `Rating` / `TabBar` / `Stepper` navigation
  映射；
- 原生 `Kbd` 快捷键展示和缓存 `Entity<SliderState>` 的单值 `Slider` 映射；
- 原生 controlled `Accordion`，使用 canonical JSON `open-indices` binding，并保证
  Action 在 binding 写回之后派发；
- 原生 controlled `Collapsible`，以及使用稳定声明式 identity 复用 window-keyed
  resize state 的 `ResizablePanelGroup` / `ResizablePanel`；
- 原生 `ScrollHandle` + `Scrollbar` 容器，要求显式 stable `id`，支持轴向、显示模式
  和有限正像素 viewport 尺寸校验；
- `Pagination`、`Rating`、`Tabs`、`Stepper` 的数值 attribute binding、点击写回
  和结构化 Action；
- `SliderEvent::Change` 连续写回 binding，`SliderEvent::Release` 在最终写回后派发
  可选结构化 Action；
- 真实、可滚动、可交互的 GPUI showcase，以及 parser、compiler、runtime、
  binding、component boundary、limits、Tailwind、input 和 diff 契约测试。

## 运行与验证

在仓库根目录运行：

```bash
cargo run -p declarative-ui-demo
```

Demo 展示：

- strict 模式编译受限 HTML；
- 固定 header 与使用 `overflow-y-scroll` 的可滚动内容区，以及带原生 scrollbar
  overlay 的 `<scroll>` viewport；
- 两列原生 `Form` / `Field`，以及 username、email、notes 双向文本绑定；
- `<input type="password" readonly>` 映射的 masked Release token；
- `<input type="checkbox">`、原生 `Switch`、`<input type="radio">` 的字符串布尔
  状态双向绑定；
- bound `Alert`、`Progress`、`Badge`，以及原生 `Spinner`、`Skeleton`、
  `Separator`；
- 由原生 table primitives 组成的完整静态表格，cell 内嵌 `Tag` 和 `Button`；
- 静态声明式列表容器和可交互原生 `ListItem` rows；
- `Avatar` / `AvatarGroup`、强结构 `DescriptionList`，以及 action-only
  `Breadcrumb`；
- 原生 `Kbd` 快捷键展示；
- bound `Pagination`、`Rating`、`<input type="range">`、`Tabs`、`Stepper`；
  选择变化先写回 state，再让 `selection-changed` handler 读取新值；range adapter
  使用单值 Slider，在拖动时连续写回，release 时派发 Action；
- bound `Accordion`，支持同时展开多项、canonical JSON 写回，并让
  `accordion-changed` handler 读取已经提交的新 binding；
- 由 Runtime Action 控制 `open` binding 的 `Collapsible`，以及可直接拖动的两栏
  原生 `Resizable`；
- 显式 stable ID、固定 viewport 高度和始终可见原生 scrollbar 的 Audit Log
  `<scroll>`；
- `<input type="submit">` / `<input type="reset">` 映射的 `save` / `reset` Action，
  以及 `inspect-row` / `select-connection` /
  `selection-changed` / `accordion-changed` / `toggle-details` / `navigate` 到 Rust
  handler 的结构化派发；
- `data-record` / `data-connection` 形成的 Action payload；
- 通过 registry 注册的自定义 `<sql-editor />`；
- Action 一次提交多个 state 变化后自动 reconcile；save 会同步更新状态
  Alert、进度和计数 Badge。

定向质量门禁：

```bash
cargo fmt -p declarative-ui-demo -- --check
cargo test -p declarative-ui-demo
cargo check -p declarative-ui-demo --all-targets
cargo clippy -p declarative-ui-demo --all-targets -- -D warnings
```

## 最小用法

### 1. 编译模板

```rust
use declarative_ui_demo::{
    CompileOptions, ComponentRegistry, compile_template,
};

let registry = ComponentRegistry::with_defaults();
let template = compile_template(
    r#"
    <div class="flex flex-col gap-2">
        <span bind="status"></span>
        <input id="username" bind="username" />
        <button action="save" data-record="profile">保存</button>
    </div>
    "#,
    &registry,
    CompileOptions::strict(),
)?;
# Ok::<(), declarative_ui_demo::TemplateCompileError>(())
```

`compile_template` 是正式入口。它先执行受限 HTML parse，再规范化受支持的
`<input type>`，然后验证 tag、attribute、identity 和 Tailwind utility，成功后返回
不可变的 `CompiledTemplate`。`CompiledTemplate::source()` 保留调用方传入的原始
HTML，`CompiledTemplate::root()` 则返回已经规范化、可直接进入 binding/render
链路的 VNode。

### 2. 创建 Runtime

`Runtime` 由 GPUI `Entity` 持有。所有可观察 state 更新都通过 GPUI context 完成：

```rust,ignore
let runtime = cx.new(|_| {
    let mut state = StateStore::default();
    state.set("username", "admin");
    state.set("status", "等待保存");

    let mut runtime = Runtime::new(state);
    runtime.on("save", |context| {
        let username = context.get("username").unwrap_or_default().to_owned();
        context.set("status", format!("已保存用户: {username}"));
        Ok(())
    })?;
    Ok::<Runtime, RuntimeError>(runtime)
});
```

实际应用通常会在创建 Entity 前处理注册错误，而不是在 Entity 中返回
`Result`。Demo 的 `demo_runtime()` 给出了完整写法。

### 3. 挂载 View

```rust,ignore
let config = DeclarativeViewConfig::new(
    template,
    runtime.clone(),
    registry,
);
let view = cx.new(|cx| DeclarativeView::new(config, cx));
```

`DeclarativeView` 会订阅 `RuntimeEvent`。外部 state 更新不需要手工调用
`view.refresh()`：

```rust,ignore
runtime.update(cx, |runtime, cx| {
    runtime.set("status", "外部更新", cx);
});
```

`Runtime` 发出 `StateChanged` 后，View 自动 resolve binding、diff、patch 并
`cx.notify()`。

## 模板编译

### Strict 与 permissive

```rust
let strict = CompileOptions::strict();
let permissive = CompileOptions::permissive();
```

- **strict**：未知 tag、未知 Tailwind class 等支持性问题是 compile error；
- **permissive**：支持性问题保留为 warning，模板仍可挂载，并在运行时渲染未知组件
  fallback；
- forbidden HTML 和资源超限在两种模式下都是 hard failure，不能通过 permissive
  绕过。

适合正式、内置 UI 的默认选择是 strict。permissive 主要用于编辑器预览、迁移工具
和诊断界面。

### 编译资源限制

默认 `CompileLimits`：

| 资源 | 默认上限 | 计数规则 |
| --- | ---: | --- |
| 原始 source | 256 KiB | UTF-8 bytes，在自闭合标签展开前检查 |
| nodes | 10,000 | Element + 非空 Text；合成 Fragment 不计 |
| element depth | 64 | 根 element 的 depth 为 1 |
| attributes | 20,000 | 文档总数；`class` 属性本身也计 1 |
| class tokens | 20,000 | `class` 按空白拆分后的文档总数 |

可以按宿主场景降低上限：

```rust
use declarative_ui_demo::{CompileLimits, CompileOptions};

let options = CompileOptions::strict().with_limits(CompileLimits {
    max_source_bytes: 64 * 1024,
    max_nodes: 2_000,
    max_depth: 32,
    max_attributes: 4_000,
    max_classes: 4_000,
});
```

资源检查使用 fail-fast 语义。自定义 `<sql-editor />` 与显式
`<sql-editor></sql-editor>` 的 VNode 成本相同。

### Typed diagnostics

`Diagnostics` 同时保留：

- `DiagnosticSeverity::{Error, Warning}`；
- `DiagnosticPhase::{Compile, Binding, Render, Runtime}`；
- 稳定的 `DiagnosticCode`；
- 人类可读 `message`；
- 可选 `NodePath`；
- 可选 `SourceSpan`。

当前 html5ever RcDom 链路没有提供可靠的原始 source offset，因此
`SourceSpan` 暂时为 `None`，不会伪造位置。调用方应优先使用 `phase`、`code` 和
`path` 做机器判断：

```rust
for diagnostic in template.diagnostics().iter() {
    eprintln!(
        "{:?}/{:?} at {:?}: {}",
        diagnostic.phase,
        diagnostic.code,
        diagnostic.path,
        diagnostic.message,
    );
}
```

同一批次中完全相同的 diagnostic 会去重；相同错误发生在不同 VNode path 时仍会
分别保留。每次 binding/render/runtime 更新会替换对应 phase，避免累积过期诊断。

## State、Action 与事件

### StateStore

v1 的 state 是确定性的字符串键值表：

```rust
let mut state = StateStore::default();
state.set("username", "admin");
state.remove("obsolete");
```

字符串模型是刻意限制，不在 HTML 中加入表达式语言、隐式类型转换或脚本执行。未来
若需要 typed state，应在 Runtime contract 层增加明确 schema，而不是让 HTML
求值任意 Rust/JavaScript。

### ActionEvent

按钮只声明 Action 名称：

```html
<button
    id="save"
    action="save"
    data-record="profile"
>
    保存
</button>
```

点击后 Runtime 收到结构化 `ActionEvent`：

- `name`：`save`；
- `source_id`：由 tag + `key` / `id` / path 生成；
- `source_path`：触发节点的 `NodePath`；
- `payload`：所有 `data-*` 属性去掉 `data-` 后形成的字符串 map。

HTML 不执行代码。只有宿主显式注册的 Rust handler 能修改业务状态。

### 事务和错误语义

一次 dispatch 的流程：

```text
clone current StateStore
  ↓
在临时 state 上运行 handler
  ├─ Ok    → 一次 commit，revision 最多 +1
  ├─ Err   → 丢弃临时 state，ActionFailed
  └─ panic → catch_unwind，丢弃临时 state，ActionFailed
```

一个成功 Action 即使多次 `context.set(...)`，也只发出一次 `StateChanged`。
`changed_keys` 是该 transaction 的最终差异，随后再发 `ActionCompleted`。没有
实际 state 差异的 Action 仍会完成，但不增加 revision，也不发
`StateChanged`。

重复注册同名 Action 返回 `RuntimeError::DuplicateAction`，不会静默覆盖已有
handler。

`RuntimeEvent` 包括：

- `StateChanged(StateChange)`；
- `ActionCompleted { event, outcome }`；
- `ActionFailed { event, error }`。

## Binding 和有状态输入

### 文本绑定

```html
<span bind="status"></span>
```

会保留 `<span>` 自身及其 class/attribute，只把 children 解析成 state 文本。
缺失 key 解析为空字符串，并产生带 path 的 `MissingBinding` warning；当 key 后续
出现时，warning 会自动清除。

### Binding 目标

`bind` 并不对所有组件都无条件替换 children。编译器和 resolver 共享同一份目标
映射：

| 标签 | state 写入目标 | children 行为 |
| --- | --- | --- |
| `input`、`textarea`、`progress` | `value` attribute | 保留 |
| `checkbox`、`switch`、`radio` | `checked` attribute | 保留标签文本 |
| `badge` | `count` attribute | 保留 Badge 内容 |
| `rating` | `value` attribute | 不接受 children |
| `slider` | `value` attribute | 不接受 children |
| `pagination` | `current-page` attribute | 不接受 children |
| `tabs`、`stepper` | `selected-index` attribute | 保留强结构 child |
| `accordion` | `open-indices` attribute | 保留强结构 `accordion-item` children |
| `collapsible` | `open` attribute | 保留 summary 与强结构 content child |
| 其他允许 `bind` 的标签 | 文本 children | 原有 children 被 state 文本替换 |

因此：

```html
<checkbox bind="notifications">Email alerts</checkbox>
<progress bind="completion"></progress>
<badge bind="save_count"><span>Committed saves</span></badge>
<pagination bind="page" total-pages="20"></pagination>
<slider bind="volume" min="0" max="100"></slider>
<tabs bind="selected_tab"><tab>Overview</tab><tab>Activity</tab></tabs>
<accordion bind="open_sections" multiple>
    <accordion-item title="General">General settings</accordion-item>
    <accordion-item title="Advanced">Advanced settings</accordion-item>
</accordion>
<collapsible bind="details_open">
    <button action="toggle-details">Toggle details</button>
    <collapsible-content>Advanced details</collapsible-content>
</collapsible>
```

分别解析为对应的原生 attribute，checkbox label、Badge child 以及 Tabs 的强结构
children、Accordion items / Collapsible content 不会被清空。目标 attribute
必须只有一个来源；
下列组合都是 compile error：

- `input` / `textarea` / `progress` / `rating` / `slider`：`bind` 与显式
  `value`；
- `checkbox` / `switch` / `radio`：`bind` 与显式 `checked`；
- `badge`：`bind` 与显式 `count`；
- `pagination`：`bind` 与显式 `current-page`；
- `tabs` / `stepper`：`bind` 与显式 `selected-index`；
- `accordion`：`bind` 与显式 `open-indices`；
- `collapsible`：`bind` 与显式 `open`。

### HTML `<input type>` adapter

受限 HTML frontend 会在 **parse 之后、registry schema / identity 校验之前**规范化
常见 `<input>`。这只是确定性的 VNode adapter，不是浏览器 DOM，也不引入 HTML form、
constraint validation 或平台键盘语义。

| HTML 声明 | 规范化后的 DSL / 原生组件 | 准确边界 |
| --- | --- | --- |
| `<input>`、`type=""`、`type="text"` | `<input>` / 原生单行 `Input` | 缺失或空 `type` 使用默认 text；显式非空值 trim 后转 ASCII 小写 |
| `type="password"` | `<input type="password">` / masked `InputState` | 真实启用原生 masked 显示；value 仍是普通字符串 state，不提供加密存储 |
| `type="email"`、`search`、`url`、`tel` | `<input>` / 原生单行 `Input` | 只是文本 alias；没有浏览器格式校验、autocomplete、专用键盘或 URL/email constraint |
| `type="checkbox"` | `<checkbox>` / 原生 `Checkbox` | `bind` 写入 `checked`；字符串布尔合同与显式 `<checkbox>` 相同 |
| `type="radio"` | `<radio>` / 原生 `Radio` | `bind` 写入 `checked`；没有 HTML `name` group 或自动互斥 |
| `type="range"` | `<slider>` / 原生单值 `Slider` | 复用现有 `value` / `min` / `max` / `step` / `bind` / Action 合同；不是双端 range slider |
| `type="button"`、`submit`、`reset` | `<button>` / 原生 `Button` | `value` 转为 `label`；submit/reset 只是 Button，可通过显式 `action` 派发宿主行为，不会自动提交或重置 HTML form |
| `number`、日期时间族、`file`、`color`、`hidden`、`image` 等其他值 | 保留为规范化小写的 `<input type="…">` | 不静默降级成文本控件；renderer 返回 `ComponentRenderFailed`，错误保持在 typed render boundary 内且不会 panic |

adapter 改写后才执行目标组件 schema 校验。因此
`<input type="checkbox" placeholder="…">`、`<input type="range" checked>` 等组合
会按 `checkbox` / `slider` 合同在 strict compile 阶段拒绝，而不是把无效属性带到
renderer。

文本类 `<input readonly>` 会规范化为 DSL 的 `read-only`；同时声明 `readonly` 与
`read-only` 是 compile-time `ConflictingAttributes`。`readonly` 不适用于转换后的
Checkbox、Radio、Slider 或 Button，它会被目标 schema 当作 unsupported attribute。
HTML 数值 `size="20"` 也没有照搬：DSL 的 `size` 始终是
`xs|sm|md|lg`（及其全名 alias）。

转换保留原始 `id` / `key` 值，并继续参与整个 `CompiledTemplate` 的全局唯一
identity namespace；但组件 stable ID 的 tag prefix 使用转换后的目标组件，例如
`checkbox:notifications`、`radio:channel`、`slider:volume`、
`button:save`。显式 identity 因此在 sibling reorder 时仍会生成稳定组件 ID，并让
Slider cache 或其他使用 keyed native state 的目标组件在适用时复用状态；转换前后的
不同组件类型也不会被误当成同一个 cache entry。

当前没有把 `name`、`required`、`autocomplete`、`inputmode`、`maxlength`、
`minlength`、`pattern`、`form`、`list`、`multiple`、`accept`、`capture` 等浏览器
attribute 伪装成已支持能力；strict schema 会明确拒绝它们。需要 checkbox/radio
旁边的可见文本时，可使用 `<field label="…">`，或直接使用支持 child label 的
显式 `<checkbox>` / `<radio>` DSL 标签。

### 双向输入绑定

```html
<input id="username" bind="username" placeholder="用户名" />
<input id="credential" type="password" bind="credential" />
<textarea key="notes" bind="notes"></textarea>
```

- state → input：reconcile 后同步到已有 `InputState`；
- input → state：`InputEvent::Change` defer 写回 `Entity<Runtime>`；
- 程序调用 `InputState::set_value` 不重新产生 Change，因此不会形成 binding loop；
- `bind` 与声明式 `value` 同时出现是 compile error。

每个输入缓存一个 `Entity<InputState>` 和对应 subscription。节点移除后，cache entry
和 subscription 一起释放；旧 Entity 即使仍被其他 Rust 代码持有，也不会继续写回
Runtime。

### 布尔表单控件

```html
<checkbox id="notifications" bind="notifications">Email alerts</checkbox>
<switch id="auto-sync" bind="auto_sync">Auto-sync metadata</switch>
<radio id="beta-mode" bind="beta_mode">Enable beta mode</radio>
```

`checkbox`、`switch` 和 `radio` 使用字符串 state，但按明确的布尔约定解析：

| 语义 | 接受的字符串（忽略大小写与外围空白） |
| --- | --- |
| true | `true`、`1`、`yes`、`on` |
| false | `false`、`0`、`no`、`off` |

普通声明式布尔 attribute 可以使用 bare attribute，例如 `disabled`；它等价于
`true`。绑定缺失 key 仍遵循通用规则：解析为空字符串并产生 `MissingBinding`
warning，而不是把空字符串当作 checked。

点击控件时先把新的 `bool` 以 Rust 的 `"true"` / `"false"` 字符串写回
`Entity<Runtime>`，再派发可选的 `action`。因此同一个控件可以同时有 `bind`、
`action` 和 `data-*` payload；业务代码仍只在宿主注册的 Rust handler 中执行。

### 数值选择 binding

```html
<pagination
    id="pager"
    bind="page"
    total-pages="20"
    action="selection-changed"
    data-control="pagination"
></pagination>
<rating bind="score" max="5"></rating>
<slider
    id="volume-slider"
    bind="volume"
    min="0"
    max="100"
    step="1"
    action="selection-changed"
    data-control="slider"
></slider>
<tabs bind="selected_tab">
    <tab>Overview</tab>
    <tab>Activity</tab>
</tabs>
<stepper bind="selected_step">
    <stepper-item>Configure</stepper-item>
    <stepper-item>Review</stepper-item>
</stepper>
```

- `pagination` 使用 1-based 正整数页码；
- `rating.value` 使用非负整数；
- `slider.value` 是有限单精度浮点数；当前只支持原生
  `SliderValue::Single(f32)`，不接受 range slider 的字符串编码；
- `tabs` / `stepper` 使用 0-based `selected-index`，并在 render 时校验索引小于
  child 数量；
- Pagination、Rating、Tabs 和 Stepper 的 native callback 先把新数值转换为十进制
  字符串写回绑定 key，然后才派发可选 Action；
- Slider 的 `Change` 事件连续把单值按 `f32::to_string()` 的确定性十进制表示写回；
  `Release` 再确认最终 binding，然后才派发可选 Action。因此 Slider 的 Action
  handler 同样能从 `context.get(...)` 读取最终值；
- 数值 binding key 应在挂载前初始化。缺失 key 仍按通用 binding 规则解析为空字符串
  并产生 `MissingBinding` warning，而空字符串不是合法数值，renderer 还会产生明确的
  `ComponentRenderFailed`，不会偷偷选用默认值。

### Accordion JSON 状态 binding

Accordion 的展开状态不是逗号分隔文本，而是一个明确的 JSON 数组协议：

```text
[]       # 全部关闭
[0]      # 展开第 0 项
[0,2]    # 展开第 0、2 项
```

```html
<accordion
    id="settings"
    bind="open_sections"
    multiple
    action="accordion-changed"
    data-control="settings"
>
    <accordion-item title="General">General settings</accordion-item>
    <accordion-item title="Advanced">Advanced settings</accordion-item>
    <accordion-item title="Runtime">Runtime settings</accordion-item>
</accordion>
```

- 数组元素是 0-based、非负、可表示为 Rust `usize` 的 JSON 整数；object、字符串、
  浮点数和负数都会进入明确的 `ComponentRenderFailed`；
- renderer 会先按升序排序并去重，所以输入 `[2,0,2]` 与 canonical state
  `[0,2]` 等价；
- 每个 index 都必须小于直接 `accordion-item` 的数量；`multiple=false`（默认）时，
  canonical 数组最多包含一个 index；
- native callback 返回的 index 顺序不稳定，adapter 会再次排序、去重，并以无空格的
  JSON（例如 `[0,2]`）写回 Runtime；
- 缺失 binding key 仍产生 `MissingBinding` warning。resolver 为该缺失 key 生成的
  空字符串在 bound Accordion 上被特殊解释为 `[]`，避免同一个缺失状态再制造 render
  failure；显式 `open-indices=""` 仍是非法 JSON，不会被静默接受；
- callback 先提交 binding，再派发可选 `action` 和 `data-*` payload，因此 Action
  handler 可立即用 `context.get("open_sections")` 读取新 JSON；
- 上游 click callback 挂在 Accordion 外层，展开内容中的 click 可能冒泡。adapter
  记住最后一次 canonical state，相同状态不会重复写回或重复派发 Action。

正式模板应在挂载前初始化 binding，例如
`state.set("open_sections", "[0]")`，而不是依赖缺失 key 的兼容路径。

### Stable identity

有状态组件 identity 的优先级：

1. `key`；
2. `id`；
3. VNode path。

显式 identity value 在一个 `CompiledTemplate` 中共享同一唯一命名空间，即
`id="field"` 与 `key="field"` 也会被判定为重复。使用显式 identity 后，输入和
Slider 节点在同一父级内 reorder 时仍分别复用原来的 `InputState` /
`SliderState`。没有 `id` / `key` 的有状态组件使用 path；结构插入或移动可能让它
重建。

当 multiline、password masked mode、placeholder 或 bind 配置改变时，输入 Entity
会重建，旧 Entity 的 write-back subscription 同时释放；仅 bound value 改变时会
复用 Entity 并同步其值。

Slider cache 同样以 `ComponentProps::stable_id()` 为 key。bound Slider 的 state
值变化时，会在原 `Entity<SliderState>` 上调用 `set_value`，该程序化同步不产生
Slider event，因而不会形成 binding loop。`min` / `max` / `step` / `scale`
configuration 改变、bound/unbound 模式切换时会重建 Entity；节点从 resolved VNode
移除后，cache entry 和 write-back subscription 一起释放。

`Pagination`、`Rating`、`TabBar` 和 `Stepper` 同样使用
`ComponentProps::stable_id()` 构造原生组件。尤其 `Rating` 和 `TabBar` 会按该 ID
访问 window 内 keyed state；模板作者应在节点可能 reorder 时提供稳定、唯一的
`id` / `key`，不要依赖会随结构变化的 VNode path。

`Accordion::new` 也接收声明节点的 `ComponentProps::stable_id()`；但展开项仍在每次
render 时由 resolved `open-indices` 明确设置，source of truth 是 Runtime binding，
不是 adapter 隐藏的持久化 open state。

`ResizablePanelGroup` 也以 `ComponentProps::stable_id()` 访问上游的 window-keyed
`ResizableState`。拖动后的 panel sizes 因而能跨普通声明式 rerender 复用；如果
Resizable 节点可能 reorder，同样应提供显式 `id` / `key`。

`Scroll` 比其他组件更严格：schema 要求非空 `id`，其原生 `ScrollHandle` 使用
`ComponentProps::stable_id()` 派生 window-keyed state key。因而在父级插入、删除或
reorder 兄弟节点时，滚动 offset 不依赖 VNode path；显式 ID 仍参与上面的全局唯一
identity namespace。

## Component Registry

### 默认组件

所有组件都支持全局 `id`、`key` 和受限 `class`。默认 registry 按能力分组如下：

| 分组 | 标签 | 原生映射 / 准确边界 |
| --- | --- | --- |
| semantic / basic | `div`、`span`、`section`、`article`、`header`、`footer`、`main`、`nav` | `gpui::div()` 容器；这些名称只保留 DSL 语义，不模拟浏览器 layout |
| semantic / basic | `button`、`img` | `gpui_component::button::Button`、`gpui::img(src)` |
| semantic / basic | `group-box`、`label`、`tag`、`skeleton` | 原生 `GroupBox`、`Label`、`Tag`、`Skeleton` |
| form / input controls | `form`、`field` | 强结构的原生 `Form` + `Field` |
| form / input controls | `input`、`textarea` | 原生 `Input` + 缓存的 `Entity<InputState>`；分别是单行和多行；HTML frontend 会先处理受支持的 `input type`，password 进入 masked mode |
| form / input controls | `checkbox`、`switch`、`radio` | 原生 `Checkbox`、`Switch`、`Radio`；支持字符串布尔双向 binding |
| static table | `table`、`thead`、`tbody`、`tfoot`、`tr`、`th`、`td`、`caption` | 原生 `Table`、section、row、cell 和 caption primitives |
| static list | `list`、`list-item` | flex-column 静态容器 + 原生 `ListItem` |
| feedback / display | `alert`、`badge`、`progress`、`spinner` | 原生 `Alert`、`Badge`、`Progress`、`Spinner` |
| feedback / display | `separator`、`divider` | 公共可用的原生 `Separator`；`divider` 是语义 alias |
| display | `avatar`、`avatar-group` | 原生 `Avatar`、`AvatarGroup`；group 强制统一 child size |
| display | `description-list`、`description-item` | 强结构的原生 `DescriptionList` + `DescriptionItem` |
| navigation | `breadcrumb`、`breadcrumb-item` | 强结构的原生 `Breadcrumb` + `BreadcrumbItem`；点击只派发 Action |
| navigation | `pagination`、`rating` | 原生 `Pagination`、`Rating`；支持数值 binding 和 Action |
| navigation | `tabs`、`tab` | 强结构的原生 `TabBar` + `Tab`；支持 selected-index binding |
| navigation | `stepper`、`stepper-item` | 强结构的原生 `Stepper` + `StepperItem`；支持 selected-index binding |
| controls | `kbd` | 原生 `Kbd`；必填 `stroke` 描述要显示的 GPUI keystroke |
| controls | `slider` | 原生 `Slider` + 缓存的 `Entity<SliderState>`；当前只支持单值数值 binding |
| layout | `accordion`、`accordion-item` | 原生 controlled `Accordion` + `AccordionItem`；使用 JSON open-index binding |
| layout | `collapsible`、`collapsible-content` | 原生 controlled `Collapsible`；普通 child 常显，唯一 content child 由 `open` 控制 |
| layout | `resizable`、`resizable-panel` | 原生 `ResizablePanelGroup` + `ResizablePanel`；拖动尺寸使用 window-keyed state |
| layout | `scroll` | 原生 `ScrollHandle` + `Scrollbar`；显式 ID 保持 handle identity，有限 viewport 由数值属性或父布局提供 |

常用 attribute：

| 标签 | 主要 attribute |
| --- | --- |
| semantic containers | `bind` |
| `button` | `label` 或直接文本、`action`、`data-*`、`variant`、`size`、`disabled`、`outline`、`loading`、`tooltip` |
| `input` | `type=text\|password\|email\|search\|url\|tel`（默认 text）、`bind`、`value`、`placeholder`、`size`、`disabled`、`read-only` / HTML `readonly` alias、`cleanable` |
| `textarea` | `bind`、`value`、`placeholder`、`size`、`disabled`、`read-only`、`cleanable`；不接受 `type` |
| `img` | 必填 `src` |
| `group-box` | `title`、`variant=normal\|fill\|outline` |
| `label` | `bind`、`secondary`、`masked` |
| `tag` | `variant`、`outline`、`size` |
| `skeleton` | `secondary` |
| `form` | `layout=vertical\|horizontal`、`columns`、`label-width`、`label-text-size`、`size` |
| `field` | `label`、`description`、`required`、`visible`、`label-indent`、`col-span`、`col-start`、`col-end`、`label-justify=start\|center\|end`、`align=start\|center\|end` |
| `checkbox`、`switch`、`radio` | `bind`、`checked`、`disabled`、`action`、`data-*`、`size`、`tooltip` |
| `table` | `size` |
| `th`、`td` | `colspan`、`align=left\|center\|right` |
| `list-item` | `selected`、`secondary-selected`、`disabled`、`confirmed`、`separator`、`action`、`data-*` |
| `alert` | `bind`、`variant=default\|info\|success\|warning\|error\|danger`、`title`、`banner`、`visible`、`size` |
| `badge` | `bind`、`count`、`max`、`dot`、`size` |
| `progress` | `bind`、`value`、`loading`、`size` |
| `spinner` | `size` |
| `separator`、`divider` | `orientation=horizontal\|vertical`、`dashed`、`label` |
| `avatar` | `name`、`src`、`size` |
| `avatar-group` | `limit`、`ellipsis`、`size` |
| `description-list` | `layout=horizontal\|vertical`、`label-width`、`bordered`、`columns`、`size` |
| `description-item` | 必填 `label`、`span` |
| `breadcrumb-item` | `label` 或直接文本、`disabled`、`action`、`data-*` |
| `pagination` | `bind`、`current-page`、`total-pages`、`visible-pages`、`compact`、`disabled`、`size`、`action`、`data-*` |
| `rating` | `bind`、`value`、`max`、`disabled`、`size`、`action`、`data-*` |
| `tabs` | `bind`、`selected-index`、`variant=tab\|outline\|pill\|segmented\|underline`、`menu`、`size`、`action`、`data-*` |
| `tab` | `label` 或直接文本、`disabled` |
| `stepper` | `bind`、`selected-index`、`layout=horizontal\|vertical`、`text-center`、`disabled`、`size`、`action`、`data-*` |
| `stepper-item` | `disabled` |
| `kbd` | 必填 `stroke`、`appearance`、`outline` |
| `slider` | `bind`、`value`、`min`、`max`、`step`、`scale=linear\|logarithmic\|log`、`orientation=horizontal\|vertical`、`disabled`、`action`、`data-*` |
| `accordion` | `bind`、`open-indices`、`multiple`、`bordered`、`disabled`、`size`、`action`、`data-*` |
| `accordion-item` | 必填 `title` |
| `collapsible` | `bind`、`open` |
| `resizable` | `orientation=horizontal\|vertical`、`size` |
| `resizable-panel` | `size`、`min-size`、`max-size`、`visible` |
| `scroll` | 必填 `id`、`axis=vertical\|horizontal\|both`、`scrollbar-show=scrolling\|hover\|always`、`width`、`height` |

通用 `size` 接受 `xs` / `sm` / `md` / `lg`（也接受对应的
`xsmall` / `small` / `medium` / `large`）。attribute 的值在 render 时继续进行
typed validation；例如有限数值、非负整数、正整数、variant 和 alignment 非法时，
renderer 返回 `ComponentError`，由组件错误边界转成 typed diagnostic 和可见 fallback。

`span` 和其他 semantic tag 不实现浏览器 inline / block 规则，它们都是 styled
GPUI div-like 容器。`section` 等标签也不会引入 HTML 默认 margin、ARIA 或
浏览器语义。

### Form 结构合同

`<form>` renderer 不把子树当作任意元素列表，而是直接从 VNode 构造强类型
`Vec<Field>`：

```html
<form layout="vertical" columns="2" label-text-size="0.875">
    <field
        label="Username"
        label-justify="start"
        col-start="1"
        col-end="2"
        required
    >
        <input bind="username" />
    </field>
    <field label="Notifications" label-indent="false">
        <checkbox bind="notifications">Email alerts</checkbox>
    </field>
</form>
```

- `field` 必须是 `form` 的直接 element child；
- `field` 内部可以渲染普通声明式 children；
- 单独渲染 `<field>` 或把其他 element 直接放到 `<form>` 下会返回结构错误；
- `columns` 和 `col-span` 必须是正整数；`label-width` 必须是有限非负像素值；
- `label-text-size` 是有限且严格大于零的 rem 数值，直接映射
  `Form::label_text_size(Rems)`；
- `col-start` / `col-end` 是 `-32768..=32767` 的 signed grid line，负数保留
  GPUI grid 的反向索引语义；DSL 不额外要求 start 小于 end；
- `label-justify` 控制 label 内容在 label 区域内的水平对齐，`align` 控制整个
  field children 的 item alignment，两者不是同一属性。

这不是浏览器 form submission，也不生成 HTTP request；状态和提交行为仍分别由
`bind` 与宿主注册的 Action handler 管理。

### Table 结构合同

Table 同样由父 renderer 一层层构造强类型原生组件：

```html
<table size="sm">
    <thead><tr><th>Name</th><th>Status</th></tr></thead>
    <tbody>
        <tr><td>Production</td><td><tag>Healthy</tag></td></tr>
    </tbody>
    <caption>Connection health</caption>
</table>
```

结构必须满足：

```text
table
  ├─ thead | tbody | tfoot
  │    └─ tr
  │         └─ th | td
  └─ caption
```

`th`、`td` 和 `caption` 内部可以包含任意已注册组件。每一层都保留准确的 VNode
path 并应用自己的受限 class。非法层级或单独渲染结构标签会进入
`ComponentRenderFailed` 错误边界。

这是**静态声明式 Table 组合**：没有 dataset delegate、排序、筛选、分页、编辑模型
或虚拟滚动。结构标签按照与 Component Registry 相同的 ASCII 大小写无关规则匹配；
HTML5 parser 通常也会把源码标签规范化为小写。模板仍应像上例一样显式给出
section / row 层级。

### List 的准确边界

```html
<list class="gap-2">
    <list-item
        selected
        action="select-connection"
        data-connection="Production"
    >
        Production
    </list-item>
    <list-item secondary-selected>Staging</list-item>
    <list-item separator>Archived connections</list-item>
</list>
```

`list` 是一个默认 `flex-column` 的静态声明式容器；每个 `list-item` 使用原生
`gpui_component::list::ListItem`，可以显示 selected / secondary-selected /
confirmed / disabled 状态并派发结构化 Action。`separator` 进入原生 separator
mode，因此不可交互，也不显示 selected / secondary-selected 高亮；即使模板同时
声明 selection state 或 `action`，DSL 也不发明冲突错误，而是保持上游 renderer
自然忽略这些交互与高亮。

它**不是** `gpui_component::list::List<D>`，不支持 delegate、search、数据驱动 row
recycling 或 virtual scroll。需要大数据列表时应由可信宿主 Rust component 提供
真正的 delegate，而不是把大量记录展开成不受控 HTML。

### Feedback 数值与包装层

- `badge.count` / `badge.max` 解析为非负 `usize`；
- `progress.value` 必须是有限 `f32`，原生 `Progress` 在显示时 clamp 到
  `0..=100`；
- `alert` 使用 variant 对应的原生 constructor，因此 success / warning / error
  同时获得对应样式和图标；
- `Badge` 和 `Spinner` 当前没有公开的 `Styled` 实现，声明式 class 应用在稳定包装
  层；计数 overlay、动画和内部 layout 仍由原生组件负责；
- 上游源码存在 `Divider` 实现，但 `gpui-component` 的公共 crate API 没有导出该
  module。在只依赖公共 API 的前提下，DSL 的 `<divider>` 明确作为
  `<separator>` 的语义 alias，由公共 `Separator` 实现；这里不声称实例化了不可访问
  的 `Divider` 类型。

### Avatar 与 DescriptionList 结构合同

```html
<avatar name="Ada Lovelace" size="lg"></avatar>
<avatar-group limit="3" ellipsis size="sm">
    <avatar name="Ada Lovelace"></avatar>
    <avatar name="Grace Hopper"></avatar>
    <avatar name="Margaret Hamilton"></avatar>
    <avatar name="Barbara Liskov"></avatar>
</avatar-group>

<description-list layout="horizontal" columns="2" label-width="120">
    <description-item label="Owner">Platform</description-item>
    <description-item label="State" span="2">
        <tag variant="success">Ready</tag>
    </description-item>
</description-list>
```

- `avatar` 没有声明式 child slot，出现 children 会报错，而不是静默丢弃；
- `avatar-group` 只接受直接 `avatar` element child；`limit` 必须是正整数；
- 原生 `AvatarGroup` 会统一 child size，因此 group 内的 `avatar` 不允许再声明自己的
  `size`，统一由 group 的 `size` 决定；
- `description-list` 只接受直接 `description-item` child；
- `description-item.label` 必填，children 渲染为 value；`span` 必须是正整数且不能
  超过父列表的 `columns`；
- `description-list.columns` 默认为 3，声明值限制在 `1..=10`；
  `label-width` 必须是有限、非负的像素数，`bordered` 默认为 true；
- 单独渲染 `description-item` 会产生结构错误。

`Avatar` / `AvatarGroup` 暴露原生 `Styled`，所以 class 直接应用于原生组件。
`DescriptionList` 本身没有公开 `Styled`，list class 应用到稳定的外层 wrapper；
`DescriptionItem` 是没有 `Styled` 的 enum，item class 应用到其 value wrapper。
wrapper 不会替代原生 description grid、label 或 border 渲染。

### Navigation 结构合同

```html
<breadcrumb>
    <breadcrumb-item action="navigate" data-page="home">Home</breadcrumb-item>
    <breadcrumb-item disabled>Connections</breadcrumb-item>
</breadcrumb>

<tabs bind="selected_tab" variant="underline">
    <tab>Overview</tab>
    <tab>Activity</tab>
</tabs>

<stepper bind="selected_step">
    <stepper-item><span>Configure</span></stepper-item>
    <stepper-item><span>Review</span></stepper-item>
</stepper>
```

结构规则：

```text
breadcrumb
  └─ breadcrumb-item

tabs
  └─ tab

stepper
  └─ stepper-item
```

- 父节点只接受对应的直接结构 child；`breadcrumb-item`、`tab`、
  `stepper-item` 单独渲染都会报错；
- `breadcrumb-item` 和 `tab` 的 label 必须二选一：使用 `label` attribute，或使用
  直接文本；二者并存、空 label、嵌套 element label 都会报错；
- `breadcrumb-item` 没有 `href`。点击只向宿主 Runtime 派发白名单 Action，
  `disabled` 由原生组件阻止点击；
- `tabs` 至少需要一个 `tab`，variant 只接受
  `tab|outline|pill|segmented|underline`；
- `stepper` 至少需要一个 `stepper-item`；item children 可以是任意已注册声明式内容；
- `pagination` 和 `rating` 没有原生 child slot，因此拒绝 children；
- `pagination`、`rating`、`tabs`、`stepper` 都可以同时声明 `bind`、`action` 和
  `data-*`，并遵循“先写回绑定，再派发 Action”的顺序。

### Kbd 与 Slider 合同

`Kbd` 只负责原生快捷键外观展示：

```html
<kbd stroke="cmd-enter" appearance="true" outline></kbd>
```

- `stroke` 必填、不得为空，并通过 `gpui::Keystroke::parse` 校验；
- `key` 仍是所有组件共享的 stable identity attribute，不能复用来表示快捷键；
- `appearance` 默认为 true，`outline` 默认为 false；两者都使用统一的严格布尔
  attribute parser，非法字符串进入 `ComponentRenderFailed`；
- `kbd` 不接受 children，受限 `class` 直接应用到原生 `Kbd`；
- 这只是视觉提示，不提供“根据字符串查找并执行任意 GPUI `Action`”的 ABI。真实
  keyboard shortcut 仍应由可信宿主通过 GPUI action / key binding 机制注册。

Slider 当前刻意只提供可完整描述的单值合同：

```html
<slider
    id="volume"
    bind="volume"
    min="0"
    max="100"
    step="1"
    scale="linear"
    orientation="horizontal"
    action="selection-changed"
    data-control="slider"
    class="w-full"
></slider>
```

- `min` / `max` / `step` 默认分别为 `0` / `100` / `1`；`scale` 默认为
  `linear`，也接受 `logarithmic` / `log`；`orientation` 默认为
  `horizontal`；
- 省略 `value` 时以 `min` 作为初始值；所有数值必须 finite，且要求
  `min < max`、`step > 0`、`value ∈ [min, max]`。logarithmic scale 还要求
  `min > 0`；
- 越界声明 value 是明确错误，不会静默 clamp。write-back 会防御性地把原生组件因
  step rounding 产生的单值限制回 `[min, max]`，避免把下一轮 render 推入非法
  state；`disabled` 使用严格布尔解析，Slider 不接受 children；
- 当前只接受 `SliderValue::Single(f32)`。没有为 range slider 发明逗号、JSON 或
  其他字符串协议，也不声称已经支持 range；
- unbound Slider 的显式 `value` 是 Entity 的初始值，用户交互后不会在每轮 render
  被该初始值覆盖；只有声明 `bind` 的 Slider 才作为 Runtime state 控制值持续同步；
- bound Slider 的 `Change` 连续写回字符串 binding；`Release` 先再次写回最终值，
  再派发可选 `action` 和 `data-*` payload。程序化 state → Slider 同步不会反向
  emit event；
- 受限 `class` 直接应用到原生 `Slider`。

### Accordion、Collapsible、Resizable 与 Scroll 布局合同

`Accordion` 是由 Runtime JSON state 控制展开项的原生结构组件：

```html
<accordion
    id="settings"
    bind="open_sections"
    multiple
    bordered="false"
    size="sm"
    action="accordion-changed"
    data-control="settings"
    class="w-full"
>
    <accordion-item title="General" class="p-2">
        <span>General settings</span>
    </accordion-item>
    <accordion-item title="Advanced">
        <tag variant="info">Advanced settings</tag>
    </accordion-item>
</accordion>
```

- `<accordion>` 至少需要一个直接 `<accordion-item>` element child，并且不接受其他
  直接 child；单独渲染 item 或混入 text / 其他 element 都是结构错误；
- 每个 item 的字符串 `title` attribute 必填且 trim 后不得为空。item body 可以包含
  任意已注册声明式组件；DSL 当前不把嵌套 element 当成标题；
- `multiple` 默认为 false，`bordered` 默认为 true，`disabled` 默认为 false；
  三者使用统一的严格布尔解析。`size` 接受通用 `xs|sm|md|lg` 尺寸；
- 原生 Accordion 是 controlled component：每轮 render 都由 canonical
  `open-indices` 设置 item 的 `open`。交互式模板应使用 `bind` 让 callback 把新
  JSON 写回 Runtime，而不是依赖不可见的内部长期状态；
- 同时声明 `bind` 和 `action` 时，adapter 先写回 binding，再 dispatch Action；
  Action handler 因此读取到新值。上游外层 click callback 可能因 content click
  冒泡而收到未变化状态，adapter 会去重，不重复写回或派发；
- 当前不暴露 item-level `disabled`：上游 group render 会用 group
  `disabled` 覆盖每个 item 的该属性，暴露它会形成误导性的无效合同；
- 原生 `Accordion` / `AccordionItem` 都没有公开 `Styled`。group class 应用到
  `w-full` 稳定 wrapper；item class 只应用到展开 body wrapper，不修改 header；
- 非法 JSON、越界 index、`multiple=false` 下的多个 index、空标题和非法结构都会
  进入 `ComponentRenderFailed`，不会 panic。

`Collapsible` 是 controlled display primitive，不是内部自带 toggle state 和 trigger
的 disclosure widget：

```html
<collapsible id="advanced" bind="details_open" class="gap-2">
    <button action="toggle-details">Toggle details</button>
    <collapsible-content class="p-4">
        <tag variant="info">Advanced details</tag>
    </collapsible-content>
</collapsible>
```

- `<collapsible>` 必须恰好包含一个直接 `<collapsible-content>` element child；
- 其他直接 children 按原顺序映射为原生普通 children，始终显示；content child
  只有在 `open=true` 时显示；
- `open` 默认为 false，接受统一的严格布尔字符串；bare `open` 等价于 true；
- `bind` 定向写入 `open` attribute，因此不会替换 summary 或 content。缺失 binding
  仍产生 `MissingBinding` warning，并以关闭状态显示；
- 上游 `Collapsible` 不保存长期 open state，也没有 toggle callback。需要交互时，
  应像示例一样让可信宿主 Action 修改 binding；DSL 不伪造一个不存在的 native
  trigger 事件；
- `collapsible` 的 class 直接应用于原生 `Collapsible`；
  `collapsible-content` 的 class 应用到稳定 content wrapper；
- 单独渲染 `<collapsible-content>`、缺失或重复 content、非法 `open` 都进入
  `ComponentRenderFailed`，不会 panic。

`Resizable` 映射真正的 native drag layout：

```html
<resizable
    id="workspace-layout"
    orientation="horizontal"
    size="240"
    class="w-full"
>
    <resizable-panel size="220" min-size="100" max-size="400" class="p-4">
        Navigation
    </resizable-panel>
    <resizable-panel min-size="120" class="p-4">
        Content
    </resizable-panel>
</resizable>
```

- `<resizable>` 只接受直接 `<resizable-panel>` element children，且至少需要两个；
  单独渲染 panel 或混入其他直接 child 是结构错误；
- `orientation=horizontal`（默认）表示 panels 横向排列并左右拖动；
  `vertical` 表示纵向排列并上下拖动；
- group `size` 必须是有限正像素值，表示 cross axis 尺寸：horizontal 时是高度，
  vertical 时是宽度。`ResizablePanelGroup` 没有公开 `Styled`，所以 adapter 在稳定
  wrapper 上落实该尺寸和 group class，而 native group 继续填满 wrapper；
- panel `size` / `min-size` / `max-size` 都是有限、非负像素值。`min-size` 默认与
  上游一致为 `100`，`max-size` 必须严格大于 `min-size`，显式初始 `size` 必须位于
  配置范围内；
- `visible` 默认为 true，并使用严格布尔解析；panel class 直接应用于原生
  `ResizablePanel`；
- native group 使用声明节点的 stable ID 取得 window-keyed `ResizableState`，所以
  rerender 不会把一次 drag 立即重置为初始声明值；
- 当前不把动态 panel sizes 编码进字符串 Runtime，也不提供 resize `bind` /
  `action`。尺寸留在上游 native state；如果宿主需要持久化，应先定义明确的结构化
  size 协议和 lifecycle，而不是临时发明逗号分隔字符串。

`Scroll` 组合原生 `ScrollHandle`、overflow viewport 与 `Scrollbar` overlay：

```html
<scroll
    id="audit-log"
    axis="vertical"
    scrollbar-show="always"
    width="320"
    height="180"
    class="w-full"
>
    <div class="flex flex-col gap-2">
        <span>Connected to primary</span>
        <span>Schema refreshed</span>
    </div>
</scroll>
```

- 非空 `id` 必填，并与所有其他显式 `id` / `key` 共享唯一命名空间。
  `ScrollHandle` 的 window-keyed state key 从该组件的 stable identity 派生，不使用
  caller location 或 VNode path fallback；普通 rerender 和兄弟节点 reorder 不会把
  另一个 Scroll 的 offset 串进来；
- `axis` 默认为 `vertical`，还接受 `horizontal` 和 `both`。adapter 分别把它们映射
  到 `overflow_y_scroll()`、`overflow_x_scroll()` 和 `overflow_scroll()`，并把相同
  轴向交给原生 `Scrollbar`；
- `scrollbar-show` 默认为明确的 `scrolling`，还接受 `hover` / `always`。adapter
  总是调用原生 `scrollbar_show(...)`，所以 DSL 默认不随应用 theme 的 scrollbar
  设置漂移；
- 可选 `width` / `height` 是 GPUI pixel，存在时必须是 finite 且严格大于零；空串、
  `0`、负数、`NaN` 和 infinity 都会成为 `ComponentRenderFailed`，不会 panic。
  数值尺寸在受限 `class` 之后应用，因此显式数值属性优先；
- 省略某个数值尺寸时，wrapper 在该方向填满可用空间。父布局或 class 必须提供有限
  viewport；如果 viewport 没有有限边界，或 child 在滚动轴上没有大于 viewport 的
  自然尺寸，上游就不会形成可滚动的 `max_offset`；
- `track_scroll` 和对应 overflow 设置位于同一个 viewport。Scrollbar 是 absolute
  overlay，不参与内容布局；children 仍由普通声明式组件渲染，不提供
  delegate、virtualization 或 row recycling；
- 这是 GPUI 原生 scrolling model，不模拟浏览器 scrollbar CSS / overscroll，也不
  暴露 imperative 或脚本式 scroll-to API。

### Display / navigation 数值边界

这些限制不仅验证能否 parse 成 `usize`，还避免不可信模板驱动巨大分配或渲染循环：

| 属性 | 声明式边界 | 原生行为 |
| --- | --- | --- |
| `description-list.columns` | `1..=10` | 构造对应 column grid |
| `description-item.span` | 正整数且 `<= parent columns` | 跨列显示 |
| `avatar-group.limit` | 正整数 | 超出部分由 group 收起 |
| `pagination.current-page` / `total-pages` | 1-based 正整数 | `total-pages` builder 会把展示页 clamp 到可用范围 |
| `pagination.visible-pages` | 正整数且 `<= 100` | 原生实现实际至少按 5 个可见页处理 |
| `rating.value` | 非负整数 | 原生 `max` builder 将显示值 clamp 到 `0..=max` |
| `rating.max` | `1..=100` | 最多渲染 100 个 rating item |
| `tabs.selected-index` | `0 <= index < tab count` | 0-based selection |
| `stepper.selected-index` | `0 <= index < item count` | 0-based selection |

对于 Pagination 和 Rating，clamp 是原生组件的**展示行为**，不会悄悄重写模板中的
state；只有用户触发 native callback 时才按 binding 合同写回新值。

### `gpui-component` 支持矩阵

这里的“支持”指正式注册到受限 DSL、具有 schema / 结构 / diagnostics 契约，而不只是
上游 Rust crate 中存在同名类型。当前矩阵按接入所需宿主能力分类：

| 分类 | 上游组件 / 模块 | standalone v1 状态与边界 |
| --- | --- | --- |
| **已映射** | `Button`、`Input`、`Checkbox`、`Switch`、`Radio`、`Form` / `Field`、`GroupBox`、`Label`、`Tag`、`Skeleton` | 使用公共原生 API；HTML `<input>` frontend adapter 覆盖 text/password/email/search/url/tel、checkbox/radio/range 和 button/submit/reset；Form / Field 映射 label rem size、label 对齐和 signed grid line；输入和布尔控件具有明确字符串 binding |
| **已映射** | `Alert`、`Badge`、`Progress`、`Spinner`、`Separator` | 使用公共原生 API；无 `Styled` 的组件通过稳定 wrapper 接收 class |
| **已映射** | `Avatar`、`AvatarGroup`、`DescriptionList` / `DescriptionItem` | 使用公共原生 API；资源、结构和 wrapper 边界见上文 |
| **已映射** | `Breadcrumb` / `BreadcrumbItem`、`Pagination`、`Rating`、`TabBar` / `Tab`、`Stepper` / `StepperItem` | 使用公共原生 API；navigation 只产生 Runtime Action，不直接执行导航 |
| **已映射** | `Kbd`、单值 `Slider` | 使用公共原生 API；Slider 使用稳定 Entity cache、双向数值 binding 和 release Action |
| **已映射** | `Accordion` / `AccordionItem` | 使用公共原生 API；controlled open state 使用 canonical JSON binding，Action 在写回后派发 |
| **已映射** | `Collapsible`、`ResizablePanelGroup` / `ResizablePanel` | 使用公共原生 API；Collapsible 是 controlled primitive，Resizable drag state 按 stable ID 存在 window keyed state |
| **已映射 / 明确受限** | `ScrollHandle` / `Scrollbar` | DSL `<scroll>` 映射原生 overflow viewport 和 overlay scrollbar；显式 ID 保持 handle state，仅支持轴向、显示模式和 pixel viewport 合同 |
| **部分映射 / 明确受限** | table primitives 与 `DataTable<D>` | DSL 只映射大小写无关的静态 `Table` / section / row / cell / caption；没有映射 `DataTable<D>`、`TableState<D>` 或 `TableDelegate` |
| **部分映射 / 明确受限** | `ListItem` 与 `List<D>` | DSL 只映射静态 flex-column list + 原生 selected / secondary-selected / confirmed / disabled / separator 状态；没有映射 delegate、search、row recycling 或 virtual scroll |
| **部分映射 / 明确受限** | `Divider` | 上游 module 未从公共 crate API 导出；DSL 的 `divider` 是公共 `Separator` alias |
| **尚未映射，可继续评估** | `Sidebar`、`Setting`、`Text` | 不能因上游存在类型就视为 DSL 已支持；需要逐个定义 schema、结构、state 和错误边界 |
| **delegate / entity / data 型** | `DataTable<D>`、`List<D>`、`Select<D>`、`Tree`、`Chart`、`Plot` | 不把任意 dataset/delegate/entity 塞入字符串 HTML；应由可信宿主 Rust component 提供有配额的数据和 lifecycle |
| **overlay / window / lifecycle 型** | `Dialog`、`Popover`、`Tooltip`、`Menu`、`HoverCard`、`Notification`、`Sheet`、`Dock`，以及 date/time picker | 尚未映射；需要 Window、focus、anchor、dismiss、subscription 和 mount/unmount 协议，不能伪装成普通 child renderer |
| **capability 型** | `Link` / `href`、Clipboard、`img src`、`avatar src` | `Link` 和 Clipboard 未映射；`img` / `avatar` 目前可把 source 交给原生渲染，但尚未经过网络/文件 capability policy，不适合直接暴露给不可信扩展 |

因此当前实现不声称支持 DataTable、虚拟化 `List<D>`、select/tree delegate、
dialog/popover/tooltip/menu overlay lifecycle、link href 导航，或 chart/plot
delegate/entity。它们需要的是新的运行时合同，不是再注册一个标签名。

### 自定义组件

```rust,ignore
struct SqlEditorComponent;

impl ComponentRenderer for SqlEditorComponent {
    fn render(
        &self,
        props: ComponentProps,
        context: &mut RenderContext<'_>,
    ) -> ComponentResult {
        let editor = gpui::div().child("SELECT 1");
        Ok(context.style(editor, &props).into_any_element())
    }
}

registry.register_with_schema(
    "sql-editor",
    ComponentSchema::new()
        .attribute("readonly")
        .data_attributes(),
    SqlEditorComponent,
)?;
```

Registry 会规范化 tag 大小写和外围空白，并拒绝空 tag 或重复注册。Schema 在 render
前验证允许属性、必填属性和 `data-*` 策略。

组件失败不会让整个声明式 View 消失：

- renderer 返回 `Err(ComponentError)`：
  `ComponentRenderFailed` error + 可见 fallback；
- renderer panic：在组件边界 `catch_unwind`，
  `ComponentPanicked` error + 可见 fallback；
- permissive 模式下未知组件：
  `UnknownTag` warning + 可见 fallback。

自定义 Rust renderer 是 **trusted in-process host code**。panic boundary 只能保护
GPUI render 调用链，不能回滚 renderer 在 panic 前执行的文件、网络或其他外部副作用。
未来 WASM 插件不能直接跨 ABI 注册 `dyn ComponentRenderer`，必须另行设计版本化 ABI、
capability 和资源隔离。

## Tailwind utility 子集

v1 支持：

- 布局：`flex`、`flex-col`、`flex-row`、`flex-1`、
  `flex-shrink-0`；
- 对齐：`items-start`、`items-center`、`items-end`、
  `justify-center`、`justify-between`、`justify-end`；
- 间距：`gap-N`、`p-N`、`px-N`、`py-N`，`0 <= N <= 96`；
- 尺寸：`w-full`、`h-full`、`size-full`、`min-w-0`、
  `min-h-0`；
- 外观：`border`、`rounded-md`、`rounded-lg`、
  `overflow-hidden`、`overflow-y-scroll`；
- 字体：`text-sm`、`text-base`、`text-lg`、`text-xl`、
  `font-semibold`；
- 颜色：有限的 Zinc、Blue、Emerald、White token，可用于
  `bg-*`、`text-*`、`border-*`。

间距使用 v1 固定映射 `N × 4px`。不支持负数、小数、任意值、`NaN`、`inf` 或
大于 96 的 scale。Modifier 严格保留 class source order；多个 setter 冲突时，后
应用的 modifier 按 GPUI builder 语义生效。

`overflow-y-scroll` 只设置纵向 overflow 为 scroll，用于 showcase 的固定 header +
可滚动 main 布局；它不会额外创建原生 `Scrollbar`。需要可见 native thumb 和稳定
`ScrollHandle` 时使用 `<scroll>`。两者都不模拟浏览器 scrollbar CSS、overscroll
或滚动脚本 API。

未知 utility 在 strict 模式产生 error，在 permissive 模式产生 warning。框架不会
静默假装支持完整 Tailwind。

## Diff 的准确边界

GPUI 的声明式 API 不提供浏览器 DOM 那种“取得任意已挂载 Element 后原位修改属性和
children”的通用接口。因此 v1 **不声称直接 patch 已挂载 GPUI Element Tree**。

真实流程：

1. 从 Runtime state 解析 new resolved VNode；
2. 对 old/new resolved VNode 生成 `Patch`；
3. 在 clone 上事务式 `apply_patches`；
4. 成功后替换 View 保存的 resolved VNode；
5. 清理已经移除的 `InputState` 和 `SliderState` cache；
6. `cx.notify()`；
7. GPUI 下一轮 `Render` 重新生成 Element 描述。

`apply_patches` 失败时不会留下半更新 VNode。基础 diff 支持：

- replace；
- text update；
- attributes update；
- classes update；
- child insert；
- child remove。

diff 仍按位置递归。key 不同的同位置节点会产生 Replace；`id` / `key` 的 v1 作用是
保留 stateful component identity，不是实现完整的 keyed move/LCS。以下内容不在 v1
范围：

- 最小编辑距离；
- 完整 keyed reorder patch；
- 跨父节点 move；
- 任意组件 lifecycle/migration ABI。

## Parser 和安全边界

Parser hard-reject：

- `<script>`；
- `<style>`；
- `style="..."`；
- 所有 `on*="..."` HTML event 属性。

HTML5 会把非 void 元素上的 XML-style `/>` 当作普通开始标签。DSL 在交给
html5ever 前会安全展开 `<sql-editor />` 一类自定义标签，同时：

- 不改写原生 void tag；
- 正确跳过引号中的 `>`；
- 跳过 comment；
- 跳过 raw-text / plaintext 内容。

这保证 HTML 本身没有脚本执行入口，但不等于完整的插件安全沙箱。特别是：

- `img src` 和 `avatar src` 都尚未接入 URI、网络或本地文件 capability policy；
  当前 renderer 会把字符串 source 直接交给对应原生 GPUI element；
- Runtime state 没有插件级内存配额；
- 自定义 Rust component 与 Action handler 都是宿主进程内可信代码。

在接入不可信扩展前，必须另行定义 capability、资源协议、WASM ABI、版本协商和
宿主隔离。

## 模块划分

| 模块 | 职责 |
| --- | --- |
| `html_source.rs` | 安全展开自定义 XML-style 自闭合标签 |
| `html_input_adapter.rs` | 常见 HTML `<input type>` → 受限原生 DSL VNode 规范化与 readonly alias 诊断 |
| `parser.rs` | html5ever fragment → 受控 VNode |
| `limits.rs` | 编译资源限制和资源类型 |
| `vnode.rs` | 可 serde 的输入无关 VNode 中间表示 |
| `template.rs` | strict/permissive 编译、schema/class/identity 校验 |
| `diagnostic.rs` | typed、分 phase、去重的 diagnostics |
| `binding.rs` | state → 文本 / attribute resolved VNode 和 missing-binding 诊断 |
| `runtime/` | StateStore、Action、transaction 和 RuntimeEvent |
| `diff.rs` | VNode diff 与事务式 patch |
| `tailwind.rs` | utility token → 语义 modifier |
| `tailwind_style.rs` | modifier → GPUI `Styled` builder |
| `component.rs` | Registry、Schema、Props、Renderer trait |
| `builtin_components.rs` | 默认组件注册入口、共享 attribute parser 和 Action helper |
| `builtin_components/basic.rs` | semantic containers、Button、Input、GroupBox、Label、Tag、Skeleton |
| `builtin_components/forms.rs` | 强结构 Form / Field 与 Checkbox / Switch / Radio |
| `builtin_components/tables.rs` | 强结构静态 Table primitive 组合 |
| `builtin_components/lists.rs` | 静态 list 容器与原生 ListItem |
| `builtin_components/feedback.rs` | Alert、Badge、Progress、Spinner、Separator / divider alias |
| `builtin_components/display.rs` | Avatar / AvatarGroup 与强结构 DescriptionList |
| `builtin_components/navigation.rs` | Breadcrumb、Pagination、Rating、Tabs、Stepper 及数值 selection handler |
| `builtin_components/controls.rs` | Kbd 与单值 Slider 的 schema、数值校验和原生渲染 |
| `builtin_components/layout.rs` | controlled Accordion、controlled Collapsible 与 window-keyed native Resizable 布局 |
| `builtin_components/scroll.rs` | stable-ID 原生 ScrollHandle viewport、Scrollbar overlay 与尺寸/枚举校验 |
| `stateful_nodes.rs` | 有状态输入 spec 与 live identity 收集 |
| `input_cache.rs` | InputState cache、双向 binding subscription |
| `slider_cache.rs` | SliderState cache、双向 binding、Change / Release subscription 与 live identity 清理 |
| `render_context.rs` | 递归组件渲染、style、action/input/slider 与 keyed ScrollHandle 服务 |
| `renderer.rs` | Runtime subscription、reconcile、View Render |
| `main.rs` | standalone 可运行 Demo |

`VNode` 已与 HTML parser 解耦并实现 serde，为 JSON DSL / AI UI 描述保留了稳定中间
层。v1 还没有承诺公开的 JSON compiler API；未来的新 frontend 必须经过与 HTML
等价的资源限制和模板校验，不能绕过 `CompiledTemplate` 直接把不可信树送入 renderer。

## 明确非目标

standalone v1 不实现：

- 浏览器 DOM、WebView 或 HTML inline layout；
- JavaScript、模板表达式或任意代码求值；
- CSS selector、cascade、inheritance、`style`；
- 完整 Tailwind；
- 完整 HTML 标准校验；
- 浏览器 form submission/reset、constraint validation、autocomplete/inputmode，或
  `number` / date-time / file / color 等专用 HTML input 控件；
- 全部 `gpui-component` 的声明式镜像；
- `DataTable<D>`、数据驱动或虚拟化 `List<D>`、select/tree delegate；
- chart/plot delegate 或 entity 数据模型；
- dialog、popover、tooltip、menu 等 overlay/window lifecycle；
- `Link` / `href` 导航或 Clipboard capability；
- 完整 keyed reconciliation；
- Navop extension catalog、插件热重载或 manifest 接入；
- WASM component/action/state ABI；
- 插件 capability sandbox；
- 网络和本地资源授权；
- 任意 typed component lifecycle ABI。

这些限制是产品边界，不是尚未声明却“碰巧不能工作”的行为。接入 Navop 扩展系统前，
应先以本 standalone crate 的 compile/runtime/component contract 为基础设计单独的
版本化插件协议。
