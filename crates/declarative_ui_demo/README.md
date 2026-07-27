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
- source、节点、深度、属性和 class token 的硬资源限制；
- 分阶段、分严重级别、带 VNode path 的 typed diagnostics；
- 默认 HTML 标签和可扩展的 `ComponentRegistry`；
- 自定义组件的 `Result` 错误边界和 panic boundary；
- 基于 `Entity<Runtime>` 的响应式状态、Action 和事件；
- Action transaction、失败回滚和 panic 回滚；
- 文本绑定以及 `input` / `textarea` 双向绑定；
- 基于 `key`、`id` 或 VNode path 的有状态输入 identity；
- old/new resolved VNode diff 和事务式 patch；
- 真实 GPUI Demo，以及 parser、compiler、runtime、binding、component boundary、
  limits、Tailwind、input 和 diff 契约测试。

## 运行与验证

在仓库根目录运行：

```bash
cargo run -p declarative-ui-demo
```

Demo 展示：

- strict 模式编译受限 HTML；
- HTML 容器、文本、输入框和按钮；
- `username` 的双向输入绑定；
- `status` 与 `save_count` 的响应式文本绑定；
- `action="save"` 到 Rust handler 的结构化派发；
- `data-record="profile"` 形成的 Action payload；
- 通过 registry 注册的自定义 `<sql-editor />`；
- Action 一次提交多个 state 变化后自动 reconcile。

定向质量门禁：

```bash
cargo fmt --all -- --check
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

`compile_template` 是正式入口。它先执行受限 HTML parse，再验证 tag、attribute、
identity 和 Tailwind utility，成功后返回不可变的 `CompiledTemplate`。

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

### 双向输入绑定

```html
<input id="username" bind="username" placeholder="用户名" />
<textarea key="notes" bind="notes"></textarea>
```

- state → input：reconcile 后同步到已有 `InputState`；
- input → state：`InputEvent::Change` defer 写回 `Entity<Runtime>`；
- 程序调用 `InputState::set_value` 不重新产生 Change，因此不会形成 binding loop；
- `bind` 与声明式 `value` 同时出现是 compile error。

每个输入缓存一个 `Entity<InputState>` 和对应 subscription。节点移除后，cache entry
和 subscription 一起释放；旧 Entity 即使仍被其他 Rust 代码持有，也不会继续写回
Runtime。

### Stable identity

有状态组件 identity 的优先级：

1. `key`；
2. `id`；
3. VNode path。

显式 identity value 在一个 `CompiledTemplate` 中共享同一唯一命名空间，即
`id="field"` 与 `key="field"` 也会被判定为重复。使用显式 identity 后，输入节点
在同一父级内 reorder 时仍复用原来的 `InputState`。没有 `id` / `key` 的输入使用
path；结构插入或移动可能让它重建。

当 multiline、placeholder 或 bind 配置改变时，输入 Entity 会重建，旧 Entity 的
write-back subscription 同时释放；仅 bound value 改变时会复用 Entity 并同步其值。

## Component Registry

### 默认组件

| 标签 | GPUI 映射 | 主要属性 |
| --- | --- | --- |
| `div` | `gpui::div()` 容器 | `bind` |
| `span` | div-like 文本容器 | `bind` |
| `button` | `gpui_component::button::Button` | `action`、`data-*` |
| `input` | 单行 `Entity<InputState>` | `bind`、`value`、`placeholder` |
| `textarea` | 多行 `Entity<InputState>` | `bind`、`value`、`placeholder` |
| `img` | `gpui::img(src)` | 必填 `src` |

所有组件都支持全局 `id` 和 `key`。`span` 不模拟浏览器 inline layout，只是一个
GPUI div-like 文本容器。

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
  `overflow-hidden`；
- 字体：`text-sm`、`text-base`、`text-lg`、`text-xl`、
  `font-semibold`；
- 颜色：有限的 Zinc、Blue、Emerald、White token，可用于
  `bg-*`、`text-*`、`border-*`。

间距使用 v1 固定映射 `N × 4px`。不支持负数、小数、任意值、`NaN`、`inf` 或
大于 96 的 scale。Modifier 严格保留 class source order；多个 setter 冲突时，后
应用的 modifier 按 GPUI builder 语义生效。

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
5. 清理已经移除的有状态输入 cache；
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

- `img src` 尚未接入网络/文件 capability；
- Runtime state 没有插件级内存配额；
- 自定义 Rust component 与 Action handler 都是宿主进程内可信代码。

在接入不可信扩展前，必须另行定义 capability、资源协议、WASM ABI、版本协商和
宿主隔离。

## 模块划分

| 模块 | 职责 |
| --- | --- |
| `html_source.rs` | 安全展开自定义 XML-style 自闭合标签 |
| `parser.rs` | html5ever fragment → 受控 VNode |
| `limits.rs` | 编译资源限制和资源类型 |
| `vnode.rs` | 可 serde 的输入无关 VNode 中间表示 |
| `template.rs` | strict/permissive 编译、schema/class/identity 校验 |
| `diagnostic.rs` | typed、分 phase、去重的 diagnostics |
| `binding.rs` | state → resolved VNode 和 missing-binding 诊断 |
| `runtime/` | StateStore、Action、transaction 和 RuntimeEvent |
| `diff.rs` | VNode diff 与事务式 patch |
| `tailwind.rs` | utility token → 语义 modifier |
| `tailwind_style.rs` | modifier → GPUI `Styled` builder |
| `component.rs` | Registry、Schema、Props、Renderer trait |
| `builtin_components.rs` | 默认标签到 GPUI component 的映射 |
| `stateful_nodes.rs` | 有状态输入 spec 与 live identity 收集 |
| `input_cache.rs` | InputState cache、双向 binding subscription |
| `render_context.rs` | 递归组件渲染、style、action/input 服务 |
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
- 完整 keyed reconciliation；
- Navop extension catalog、插件热重载或 manifest 接入；
- WASM component/action/state ABI；
- 插件 capability sandbox；
- 网络和本地资源授权；
- 任意 typed component lifecycle ABI。

这些限制是产品边界，不是尚未声明却“碰巧不能工作”的行为。接入 Navop 扩展系统前，
应先以本 standalone crate 的 compile/runtime/component contract 为基础设计单独的
版本化插件协议。
