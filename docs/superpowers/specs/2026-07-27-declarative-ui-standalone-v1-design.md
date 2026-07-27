# Rust + GPUI 声明式 UI Standalone v1 架构与实现设计

| 项目 | 内容 |
| --- | --- |
| 状态 | Implemented（standalone v1 已落地） |
| 日期 | 2026-07-27 |
| 适用范围 | `crates/declarative_ui_demo` 的独立声明式 UI 编译、运行时和 GPUI 渲染 |
| 当前 package | `declarative-ui-demo` |
| 运行时 | Rust + GPUI |
| 输入格式 | 受限 HTML；未来可增加 JSON/AI frontend |
| 读者 | standalone runtime 维护者、GPUI 宿主、未来扩展协议设计者 |

> 本文是当前已实现 standalone v1 的架构和行为合同，不是“未来一定会实现”的浏览器
> 或插件平台方案。文中把已经存在的行为称为 **v1 contract**，把尚未实现的能力单独
> 标为 **后续设计**。当前 crate 尚未接入 Navop extension catalog、WASM host、插件
> manifest、capability 系统或热重载生命周期。

## 摘要

`declarative-ui-demo` 将受限 HTML 编译为与输入格式无关的 `VNode`，再通过显式的
模板校验、资源预算、状态绑定、VNode diff 和 Component Registry 生成 GPUI 原生
Element。HTML 只描述结构、Tailwind utility 和 Action 名称，不执行 JavaScript、
模板表达式或 inline event code。

v1 的完整数据流是：

```text
受限 HTML
  ↓ html5ever / markup5ever_rcdom
VNode
  ↓ Compile Validation + Resource Limits + Typed Diagnostics
CompiledTemplate
  ↓ Runtime State Binding
Resolved VNode
  ↓ Diff + Transactional Patch
Stored VNode
  ↓ Component Registry + Tailwind Utility
GPUI Element / Entity<InputState>
```

这个分层有三个目的：

1. **隔离输入语言和渲染层**：未来 JSON DSL 或 AI UI 描述只需生成等价 `VNode`
   并经过同样的校验，不需要复制 GPUI Renderer；
2. **隔离业务逻辑和 UI**：Action 由 HTML 声明、由 Rust Runtime 派发，状态更新
   通过 GPUI Entity 事件驱动 View reconcile；
3. **把“不支持”变成显式合同**：未知 tag、Tailwind utility、组件属性、资源超限
   和运行时失败都有 typed diagnostic 或 typed error，不静默假装兼容完整浏览器。

## 1. 背景与问题

最初的需求是“让插件用 HTML 写 UI，再映射到 GPUI”。如果直接实现一个 HTML
渲染器，很容易滑向一个不可控的浏览器子集：

- HTML parser、DOM、CSS cascade、selector、事件脚本和状态系统互相耦合；
- 任意 HTML 属性都可能变成隐式行为入口；
- 每次状态变化都重新解析和重新创建所有状态；
- 自定义组件没有错误边界，单个组件失败会让整个 View 崩溃；
- 插件边界、资源限制和未来 ABI 会被迫绑定到内部 Rust 类型；
- “支持 Tailwind”容易被误解成支持完整 CSS/Tailwind。

standalone v1 选择一个更窄但可以验证的产品边界：

```text
受限结构描述 + 有界 utility + 显式 Action + 字符串状态
```

而不是实现浏览器。这样可以先稳定编译器、Runtime、组件契约和 GPUI 生命周期，
再决定如何把它暴露给第三方扩展。

## 2. 目标与非目标

### 2.1 v1 目标

- 使用 `html5ever` 和 `markup5ever_rcdom` 解析 HTML fragment；
- 将输入转换为稳定、可序列化、与 HTML 解耦的 `VNode`；
- 只接受明确支持的 Tailwind utility；
- 提供 strict/permissive 两种编译模式；
- 对 source、node、depth、attribute 和 class token 设置硬资源上限；
- 提供带 phase、severity、stable code 和 VNode path 的 typed diagnostics；
- 通过 `ComponentRegistry` 映射默认组件和自定义 Rust 组件；
- 让组件失败和 panic 被隔离为可见 fallback；
- 通过 GPUI `Entity<Runtime>` 提供响应式状态和 Action；
- 保证 Action 的状态更新具备事务性，失败和 panic 可回滚；
- 支持文本绑定以及 `input`/`textarea` 双向绑定；
- 通过 `key`、`id` 或 VNode path 保持有状态输入的 identity；
- 对 old/new resolved VNode 执行 diff，并以事务方式应用 patch；
- 自动响应 Runtime event，重新 reconcile 并通知 GPUI；
- 提供一个不依赖 Navop extension 系统的可运行 Demo；
- 用 contract tests 固定 parser、compiler、runtime、binding、component、input、
  Tailwind 和 diff 的行为。

### 2.2 明确非目标

standalone v1 不实现：

- 浏览器 DOM、WebView 或完整 HTML layout；
- JavaScript、模板表达式、任意代码求值；
- CSS selector、cascade、inheritance 或 `style` 属性；
- 完整 Tailwind 或完整 HTML 标准校验；
- 直接取得并原位修改任意已挂载 GPUI Element Tree；
- 完整 keyed reorder、LCS、最小编辑距离或跨父节点 move；
- 任意自定义组件 lifecycle/migration ABI；
- Navop extension catalog、manifest、插件热重载；
- WASM component/action/state ABI；
- 插件 capability sandbox；
- 网络和本地资源授权；
- 插件级内存、CPU 或执行时间配额；
- `img src` 的网络/文件 capability 策略；
- 任意 typed state schema。

这些是产品边界，不是“尚未声明但碰巧不能工作”的行为。未来接入不可信扩展前，
必须另行设计版本化协议和隔离模型。

## 3. 设计原则

1. **显式能力**：支持的标签、属性和 utility 必须在 registry/schema/parser 中可见；
2. **失败关闭**：编译错误、资源超限、patch 错误和未知 Action 不得静默成功；
3. **状态单向归属**：业务状态归 Runtime，组件局部状态只保留在明确的 stateful
   cache 中；
4. **输入不可执行**：HTML 不承载 Rust/JavaScript 代码；
5. **事务优先**：跨多个 state key 的 Action 只能整体提交或整体丢弃；
6. **有界资源**：任何未来 frontend 都必须经过与 HTML 等价的 limits/validation；
7. **GPUI 诚实边界**：只承诺重新生成 Element 描述，不伪称浏览器式 DOM patch；
8. **可演进但不越权**：VNode 可以被 JSON/AI frontend 复用，但新 frontend 不能绕过
   `CompiledTemplate` 直接把不可信树送进 Renderer。

## 4. 分层架构

```text
┌─────────────────────────────────────────────────────────┐
│ HTML / future JSON / future AI UI frontend              │
└──────────────────────────────┬──────────────────────────┘
                               │ parse / compile
┌──────────────────────────────▼──────────────────────────┐
│ html_source + parser + vnode                            │
│ source normalization → VNode                            │
└──────────────────────────────┬──────────────────────────┘
                               │ validate
┌──────────────────────────────▼──────────────────────────┐
│ limits + template + diagnostic                         │
│ strict/permissive · schema · utility · identity         │
└──────────────────────────────┬──────────────────────────┘
                               │ immutable template
┌──────────────────────────────▼──────────────────────────┐
│ binding + runtime                                       │
│ StateStore · Action · RuntimeEvent · resolved VNode     │
└──────────────────────────────┬──────────────────────────┘
                               │ reconcile
┌──────────────────────────────▼──────────────────────────┐
│ diff + input_cache + stateful_nodes                    │
│ patches · identity · transactional VNode update         │
└──────────────────────────────┬──────────────────────────┘
                               │ render
┌──────────────────────────────▼──────────────────────────┐
│ component + render_context + tailwind_style            │
│ Registry · fallback boundary · GPUI Element             │
└─────────────────────────────────────────────────────────┘
```

| 模块 | 职责 |
| --- | --- |
| `html_source.rs` | 展开自定义 XML-style 自闭合标签，不改写原生 void tag |
| `parser.rs` | HTML fragment → 受控 `VNode`，执行 forbidden input 检查 |
| `vnode.rs` | 输入无关的 `VNode`/`VElement` 数据模型 |
| `limits.rs` | source、node、depth、attribute、class token 预算 |
| `template.rs` | strict/permissive 编译、schema/class/identity 校验 |
| `diagnostic.rs` | typed diagnostics、去重和按 phase 替换 |
| `binding.rs` | Runtime state → resolved VNode |
| `runtime/` | `StateStore`、Action、事务提交和 `RuntimeEvent` |
| `diff.rs` | old/new VNode diff、patch 类型和事务式 apply |
| `tailwind.rs` / `tailwind_style.rs` | utility → modifier → GPUI style |
| `component.rs` / `builtin_components.rs` | Registry、Schema 和默认组件 |
| `stateful_nodes.rs` / `input_cache.rs` | 输入 identity、Entity 和 subscription |
| `render_context.rs` / `renderer.rs` | 递归渲染、reconcile 和 GPUI Render |

## 5. VNode 数据合同

VNode 是 HTML frontend 与运行时/渲染层之间的中间表示。当前公开数据结构为：

```rust
pub enum VNode {
    Element(VElement),
    Text(String),
    Fragment(Vec<VNode>),
}

pub struct VElement {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub classes: Vec<String>,
    pub children: Vec<VNode>,
}
```

`VNode` 和 `VElement` 均实现 `Clone`、`PartialEq`、`Eq`、`Serialize` 和
`Deserialize`。这使 VNode 可以用于测试、diff、调试以及未来的序列化 frontend，
但当前还没有声明稳定的跨版本 VNode wire schema。

### 5.1 结构语义

- `Element` 保存规范化后的 tag、非 `class` 属性、class token 和子节点；
- `Text` 保存经过空白规范化的可渲染文本；
- `Fragment` 表示 HTML fragment 中存在多个根节点；
- `attrs` 使用 `BTreeMap`，因此提供确定性遍历顺序，但不承诺保留源属性顺序；
- `classes` 使用 `Vec<String>`，保留源码中的 token 顺序，以支持顺序应用 modifier；
- comment、doctype 和 processing instruction 不进入 VNode；
- 只有一个根节点时直接返回该节点，多个根节点时使用 `Fragment`，没有可渲染节点
  时返回 `HtmlParseError::EmptyFragment`。

### 5.2 Identity 语义

`VElement::key()` 的优先级为：

```text
key → id → None
```

组件的 stateful identity 则是：

```text
normalized tag + ":" + (key → id → VNode path)
```

`key` 和 `id` 的显式值在一个已编译模板内共享同一个全局命名空间；即使一个值来自
`id`、另一个来自 `key`，重复也会产生 `DuplicateIdentity` 编译错误。

`NodePath(Vec<usize>)` 是从根节点开始的子节点位置，例如 `root.1.0`。它适合：

- diagnostic 定位；
- diff patch 定位；
- 没有显式 `key`/`id` 时的本地 fallback identity。

它**不是**跨模板版本的永久标识。兄弟节点插入、删除或重排会改变后续节点的 path；
需要保持输入状态的节点应声明唯一 `key` 或 `id`。

## 6. HTML 解析与模板编译

### 6.1 解析入口

低层解析 API 是：

```rust
pub fn parse_html(source: &str) -> Result<VNode, HtmlParseError>;

pub fn parse_html_with_limits(
    source: &str,
    limits: CompileLimits,
) -> Result<VNode, HtmlParseError>;
```

正式宿主入口应使用：

```rust
pub fn compile_template(
    source: &str,
    registry: &ComponentRegistry,
    options: CompileOptions,
) -> Result<CompiledTemplate, TemplateCompileError>;
```

`CompiledTemplate` 保存：

- 原始 `source`；
- 编译后的 `root: VNode`；
- 编译阶段产生的 `Diagnostics`。

字段保持私有，通过 `source()`、`root()` 和 `diagnostics()` 读取。这可以防止宿主在
绕过编译校验的情况下直接构造“已编译”模板。

### 6.2 自闭合标签规范化

HTML5 对未知元素的 `<sql-editor />` 不采用 XML 自闭合语义。为满足声明式组件使用
习惯，`html_source.rs` 在进入 `html5ever` 前将非 void tag：

```html
<sql-editor />
```

安全地展开为：

```html
<sql-editor></sql-editor>
```

扫描器会处理引号、comment、ignored markup 和 raw-text element，避免把属性字符串
中的 `/>` 当作标签结束；`img`、`input`、`br` 等 HTML void tag 不会被改写。
原始 source byte limit 在展开前检查，因此内部规范化不会放大调用方可提交的输入
预算。

这一步只解决 custom element 自闭合语义，不把输入转换为 XML，也不改变
`html5ever` 对一般 HTML fragment 的错误恢复规则。

### 6.3 硬拒绝输入

Parser 在任何 validation mode 下都拒绝：

- `<script>`；
- `<style>`；
- `style="..."`；
- 所有名称以 `on` 开头的属性，例如 `onclick`、`oninput`。

这些输入返回 typed `HtmlParseError`，不会降级为 permissive warning。HTML 中没有
任何 JavaScript、Rust 代码或模板表达式执行入口。

### 6.4 Strict 与 permissive

`CompileOptions` 提供 `strict()` 和 `permissive()`，并可通过 `with_limits(...)`
覆盖默认资源预算。

| 情况 | Strict | Permissive |
| --- | --- | --- |
| 未注册 tag | `UnknownTag` error，编译失败 | `UnknownTag` warning，保留节点 |
| 不支持的 Tailwind class | `UnsupportedClass` error，编译失败 | warning，保留 token |
| 不支持/缺失的组件属性 | error，编译失败 | error，编译失败 |
| 空 `action`/`bind`、`bind` 与 `value` 冲突 | error，编译失败 | error，编译失败 |
| 重复或空显式 identity | error，编译失败 | error，编译失败 |
| `script`/`style`/`style=`/`on*` | parser hard failure | parser hard failure |
| 任意资源预算超限 | parser hard failure | parser hard failure |

因此 permissive 只允许宿主在已知风险下保留“暂未支持的 tag/utility”，不能绕过
安全限制、资源限制或组件 schema。

## 7. 资源限制与性能边界

### 7.1 默认预算

| 资源 | 默认上限 | 计数语义 |
| --- | ---: | --- |
| 原始 source | 256 KiB | 原始 UTF-8 byte 数，规范化前检查 |
| node | 10,000 | element 加非空 text node 的文档总量 |
| element depth | 64 | 根 element 深度为 1 |
| attribute | 20,000 | 原始 HTML attribute 的文档总量，包含 `class` |
| class token | 20,000 | `class` 按空白拆分后的文档总量 |

预算由 `CompileLimits` 显式承载。超过上限会返回：

```rust
HtmlParseError::ResourceLimitExceeded {
    resource,
    limit,
    actual,
}
```

所有计数使用 checked policy：只有本次增量仍在预算内时才更新 counter。宿主可针对
较小的插件面板调低预算，不应因为使用 permissive 模式而提高或取消预算。

### 7.2 成本模型

当前实现以简单、可验证的树操作为主：

- parser、validation、binding 和 render 都需要遍历相关树；
- diff 对 old/new tree 做位置递归；
- `apply_patches` 先 clone 整个 stored VNode，再在 candidate 上应用全部 patch；
- reconcile 成功后才替换 stored VNode；
- GPUI `Render` 会根据 stored VNode 重新生成 Element 描述。

因此 v1 的“增量”主要体现在：

- 可以观察具体 patch；
- patch 批次具有事务性；
- stateful `InputState` Entity 能跨 render 复用；
- 不需要重新解析 HTML 或重建 Runtime。

它不承诺 zero-copy tree update，也不承诺浏览器 DOM 级别的 layout/paint 增量性能。
在扩大默认预算、引入高频状态流或接入不可信扩展之前，应增加基准测试，并分别测量
解析、binding、diff、VNode clone、组件渲染和 GPUI layout 的成本。

## 8. Typed Diagnostics

每条 `Diagnostic` 包含：

```text
severity  Error | Warning
phase     Compile | Binding | Render | Runtime
code      稳定的枚举代码
message   面向开发者的说明
path      可选 NodePath
span      可选 SourceSpan
```

当前 phase 的职责是：

| Phase | 典型来源 |
| --- | --- |
| `Compile` | unknown tag/class、schema、identity |
| `Binding` | 缺失 state key |
| `Render` | unknown component、renderer error/panic |
| `Runtime` | Action 失败、reconcile 失败 |

`Diagnostics` 按完整 `Diagnostic` 值精确去重；相同 code/message 但不同 path 的诊断会
分别保留。`replace_phase` 会先删除指定 phase 的旧条目及其去重索引，再加入本轮结果，
所以重复 reconcile/render 不会无限累积已过期 warning。

`SourceSpan` 已作为公开合同预留，但当前 `html5ever` + `RcDom` 转换没有提供可依赖的
原始 source offset，所有现有诊断的 `span` 为 `None`。在没有可靠映射前，不应伪造
近似 offset；未来若更换 parser adapter，应通过 contract test 验证 UTF-8 byte
边界、自闭合规范化前后映射和错误恢复场景。

## 9. Runtime、State 与 Action

### 9.1 StateStore

v1 状态模型有意保持简单：

```rust
pub struct StateStore {
    values: BTreeMap<String, String>,
}
```

公开操作为 `get`、`set` 和 `remove`。`set`/`remove` 返回值表示内容是否实际变化。
Runtime 还维护单调递增的 `revision`；只有状态真正提交时 revision 才增加。

Runtime 应由 GPUI 持有为：

```rust
Entity<Runtime>
```

外部状态更新使用 `Runtime::set` 或 `Runtime::transaction`，并在 GPUI
`Entity::update` 提供的 `Context<Runtime>` 中执行。状态提交会发出
`RuntimeEvent::StateChanged`，其中包含：

- 新 revision；
- `changed_keys`；
- `StateChangeOrigin::External` 或具体 Action 来源。

### 9.2 ActionEvent

UI 只声明 Action 名称：

```html
<button id="save" action="save" data-record="profile">保存</button>
```

Button 将其转换为结构化 `ActionEvent`：

```text
name         "save"
source_id    stable component id
source_path  VNode path
payload      所有 data-* 属性，去掉 "data-" 前缀
```

HTML 不决定 handler 实现，也不能传入可执行代码。

### 9.3 注册和同步执行合同

Action 注册 API 是同步闭包：

```rust
runtime.on("save", |ctx| {
    ctx.set("status", "saved");
    Ok(())
})?;
```

内部 handler 类型等价于：

```rust
Rc<dyn Fn(&mut ActionContext<'_>) -> Result<(), ActionError>>
```

v1 没有 async Action handler API。重复注册同名 Action 返回
`RuntimeError::DuplicateAction`，并保留原 handler，不执行覆盖。

`ActionContext` 只暴露当前 event 和事务候选 `StateStore` 的 `get`、`set`、
`remove`；handler 不直接持有 GPUI `Context`。

### 9.4 事务与事件顺序

dispatch 时，Runtime：

1. clone 当前 `StateStore`；
2. 在 clone 上执行 handler；
3. 捕获 handler 返回错误或 panic；
4. 只有 handler 成功时才比较并提交最终状态；
5. 一个 Action 最多产生一次 revision 增量。

成功且有状态变化时，事件顺序固定为：

```text
StateChanged → ActionCompleted
```

成功但无状态变化时：

```text
ActionCompleted
```

此时 revision 不变，`ActionOutcome::state_changed == false`。

handler 返回 `ActionError`、panic 或 Action 未注册时：

- 候选状态整体丢弃；
- revision 不变；
- 发出 `ActionFailed`；
- `dispatch` 返回 typed `RuntimeError`；
- View 记录 Runtime phase diagnostic 并保持上次成功渲染的状态。

panic boundary 只保护 Runtime 内部事务一致性，不能撤销 handler 已经执行的文件写入、
网络请求或其他外部副作用。因此 Action handler 仍属于可信宿主代码。

### 9.5 异步工作边界

长耗时 I/O 不应放在同步 Action handler 内阻塞 GPUI。推荐流程是：

```text
Action 启动 job / 记录 loading state
  → 合适的 async executor 执行 I/O
  → 回到 GPUI App/Entity context
  → Runtime::set 或 Runtime::transaction
  → StateChanged 驱动 reconcile
```

具体 executor 必须根据 Future 是否依赖 Tokio runtime 选择；无论使用哪种 executor，
最终状态 mutation 都必须回到合法的 GPUI context。

## 10. Binding、有状态输入与 Identity

### 10.1 单向文本绑定

非输入组件可以声明：

```html
<span bind="username"></span>
```

binding resolution 保留原 element 的 tag、attrs 和 classes，但把 children 替换为当前
state value 的单个 `Text` 节点。这样样式和组件 shell 不会因为状态变化而丢失。

若 key 不存在：

- resolved value 为 `""`；
- 产生 `Binding / MissingBinding` warning；
- warning 带对应 `NodePath`；
- 后续 state key 出现时，下一次 reconcile 会更新内容，并通过 phase replacement
  自动清除旧 warning。

### 10.2 `input` / `textarea` 双向绑定

```html
<input id="username" bind="username" placeholder="用户名" />
<textarea key="notes" bind="notes"></textarea>
```

Runtime state → input 的方向在 binding 阶段将 state value 写入 resolved VNode 的
`value` 属性。input → Runtime 的方向由 `InputState` subscription 完成：

1. 监听 `InputEvent::Change`；
2. 读取 `InputState::value()`；
3. 使用 `cx.defer` 避免在当前事件回调中重入；
4. 通过 `Entity<Runtime>::update` 调用 `Runtime::set`；
5. no-op `set` 不发出新事件，避免值同步循环。

`bind` 与字面 `value` 同时声明会产生 `ConflictingAttributes` 编译错误。

### 10.3 InputState cache

有状态输入不能在每次 GPUI `Render` 时都创建新的 `Entity<InputState>`，否则会丢失：

- 光标和 selection；
- composition；
- focus 相关内部状态；
- input event subscription。

`InputCache` 按 stable component id 保存：

```text
Entity<InputState>
StatefulInputSpec
Subscription（绑定输入才存在）
```

identity 优先级为：

```text
key → id → 当前 VNode path
```

行为合同如下：

| 变化 | InputState 处理 |
| --- | --- |
| 只有 bound `value` 变化 | 复用 Entity，调用 `set_value` 同步 |
| `placeholder` 变化 | 新建 Entity |
| `bind` key 变化或新增/移除 | 新建 Entity 和 subscription |
| `input` ↔ `textarea` / multiline 变化 | 新建 Entity |
| 未绑定 input 的字面 `value` 配置变化 | 新建 Entity |
| 节点移除 | reconcile 后删除 cache entry 和 subscription |
| 有显式 key/id 的节点仅位置变化 | stable id 不变，可复用 Entity |

替换 entry 时旧 `Entity`/`Subscription` 随旧 entry 释放；删除节点时
`retain_live` 根据最新 resolved VNode 清理已不再存在的 identity。

没有显式 identity 的输入使用 path，只适合结构稳定的局部模板。v1 尚未实现跨父节点
move 或通用 keyed list reconciliation。

## 11. Component Registry

### 11.1 核心 Trait 和类型

```rust
pub trait ComponentRenderer: 'static {
    fn render(
        &self,
        props: ComponentProps,
        context: &mut RenderContext<'_>,
    ) -> ComponentResult;
}

pub type ComponentResult = Result<AnyElement, ComponentError>;
```

`ComponentProps` 包含当前 `VElement` 和 `NodePath`，并提供 `stable_id()`。
`RenderContext` 向组件提供受控服务：

- 递归渲染 children；
- 应用 Tailwind modifier；
- 取得/创建 stateful input；
- 创建 Action dispatcher；
- 记录 render diagnostic 和 unsupported-class warning。

组件不需要直接理解 HTML parser、binding 或 diff。

### 11.2 Schema

`ComponentSchema` 支持：

- `attribute(name)`：允许可选属性；
- `required_attribute(name)`：要求非空属性；
- `data_attributes()`：允许任意 `data-*`；
- 全局允许 `id` 和 `key`。

Schema 在 `compile_template` 阶段执行，而不是等到点击或渲染时才发现错误。当前还会
统一校验：

- 空 `action`；
- 空 `bind`；
- 同时存在 `bind` 和 `value`。

Registry 对 tag 做 `trim + ASCII lowercase` 规范化。空 tag 返回
`RegistryError::EmptyTag`；规范化后重复返回 `AlreadyRegistered`，不会覆盖原组件。

### 11.3 默认组件

| Tag | 当前实现 | 主要属性 |
| --- | --- | --- |
| `div` | GPUI `div` container | `bind` |
| `span` | GPUI `div` container，用于文本/子节点 | `bind` |
| `button` | `gpui_component::button::Button` | `action`、`data-*` |
| `input` | `gpui_component::input::Input` | `bind`、`placeholder`、`value` |
| `textarea` | multiline `Input` | `bind`、`placeholder`、`value` |
| `img` | GPUI `img` | 必填非空 `src` |

`img src` 当前只是传给 GPUI image element；v1 没有定义 URL、文件路径、协议或扩展
asset capability 策略，因此不应直接把不可信扩展提供的任意 URI 视为已授权资源。

### 11.4 自定义组件

```rust
struct SqlEditorComponent;

impl ComponentRenderer for SqlEditorComponent {
    fn render(
        &self,
        props: ComponentProps,
        context: &mut RenderContext<'_>,
    ) -> ComponentResult {
        let editor = gpui::div().child("SELECT * FROM connections;");
        Ok(context.style(editor, &props).into_any_element())
    }
}

let mut registry = ComponentRegistry::with_defaults();
registry.register("sql-editor", SqlEditorComponent)?;
```

若组件需要属性合同，应优先使用 `register_with_schema`，而不是在 renderer 中静默忽略
未知输入。

### 11.5 错误边界

每次组件 render 都由边界包裹：

- 返回 `Err(ComponentError)` → `ComponentRenderFailed`；
- panic → `ComponentPanicked`；
- permissive 模式保留的未知 tag → Render phase `UnknownTag` warning。

上述情况都会生成一个可见的 fallback element：

```text
component <tag> failed: <message>
```

单个组件失败不会直接终止整个 View 的递归渲染。与 Action 相同，panic boundary 不能
撤销组件在 panic 前发生的外部副作用，也不能限制 CPU、内存、文件系统或网络访问。
自定义 Rust renderer 是可信的进程内代码，不是插件 sandbox。

## 12. Tailwind Utility 子集

v1 不解析 CSS。class token 按源码顺序转换为 `TailwindModifier`，再顺序调用 GPUI
`Styled` API。

### 12.1 已支持 utility

| 类别 | Utility |
| --- | --- |
| Flex | `flex`、`flex-col`、`flex-row`、`flex-1`、`flex-shrink-0` |
| Cross-axis | `items-start`、`items-center`、`items-end` |
| Main-axis | `justify-center`、`justify-between`、`justify-end` |
| Spacing | `gap-N`、`p-N`、`px-N`、`py-N`，其中 `0 <= N <= 96` |
| Size | `w-full`、`h-full`、`size-full`、`min-w-0`、`min-h-0` |
| Overflow | `overflow-hidden` |
| Border/radius | `border`、`border-<color>`、`rounded-md`、`rounded-lg` |
| Background | `bg-<color>` |
| Text color | `text-<color>` |
| Text size | `text-sm`、`text-base`、`text-lg`、`text-xl` |
| Font | `font-semibold` |

颜色 token：

```text
zinc-950  zinc-900  zinc-800  zinc-700  zinc-400  zinc-100
blue-600  emerald-400  white
```

`text-*` 先匹配固定字号，再匹配颜色，因此 `text-lg` 是字号、`text-zinc-100` 是颜色。

### 12.2 数值和顺序语义

spacing 采用 Tailwind 基础比例：

```text
N × 4 px
```

例如：

```text
gap-2 → gap(8 px)
p-4   → padding(16 px)
```

小于 0、非整数、解析溢出或大于 96 的值都视为 unsupported class。modifier 保留
source order，因此：

```html
class="p-2 p-4 flex-col flex-row"
```

按该顺序应用，后设置的同类 GPUI style 通常覆盖前值。这里没有 CSS specificity、
selector、cascade、responsive variant、state variant、arbitrary value、plugin
utility 或 theme expansion。

Strict 编译把 unsupported class 视为 error；permissive 编译保留 warning，render
时不会应用该 token，并在 View 的 `warnings()` 中记录组件 stable id 和 class。

## 13. Diff 与 Reconcile

### 13.1 Patch 模型

公开 patch 类型为：

```text
Replace
SetText
UpdateAttributes
UpdateClasses
InsertChild
RemoveChild
```

每个 patch 由 `NodePath` 定位。`diff(old, new)` 的基本规则：

- 两个 Text 内容不同 → `SetText`；
- 两个 Element 的 tag 和 `key()` 相同 → 比较 attrs、classes，并按位置递归 children；
- 两个 Fragment → 按位置递归 children；
- 节点 kind、tag 或 explicit identity 不匹配 → `Replace`；
- old 多出的尾部 children 逆序删除；
- new 多出的尾部 children 顺序插入。

当前算法不是完整 keyed list diff。它不会做 LCS、兄弟节点 move、跨父节点 move 或
最小 patch 优化；key 只参与“同一位置的两个 element 是否可继续递归”的判断。

### 13.2 事务式 patch apply

`apply_patches` 不直接在调用方树上逐条提交：

```text
clone root → apply all patches to candidate → success: replace root
                                      └────→ failure: discard candidate
```

不存在 path 返回 `DiffError::InvalidPath`；patch kind 与目标节点不匹配返回
`DiffError::KindMismatch`。任一 patch 失败，原树保持不变。

### 13.3 DeclarativeView reconcile 生命周期

状态变化后的完整顺序是：

```text
RuntimeEvent::StateChanged
  ↓
用最新 StateStore 解析 template bindings
  ↓
替换 Binding phase diagnostics
  ↓
diff(stored rendered VNode, next resolved VNode)
  ↓
clone stored VNode + transactional apply
  ↓
提交 stored VNode
  ↓
清理已移除 input identity 的 cache/subscription
  ↓
记录 last_patches
  ↓
cx.notify()
```

下一次 GPUI `Render`：

1. 清空旧 Render phase diagnostics 和普通 warnings；
2. 从 stored VNode 递归调用 Component Registry；
3. 复用可复用的 `Entity<InputState>`；
4. 重新生成 GPUI Element 描述。

因此 v1 的准确边界是：

```text
old/new resolved VNode
  → diff
  → transactionally patch cloned stored VNode
  → commit stored VNode
  → notify GPUI
  → next Render regenerates Element descriptions
```

它**不会**取得一个已挂载的任意 GPUI Element Tree 并在原对象上执行 DOM 式 patch。
真正跨 Render 保持的有状态对象由明确的 cache 管理，目前主要是 `InputState`。

## 14. 错误与恢复矩阵

| 阶段 | 失败示例 | 对当前状态/树的影响 | 可见结果 | 恢复方式 |
| --- | --- | --- | --- | --- |
| Parse | 空 fragment、forbidden input | 不产生模板 | `HtmlParseError` | 修正 source 后重新编译 |
| Limits | source/node/depth/attrs/classes 超限 | 不产生模板 | `ResourceLimitExceeded` | 减小输入或显式调整预算 |
| Compile | schema、identity、strict unknown utility/tag | 不产生模板 | `TemplateCompileError::Validation` | 修正模板/registry 后重新编译 |
| Permissive compile | unknown utility/tag | 产生模板 | Compile warning | 注册组件/支持 utility，或接受 fallback |
| Binding | state key 缺失 | resolved value 为空字符串 | Binding warning | 设置 key 后自动 reconcile 并清除 warning |
| Render | renderer 返回 `Err` | 其他节点继续 render | fallback + Render error | 修复组件；下一轮 Render 重建 phase 诊断 |
| Render | renderer panic | 其他节点继续 render | fallback + `ComponentPanicked` | 修复可信组件；panic 前副作用不可回滚 |
| Runtime | unknown Action | state/revision 不变 | `ActionFailed` + Runtime error | 注册 Action 或修正模板名称 |
| Runtime | handler error/panic | 候选 state 整体回滚 | `ActionFailed` + Runtime error | 修复 handler；后续成功 Action 清除旧 Action error |
| Reconcile | invalid patch/path/kind | stored VNode 保持上次成功值 | `ReconciliationFailed` | 修复内部 diff/apply 缺陷并调用 `refresh`/触发状态更新 |

`DeclarativeView::last_error()` 提供当前 Runtime/reconcile 错误文本，Render 还会把它附加
到 root 作为可见信息。`diagnostics()` 是结构化处理的首选接口，`warnings()` 主要用于
render 时 unsupported class 的简化文本记录。

Compile failure 不应被转换为空白 View；宿主应在创建 `DeclarativeView` 前处理
`TemplateCompileError`。运行时错误则保留最后一次成功提交的 VNode/State，并允许
后续事件恢复。

## 15. 宿主集成合同

### 15.1 最小挂载顺序

```rust
use declarative_ui_demo::{
    CompileOptions, ComponentRegistry, DeclarativeView, DeclarativeViewConfig,
    Runtime, StateStore, compile_template,
};

let registry = ComponentRegistry::with_defaults();
let template = compile_template(
    r#"<span bind="status"></span>"#,
    &registry,
    CompileOptions::strict(),
)?;

let mut initial_state = StateStore::default();
initial_state.set("status", "ready");

// 以下代码位于合法的 GPUI App context 内。
let runtime = cx.new(|_| Runtime::new(initial_state));
let config = DeclarativeViewConfig::new(template, runtime.clone(), registry);
let view = cx.new(|cx| DeclarativeView::new(config, cx));
```

之后宿主把 `view` 挂入普通 GPUI View/`Root` 即可。若使用默认
`gpui-component` 组件，应按应用既有流程初始化 `gpui_component` 和 assets。

### 15.2 Ownership 与生命周期

- `DeclarativeViewConfig` 按值接收 `CompiledTemplate`、`Entity<Runtime>` 和
  `ComponentRegistry`；
- `DeclarativeView::new` 立即解析初始 binding，并订阅 Runtime event；
- subscription 由 View 字段持有，View 释放时 subscription 随之释放；
- `InputCache` 由单个 View 拥有，不是全局 cache；
- Runtime 可以被宿主与多个合法订阅者共享，但每个 View 保存自己的 resolved VNode、
  diagnostics 和 input cache；
- Registry 和 template 在 View 构造后按该 View 的 snapshot 使用。v1 没有 mounted
  registry mutation、模板 hot-swap 或 component migration API。

### 15.3 Context 与线程约束

- `Entity<Runtime>` 通过 GPUI `AppContext::new` 创建；
- Runtime mutation 通过 `Entity::update` 进入正确 context；
- `Runtime::set`、`transaction`、`dispatch` 负责 emit + notify；
- Button dispatcher 已回到 `App` context 后再 dispatch；
- `InputState` 的创建和 `set_value` 需要当前 `Window` 与 `App`；
- 后台任务不能在线程上直接修改 Entity，必须回到 GPUI context。

Action handler 当前是同步、进程内调用。需要数据库、网络、文件、进程等异步能力时，
宿主负责 job 生命周期、取消和 executor 选择；Runtime 只接收回到前台后的结果状态。

## 16. 最小可运行 Demo

运行：

```bash
cargo run -p declarative-ui-demo
```

Demo 展示：

- standalone GPUI window，不依赖 Navop extension catalog；
- restricted HTML 模板；
- 默认 `div`、`span`、`button`、`input`；
- 自定义 `<sql-editor />`；
- Tailwind layout、spacing、color、border、radius 和 text utility；
- `username` 双向输入绑定；
- `status`、`save_count` 文本绑定；
- `save` Action；
- `data-record="profile"` 结构化 payload；
- 一次 Action 内多个 state key 的事务式更新；
- Runtime event 触发的自动 reconcile。

点击“保存”后，handler 读取当前 username、递增计数并更新状态文字。整个过程没有
JavaScript，也不要求组件直接操作 `DeclarativeView`。

Demo 能启动并保持运行只能证明应用初始化和主事件循环未立即 panic；正式视觉验收还应
手工验证窗口缩放、输入、中文 composition、焦点、按钮点击、错误 fallback 和不同平台
的外观。

## 17. 测试与验收矩阵

| 测试文件 | 主要合同 |
| --- | --- |
| `tests/contracts.rs` | HTML→VNode、自闭合组件、安全拒绝、Tailwind、binding、diff/apply |
| `tests/v1_contracts.rs` | strict/permissive、schema、identity、payload、事务、自动 reconcile |
| `tests/runtime_contracts.rs` | duplicate registration、panic rollback、事件顺序、no-op Action |
| `tests/binding_contracts.rs` | 缺失 binding、自动清 warning、phase replacement |
| `tests/component_contracts.rs` | 默认 registry、自定义组件、tag 规范化和重复保护 |
| `tests/component_boundary_contracts.rs` | renderer error/panic、permissive unknown fallback diagnostics |
| `tests/limits_contracts.rs` | 五类资源预算、计数语义、permissive hard limit |
| `tests/tailwind_contracts.rs` | spacing 边界、非法数值、modifier source order |
| `src/input_cache_tests.rs` | 双向写回、值同步、key 重排、配置替换、旧 subscription 释放、节点清理 |

当前 crate 同时包含普通 unit/contract test 和 `#[gpui::test]`，后者使用 GPUI
`test-support` 验证 Entity、event、subscription 和真实 View render 边界。

交付前最小验证集合：

```bash
cargo fmt --all -- --check
cargo test -p declarative-ui-demo
cargo check -p declarative-ui-demo --all-targets
cargo clippy -p declarative-ui-demo --all-targets -- -D warnings
git diff --check
```

对于修改 GPUI 生命周期、input cache 或组件 fallback 的变更，还应保留/增加真实
`VisualTestContext` 或布局测试；对于视觉行为变化，应额外执行手工 UI 验收。

## 18. 安全与信任模型

### 18.1 受限但仍不等于 sandbox 的 HTML

standalone v1 可把 HTML source 当作不可信结构输入，前提是宿主使用
`compile_template` 并保留合理的 `CompileLimits`。当前防线包括：

- 不执行 JavaScript/模板表达式；
- hard-reject `script`、`style`、`style=` 和 `on*`；
- Registry/schema allowlist；
- Tailwind utility allowlist；
- source/tree/attribute/class 资源预算；
- typed compile failure；
- 组件 render 和 Action panic boundary。

这些防线限制“描述语言能表达什么”，但不构成进程隔离。

### 18.2 可信进程内代码

以下对象必须视为可信宿主代码：

- `dyn ComponentRenderer`；
- `Rc<dyn Fn(&mut ActionContext) -> Result<...>>`；
- 创建/修改 `Entity<Runtime>` 的 Rust 代码；
- renderer 或 handler 调用的任意外部库。

它们与 Navop/宿主进程共享：

- 内存和线程；
- 文件系统权限；
- 网络权限；
- CPU；
- GPUI context；
- 进程崩溃域（除被明确 catch 的局部 panic）。

错误/panic boundary 只提供局部故障降级和状态事务保护，不能防止死循环、abort、
内存耗尽、unsafe UB、系统调用或已经提交的外部副作用。

### 18.3 尚未提供的隔离

v1 没有：

- capability grant；
- WASM/process sandbox；
- 文件/网络 URI policy；
- CPU/内存/时间配额；
- async job timeout/cancellation；
- plugin signing/trust policy；
- 跨进程序列化协议。

因此可以独立完善 standalone 机制，但在这些问题解决前，不应把任意第三方 Rust
component/action 动态加载进主进程。

## 19. Public API 与版本策略

当前 package：

```toml
name = "declarative-ui-demo"
version = "0.1.0"
```

名称仍带 `demo`，版本仍处于 `0.x`。本文定义的是当前 standalone v1 的行为合同，
不是永久 Rust ABI、C ABI 或跨进程 wire compatibility 承诺。进入扩展系统前可以根据
职责拆 crate、收窄 API 或重命名 package，但应通过迁移说明和 contract tests 保护
已公开行为。

### 19.1 当前公开 API 面

crate root 主要 re-export：

- VNode：`VNode`、`VElement`；
- 编译：`CompileOptions`、`ValidationMode`、`CompiledTemplate`、
  `compile_template`；
- parser/limits：`parse_html*`、`CompileLimits`、`HtmlParseError`；
- diagnostics：`Diagnostic*`、`Diagnostics`、`SourceSpan`；
- component：`ComponentRegistry`、`ComponentSchema`、`ComponentRenderer`、
  `ComponentProps`、`RenderContext`；
- runtime：`Runtime`、`StateStore`、`Action*`、`RuntimeEvent`；
- binding：`resolve_bindings*`；
- diff：`NodePath`、`Patch*`、`diff`、`apply_patches`；
- Tailwind：`TailwindModifier`、`parse_classes`、`apply_modifiers`；
- GPUI View：`DeclarativeViewConfig`、`DeclarativeView`。

部分 modules 保持私有，只通过受控 re-export 暴露类型；`CompiledTemplate` 内部字段也
保持私有。未来调整 public surface 时，应优先保留高层入口，而不是要求每个宿主手工
拼接 parser、validator、binding 和 renderer。

### 19.2 优先保持稳定的行为合同

在 pre-1.0 演进中，以下行为应视为高价值兼容面：

- HTML 不执行代码；
- forbidden input 与资源预算不能被 permissive 绕过；
- strict/permissive 的支持项降级语义；
- Action 成功最多提交一次、失败/panic 整体回滚；
- `StateChanged → ActionCompleted` 的成功事件顺序；
- typed diagnostic phase/code/path；
- `key → id → path` 的输入 identity 优先级；
- failed patch batch 不修改原 VNode；
- Component error/panic 产生 fallback，而不是直接展开 panic；
- GPUI 边界始终如实描述为“VNode reconcile + next Render”，而不是 DOM patch。

新增 enum variant、utility、component 属性或 diagnostic code 通常可以是向后兼容扩展；
改变现有 utility 数值、事件顺序、identity 规则、rollback 语义或安全拒绝规则，则需要
显式版本评估和迁移测试。

## 20. 未来 JSON DSL / AI UI Frontend

VNode 与 HTML 解耦，且已实现 serde，因此可以支持新的 source frontend；但
“能反序列化 VNode”不代表“可以跳过编译器”。

未来 frontend 的推荐合同是：

```text
JSON / AI response
  ↓ source/decode limits
untrusted frontend AST
  ↓ normalize
VNode candidate
  ↓ tree limits + schema + utility + identity validation
CompiledTemplate-equivalent artifact
  ↓ binding / diff / component render
GPUI
```

必须满足：

1. 在大规模 allocation 前限制 source/message bytes；
2. 对反序列化后的 node、depth、attribute 和 class token 重新计数；
3. 使用同一个 `ComponentRegistry` schema；
4. 使用同一个 Tailwind allowlist；
5. 使用同一个 identity namespace；
6. 产生同结构的 typed diagnostics；
7. 不能允许调用方直接伪造“已验证”的 `CompiledTemplate`；
8. AI 生成内容始终作为不可信描述数据处理。

实现上可抽取一个 input-independent `compile_vnode(candidate, registry, options)` 内核，
由 HTML/JSON/AI adapter 调用。该 API 尚未实现；在实现前不要复制一套行为不同的
validator。

## 21. 接入扩展系统前的协议前置条件

standalone v1 的进程内 Rust 类型不适合作为插件 ABI。尤其不能直接跨 WASM、动态库
或进程边界暴露：

```rust
dyn ComponentRenderer
Rc<dyn Fn(&mut ActionContext<'_>) -> Result<(), ActionError>>
Entity<Runtime>
```

这些类型包含 Rust trait object、`Rc`、借用生命周期和 GPUI runtime ownership，
既不可稳定序列化，也不是 ABI-safe。

正式扩展协议至少需要：

### 21.1 版本化数据协议

- manifest 中声明 declarative UI protocol version；
- VNode/template schema version；
- 可序列化的 state snapshot/delta；
- 可序列化的 Action event/result/error；
- stable diagnostic code 和可选 source location；
- feature/capability negotiation；
- unknown field/unknown enum 的前后兼容策略。

### 21.2 组件模型

需要明确区分：

1. **宿主内建组件**：插件只引用 versioned tag/schema；
2. **可信原生组件**：通过明确的本地 trust policy 载入；
3. **不可信扩展组件**：通过 WASM/process protocol 或受限绘制/数据模型提供，不能
   直接传 Rust `ComponentRenderer`。

还需要定义组件实例 identity、create/update/dispose、hot reload、状态迁移和错误隔离。

### 21.3 Action 和异步 job

扩展 Action 协议应支持：

- request id；
- timeout；
- cancellation；
- progress；
- structured result/error；
- state patch 的原子提交；
- 幂等/重复消息策略；
- 插件退出或重载时的 pending job 清理。

### 21.4 Capability 与资源隔离

至少定义：

- 网络域名/端口授权；
- 文件和 workspace path 授权；
- image/asset URI scheme；
- clipboard、process、terminal、database 等 capability；
- source/tree/state/message 大小；
- CPU、内存、执行时间和并发 job 配额；
- crash/restart 和熔断策略。

只有完成这些协议后，standalone Runtime 才适合成为 Navop extension UI host 的内部
实现组成，而不是把当前进程内 API 原样公开给第三方插件。

## 22. 已知限制与开放问题

| 主题 | 当前状态 | 后续需要决定 |
| --- | --- | --- |
| Typed state | 只有 `String → String` | bool/number/list/object、schema、序列化和 patch 语义 |
| VNode version | serde 可用但无 schema version | wire schema、兼容策略、migration |
| Source span | 字段存在，值为 `None` | parser offset、UTF-8、规范化映射 |
| Diff | 位置递归、尾部 insert/remove | keyed list、move、复杂度阈值 |
| GPUI patch | 重新生成 Element 描述 | 是否需要更细的 stateful component lifecycle |
| Image | `src` 直接交给 GPUI | asset registry、URI allowlist、缓存、权限 |
| Async Action | 无 async handler API | job protocol、进度、取消、timeout |
| State namespace | 单个 flat map | 组件/插件 namespace、冲突和 ownership |
| Component lifecycle | registry snapshot | mounted update、dispose、hot reload、migration |
| Core/GPUI 耦合 | 同一个 crate | 是否拆成 parser/runtime core 与 GPUI adapter |
| Accessibility | 依赖具体 GPUI 组件 | role、label、keyboard/focus contract |
| Theme | 固定颜色 token | host theme token、dark/light、高对比度 |
| Observability | diagnostics/last patches | tracing、性能计数、Action/reconcile telemetry |

这些开放问题不阻止 standalone v1 独立使用，但会影响未来扩展 ABI。特别是
schema version、capability、async job 和组件 lifecycle，应在第三方协议冻结前解决。

## 23. 实现索引

| 主题 | 实现文件 |
| --- | --- |
| Public API | `crates/declarative_ui_demo/src/lib.rs` |
| VNode | `crates/declarative_ui_demo/src/vnode.rs` |
| HTML 自闭合规范化 | `crates/declarative_ui_demo/src/html_source.rs` |
| HTML parser | `crates/declarative_ui_demo/src/parser.rs` |
| Limits | `crates/declarative_ui_demo/src/limits.rs` |
| Template compile/validation | `crates/declarative_ui_demo/src/template.rs` |
| Diagnostics | `crates/declarative_ui_demo/src/diagnostic.rs` |
| Binding | `crates/declarative_ui_demo/src/binding.rs` |
| Runtime/Action/State | `crates/declarative_ui_demo/src/runtime/` |
| VNode diff/apply | `crates/declarative_ui_demo/src/diff.rs` |
| Component Registry | `crates/declarative_ui_demo/src/component.rs` |
| 默认组件 | `crates/declarative_ui_demo/src/builtin_components.rs` |
| Render services/boundary | `crates/declarative_ui_demo/src/render_context.rs` |
| View/reconcile | `crates/declarative_ui_demo/src/renderer.rs` |
| Stateful input spec | `crates/declarative_ui_demo/src/stateful_nodes.rs` |
| Input cache | `crates/declarative_ui_demo/src/input_cache.rs` |
| Tailwind parser | `crates/declarative_ui_demo/src/tailwind.rs` |
| GPUI style mapping | `crates/declarative_ui_demo/src/tailwind_style.rs` |
| Standalone demo | `crates/declarative_ui_demo/src/main.rs` |
| 使用说明 | `crates/declarative_ui_demo/README.md` |

## 24. Standalone v1 验收清单

### 24.1 代码与自动化合同已实现

- [x] workspace 中存在独立 `declarative-ui-demo` crate；
- [x] `html5ever` + `markup5ever_rcdom` HTML fragment parser；
- [x] custom element XML-style 自闭合兼容；
- [x] 可序列化、与输入语言无关的 VNode；
- [x] strict/permissive 编译；
- [x] source/node/depth/attribute/class token 硬限制；
- [x] forbidden element/attribute hard rejection；
- [x] typed diagnostics、精确去重和 phase replacement；
- [x] 默认 Component Registry 和自定义 Rust 组件；
- [x] component schema；
- [x] renderer error/panic fallback；
- [x] Tailwind utility allowlist；
- [x] GPUI `Entity<Runtime>` 状态；
- [x] structured Action payload；
- [x] Action transaction、error/panic rollback 和有序事件；
- [x] 文本 binding；
- [x] `input`/`textarea` 双向 binding；
- [x] `key → id → path` stateful identity；
- [x] InputState Entity/subscription cache lifecycle；
- [x] VNode diff 和事务式 patch apply；
- [x] Runtime event 自动 reconcile；
- [x] standalone GPUI demo；
- [x] parser/compiler/runtime/binding/component/input/Tailwind/diff contract tests；
- [x] crate README 和本正式架构文档。

### 24.2 尚未作为 v1 完成项

以下项目不影响当前 standalone 机制的代码合同，但其中“完整跨平台手工验收”仍应
作为面向用户发布前的质量门禁：

- [ ] Navop extension catalog/manifest 接入；
- [ ] versioned extension/WASM/process ABI；
- [ ] capability sandbox 和资源隔离；
- [ ] async Action job、timeout 和 cancellation；
- [ ] versioned VNode/State wire schema；
- [ ] reliable source spans；
- [ ] 通用 keyed reorder/move diff；
- [ ] image/asset URI policy；
- [ ] component hot reload/migration lifecycle；
- [ ] 完整跨平台手工视觉、输入法、键盘和可访问性验收。

## 结论

standalone v1 已经不是“把 HTML 字符串直接翻译成若干 GPUI 调用”的一次性 Demo，
而是一个边界明确、可测试的声明式 UI 内核：

```text
受限输入
  → 有界编译
  → typed diagnostics
  → transactional runtime
  → stateful identity
  → VNode reconciliation
  → native GPUI rendering
```

下一阶段应优先继续稳定 standalone API、性能数据和手工 UI 验收，而不是立即把
进程内 Rust trait/Entity 暴露为插件 ABI。待版本化协议、capability、async job 和
隔离模型明确后，再把该 crate 作为 Navop extension UI host 的实现基础接入。
