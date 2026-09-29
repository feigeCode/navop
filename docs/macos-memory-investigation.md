# Navop macOS 内存占用排查记录

## 1. 问题概述

Navop 0.15.2 在 macOS 上运行一段时间后，Activity Monitor 显示内存约 1.8 GB。初始实测 footprint 为约 1.87 GB，峰值约 2.15 GB。

后续执行 `heap --forkCorpse` 等深度诊断后，目标进程 footprint 被采样操作扰动到约 2.07 GB，主要增加在 malloc allocator 的保留区。因此后续优化应以应用冷启动和固定操作流程重新建立基线，不能直接把 2.07 GB 当作自然使用状态。

本次排查结论：

- 独立弹窗的关闭 API 没有用错，`window.remove_window()` 是正确用法。
- App 层会在窗口更新流程中移除已标记关闭的窗口。
- 主要占用来自 GPUI/Metal 渲染资源，而不是传统 Rust 对象泄漏。
- 当前最明确的问题是全局 `InstanceBufferPool` 无上限缓存 Metal buffer。
- 进程同时保留了 8 个 `GPUIView`、8 个 `CAMetalLayer`，但只有 1 个窗口处于 onscreen 状态，需要继续验证这些 renderer 是否对应仍存活的独立弹窗。

## 2. 现场采样

目标进程：

```text
PID:       75089
Executable: /Applications/Navop.app/Contents/MacOS/navop
Version:   0.15.2
Platform:  macOS ARM64
Launch:    2026-09-01 13:18:30
```

### 2.1 footprint 分布

初始、较有代表性的 footprint 约 1.87 GB：

| 分类 | 大小 | 说明 |
| --- | ---: | --- |
| `IOAccelerator (graphics)` | 1007 MB | Metal/GPU 资源，最大项 |
| `MALLOC_LARGE` | 477 MB | 大块普通分配及 allocator 保留区 |
| `IOSurface` | 237 MB | CAMetalLayer drawable |
| `MALLOC_SMALL` | 110 MB | 普通小对象和分配碎片 |
| `owned unmapped (graphics)` | 24 MB | GPU 相关未映射物理资源 |
| 线程栈实际驻留 | 约 1 MB | 不是主要来源 |

执行深度 heap/corpse 分析后，`MALLOC_LARGE` 一度上升到约 659 MB，footprint 稳定在约 2.067 GB。GPU 和 IOSurface 分类基本不变，说明这部分额外增长主要是诊断造成的 allocator 扰动，不应作为应用自身泄漏证据。

5 秒连续采样没有看到持续线性增长。

### 2.2 Metal 窗口和 drawable

`heap`/`vmmap` 观察到：

- `GPUIView`: 8 个。
- `CAMetalLayer`: 8 个。
- `CAMetalLayer Display Drawable`: 24 个。
- GPUI 每个 layer 设置 `maximum_drawable_count = 3`，所以 8 个 layer 正好对应 24 个 drawable。
- CoreGraphics 窗口列表中只有 1 个 onscreen 窗口，说明其余 renderer 可能是隐藏窗口、非 onscreen 窗口，或者仍未完成释放。

drawable 示例：

- 3 个约 12.1 MB 的 `2200x1400` surface。
- 3 个约 18.4 MB 的 `2742x1718` surface。
- 多个约 8.0 MB 的 `1400x1440` surface。

这些 surface 合计约 237 MB，与 `IOSurface` footprint 分类一致。

### 2.3 16 MB buffer 证据

`vmmap`/`heap` 发现：

- 28 个精确的 16 MB raw allocation，约 448 MB。
- 15 个 `AGXG16GFamilyBuffer` 和 13 个 `AGXBuffer`，合计 28 个 Metal buffer 对象。
- 代码中的 GPUI `InstanceBufferPool` 初始 buffer size 为 2 MB，渲染失败时按 2 倍增长，可能增长到 16 MB。

由于目标进程是 hardened/ad-hoc bundle，系统工具无法读取完整的 malloc allocation backtrace，因此“28 个 16 MB 分配就是 28 个 instance buffer”的结论属于强关联证据，不是符号级 100% 证明。但数量、大小和对象类型完全吻合，应优先按此方向修复和验证。

### 2.4 传统泄漏检查

`leaks` 结果约为：

```text
356 nodes
17.76 KB leaked
```

这不支持“数百 MB 已不可达泄漏”的判断。当前问题更像是资源仍然可达，但被全局缓存或仍存活的 renderer 持有。

## 3. 关闭流程是否正确

### 3.1 App 层

GPUI 的 `Window::remove_window()` 只是把当前窗口标记为 removed：

```text
gpui-ce/crates/gpui/src/window.rs:2111-2114
```

真正移除发生在 `App::update_window_id` 的收尾逻辑：

```text
gpui-ce/crates/gpui/src/app.rs:1881-1924
```

当 `window.removed` 为 true 时，会移除：

- `cx.window_handles`
- `cx.windows`
- 窗口关联的 entity invalidator
- 已关闭窗口观察者

因此业务层调用 `window.remove_window()` 的姿势是正确的。

### 3.2 独立弹窗

独立弹窗统一通过：

```text
navop/crates/core/src/popup_window.rs:148-256
```

创建为普通 GPUI window，并注册窗口关闭处理。

复用弹窗（`open_reusable_popup_window`，登记了复用键）的关闭路径**不再是** `remove_window()`：

- **原生窗口**：`orderOut` 隐藏后登记复用。原因是在 macOS 上销毁原生窗口会触发 AppKit
  Touch Bar 观察者向已 dealloc 的对象注销，抛出的 ObjC 异常无人接住 ⇒ 闪退（issue #262）。
- **业务会话**：关闭时立即结束 —— 卸载业务 view 及它持有的数据、连接与任务句柄，清掉焦点与通知。

```text
navop/crates/core/src/window_close.rs      # close_window_for_reuse(window, cx)
navop/crates/core/src/popup_window.rs      # end_reusable_popup_session / PopupWindowContent::end_session
navop/crates/core/src/popup_lifecycle.rs   # live_windows / live_sessions 计数与日志
```

关键约束：**复用窗口不等于复用业务状态**。只隐藏不卸载，业务 view 会被仍然存活的内容树一直强引用着 ——
用户不再打开那类窗口时就是纯泄漏（注册表里只存 `WeakEntity`，管不到它）。
没有登记复用键的窗口仍然走 `remove_window()`；不要用 `minimize_window()` 代替关闭。

### 3.3 macOS 平台层

macOS `MacWindow` 被 Rust 层释放时：

```text
gpui-ce/crates/gpui_macos/src/window.rs:1181-1203
```

当前流程会调用 `renderer.destroy()`，随后异步执行 native window close 和 autorelease。

问题是：

- `MetalRenderer::destroy()` 当前是空实现：
  `gpui-ce/crates/gpui_macos/src/metal_renderer.rs:582-584`
- 全局共享的 `InstanceBufferPool` 不属于单个 renderer，renderer 关闭后仍会保留 buffer。
- `setReleasedWhenClosed:NO` 要求后续 native window 生命周期处理必须可靠完成；这条链路需要通过关闭后 layer 数量下降来验证。

## 4. 代码原因

### 4.1 全局共享的 InstanceBufferPool 无上限

macOS 平台状态创建一个共享 renderer context：

```text
gpui-ce/crates/gpui_macos/src/platform.rs:171-227
gpui-ce/crates/gpui_macos/src/platform.rs:654-682
```

每个 MacWindow 都 clone 同一个 `renderer_context`，类型为：

```rust
Arc<Mutex<InstanceBufferPool>>
```

池的行为：

```text
gpui-ce/crates/gpui_macos/src/metal_renderer.rs:74-127
```

- 默认大小 2 MB。
- 不够用时创建当前大小的 Metal buffer。
- 渲染完成后放回 `buffers`。
- `buffers` 没有数量上限或总字节上限。
- 窗口关闭不会清理这个全局池。

因此只要历史上有多个窗口同时渲染，或者有多个较复杂 scene 同时提交，池就可能保留大量 16 MB buffer。当前约 28 个 16 MB allocation 与此行为高度吻合。

### 4.2 每个 renderer 预分配多张离屏纹理

窗口尺寸变化时：

```text
gpui-ce/crates/gpui_macos/src/metal_renderer.rs:496-573
```

会创建：

- path intermediate texture
- scene color texture
- 两张 group texture
- 两张 half-resolution blur texture
- 额外的 MSAA texture

当前实现即使 scene 没有 blur/filter，也会创建 scene/group/blur 相关纹理。这会放大多窗口场景下的 GPU 占用。它是明确的优化点，但目前没有证据证明它单独造成了关闭后的长期泄漏。

### 4.3 8 个 renderer 的生命周期需要确认

采样显示 8 个 GPUIView/CAMetalLayer，而系统只有 1 个 onscreen 窗口。可能原因：

- 仍有多个独立弹窗实际存活但不可见。
- native window 已关闭，但 Objective-C retain/autorelease 尚未完成。
- renderer 尚存于 GPUI/平台层引用中。
- 某些窗口在使用过程中被隐藏，而不是走 remove 流程。复用弹窗是**有意如此**（见 3.2）：
  它属于「受控的固定保留」，上限是复用键数量；用 `popup_lifecycle` 的 `live_windows` / `live_sessions`
  计数把它跟「持续增长」区分开。

需要加入窗口创建、关闭、`MacWindow::drop` 和 renderer 数量日志，才能把这 8 个 renderer 映射到具体窗口。

## 5. 推荐修改方案

建议按风险从低到高分阶段修改。

### 阶段一：限制 InstanceBufferPool 缓存

目标：先把约 448 MB 的 16 MB buffer 缓存压到可控范围。

在 `metal_renderer.rs` 增加具名上限，例如：

```rust
const MAX_CACHED_INSTANCE_BUFFERS: usize = 8;
const MAX_CACHED_INSTANCE_BUFFER_BYTES: usize = 128 * 1024 * 1024;
```

调整 `InstanceBufferPool::release`：

- size 不匹配时直接丢弃。
- 已缓存数量达到上限时直接丢弃。
- 已缓存字节数达到上限时直接丢弃。
- 只缓存已完成 command buffer 的 buffer。

参考实现：

```rust
const MAX_CACHED_INSTANCE_BUFFERS: usize = 8;
const MAX_CACHED_INSTANCE_BUFFER_BYTES: usize = 128 * 1024 * 1024;

pub(crate) fn release(&mut self, buffer: InstanceBuffer) {
    if buffer.size != self.buffer_size {
        return;
    }

    let cached_bytes = self.buffers.len().saturating_mul(buffer.size);
    let can_cache = self.buffers.len() < MAX_CACHED_INSTANCE_BUFFERS
        && cached_bytes.saturating_add(buffer.size) <= MAX_CACHED_INSTANCE_BUFFER_BYTES;

    if can_cache {
        self.buffers.push(buffer.metal_buffer);
    }
}
```

建议初始上限为 8 个或 128 MB，不建议一开始设置为 3 个，因为多窗口并发渲染可能造成频繁申请和释放。后续依据帧率和内存数据再调小。

### 阶段二：让 renderer destroy 清理高占用资源

将：

```rust
pub fn destroy(&self) {}
```

改为可变清理方法，并在 `MacWindow::drop` 中调用：

- `path_intermediate_texture = None`
- `path_intermediate_msaa_texture = None`
- `scene_color_texture = None`
- `blur_ping_texture = None`
- `blur_pong_texture = None`
- `group_textures.clear()`
- 必要时释放或断开 `CAMetalLayer`

参考结构：

```rust
pub fn destroy(&mut self) {
    self.path_intermediate_texture = None;
    self.path_intermediate_msaa_texture = None;
    self.scene_color_texture = None;
    self.blur_ping_texture = None;
    self.blur_pong_texture = None;
    self.group_textures.clear();
}
```

该方法应设计为幂等。不要在 command buffer 仍可能使用资源时绕过 Metal 引用计数；只清理 Rust 持有的引用，实际资源由 Metal 在 GPU 完成后回收。

注意：这一阶段不能清理全局 `InstanceBufferPool`，因为它被所有窗口共享。buffer pool 必须通过阶段一的容量限制，或者单独增加显式 trim API。

### 阶段二补充：验证 native window 是否及时析构

先用日志确认以下顺序是否完整发生：

1. 业务层调用 `window.remove_window()`。
2. `App::update_window_id` 移除窗口。
3. `MacWindow::drop` 执行。
4. `dealloc_view` 和 `dealloc_window` 最终执行。

只有前三步发生、第四步长期不发生时，再调整 Objective-C 释放策略。可选方向：

- 在异步 native close 闭包中创建局部 `NSAutoreleasePool` 并在 close 后 drain。
- 在关闭前停止 display link。
- 将 native view 从 superview 移除，并断开它持有的 CAMetalLayer。
- 检查 `setReleasedWhenClosed:NO` 对应的 retain 是否被可靠平衡。

示意代码：

```rust
this.foreground_executor
    .spawn(async move {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);

            if let Some(parent) = sheet_parent {
                let _: () = msg_send![parent, endSheet: window];
            }

            window.close();
            window.autorelease();
            pool.drain();
        }
    })
    .detach();
```

这部分改动的生命周期风险高于 buffer pool 限制，必须在确认 native dealloc 没有发生后再做，不能仅凭 `renderer.destroy()` 为空就直接替换为手动 `release`。

### 阶段三：离屏纹理惰性创建

根据 scene 是否包含 blur/filter 决定是否创建：

- scene color texture
- group textures
- blur ping/pong textures

没有 filter 时不创建这些纹理；从有 filter 切换到无 filter 时可以清理或延迟清理。path rasterization 所需纹理要根据实际调用路径单独保留，不能直接全部删除。

### 阶段四：加入资源生命周期诊断

建议增加低频日志或 debug 计数：

- renderer created/dropped 数量
- `MacWindow` created/dropped 数量
- `InstanceBufferPool` 当前 buffer 数量和字节数
- 每个 buffer 的 size
- `CAMetalLayer` 创建和销毁数量
- 弹窗关闭后的窗口 ID

不要在每一帧打印日志，避免日志本身影响性能和内存。

## 6. 构建注意事项

Navop 当前使用 GPUI CE 的 git 依赖：

```text
navop/Cargo.toml:61-64
rev = 9086e0b273bddc083fb030a8aadfc27767eda88e
```

实际编译使用的是 Cargo git checkout 中的对应 revision，不是自动使用旁边的 `gpui-ce` 工作区目录。修改 GPUI 后需要：

1. 临时改成 path dependency，或
2. 提交 GPUI 修改并更新 Navop 的 git revision。

不要直接修改 `.cargo/git/checkouts` 下的临时源码作为最终方案。

## 7. 验证标准

### 7.1 关闭流程

重复执行以下操作：

1. 启动 Navop。
2. 连续打开多个独立弹窗，例如 SSH、数据库或新建连接窗口。
3. 分别通过取消、标题栏关闭按钮和系统关闭按钮关闭。
4. 等待 2-5 秒。
5. 再执行内存采样。

预期：

- GPUIView/CAMetalLayer 从 8 回落到接近 1。
- drawable 从 24 回落到接近 3。
- 关闭弹窗后 `InstanceBufferPool` 不超过设定上限。
- footprint 明显下降，而不是只在重启后下降。

### 7.2 建议命令

```bash
footprint --pid <PID>
vmmap -summary <PID>
heap -sH <PID>
leaks --noContent <PID>
```

重点比较：

- `IOAccelerator (graphics)`
- `IOSurface`
- `MALLOC_LARGE`
- `GPUIView` 数量
- `CAMetalLayer` 数量
- 16 MB allocation 数量

### 7.3 回归风险

- buffer 上限过小可能造成 Metal buffer 频繁申请，表现为 CPU 占用上升或帧率下降。
- renderer 清理需要兼容 command buffer 异步完成。
- blur 惰性创建可能影响首帧，需要验证窗口 resize、透明标题栏和 blur UI。
- 必须同时验证主窗口、独立弹窗、最小化窗口和多窗口并发渲染。

## 8. 最终判断

用户关闭独立弹窗的操作不是根因。当前优先级如下：

1. 先限制全局 `InstanceBufferPool`，这是最明确且收益最大的修改。
2. 再验证关闭后 8 个 renderer 是否下降到 1 个。
3. 若 renderer 数量不下降，继续修复 macOS native window/renderer 生命周期。
4. 最后将 blur/filter 离屏纹理改为惰性创建，降低正常多窗口场景的 GPU 基线。

## 9. 复用弹窗的现状与判据

复用弹窗（登记了复用键的那些）在关闭时**不销毁原生窗口**，而是 `orderOut` 隐藏并结束业务会话（见 3.2）。
这不是「泄漏」，但必须有判据，否则无法把它和真正的增长区分开。`crates/core/src/popup_lifecycle.rs` 提供：

| 计数 | 含义 | 正常表现 |
|---|---|---|
| `live_windows` | 登记在册（隐藏后等待复用）的原生窗口 | 每类弹窗首次打开 +1，之后开关不再增长；上限＝复用键数量 |
| `live_sessions` | 当前仍持有业务 view 的弹窗 | 关闭后回落到 0；随开关次数持续上涨＝旧会话没卸载 |
| `opened_windows` | 累计创建的原生窗口 | 只在复用没命中（退化成每次新建）时随开关次数上涨 |
| `opened_sessions` | 累计打开的业务会话 | 每次打开 +1，属预期 |

日志 target 为 `one_core::popup_lifecycle`（stage 取 `popup_window_registered` / `popup_window_unregistered` /
`popup_session_ended` / `popup_session_reopened`），只记数字，不记标题、路径或业务内容。

采样时的判断顺序：先看 `opened_windows` 是否随开关次数上涨（是 ⇒ 复用没命中，问题不在保留策略）；
再看 `live_sessions` 是否回落（否 ⇒ 会话没卸载）；两者都正常但 footprint 仍涨，才回到第 4、5 节找 GPU/renderer 侧原因。

注意：不要给复用注册表加 LRU 淘汰来「限制保留」——淘汰即销毁，等于把 Touch Bar 崩溃挪到淘汰路径上。
注册表按 `&'static str` 复用键组织，结构上已有上限。

## 10. 原生窗口退役方案：试过、被否，以及原因（2026-09-26）

背景：`fork-0.3.104` 起关窗会真正释放原生窗口，AppKit 的 Touch Bar 观察者可能在被观察视图
析构之后才注销观察 → 未捕获 ObjC 异常 → 进程终止（上游单据 zed-industries/zed#64819）。
「等一段时间再释放」已经被现场否证（v0.18.6 = `fork-0.3.110`、v0.19.1 = `fork-0.3.114`
都带 100 ms 等待，帧序列一致，仍然崩），于是试了另一个方向：
**关闭后不释放原生窗口，只把重资源摘出来**（`MacWindow::drop` 里把 `MacWindowState`
从 `windowState` ivar 上摘掉，原生窗口交给一张进程级退役表 `window_teardown::retire`）。

这份实现在 review 中被否，理由不是风格问题，而是明确的正确性/边界问题：

1. **阻断：先摘状态、再 `close()`，会走空指针 `Arc`。**
   GPUI 在 `GPUIWindow` 上注册了自己的 `close`（`window.rs:487` →
   `close_window`，`window.rs:3255`），它第一步就是 `get_window_state(this)`；
   而 `get_window_state`（`window.rs:2468`）不判空就直接 `Arc::from_raw`。
   所以「`take_window_state(window)` → 之后 `window.close()`」这条链**必然**用空指针构造
   `Arc`（UB），与 Touch Bar 无关，是每次关窗都走——而且不能简单把 `close()` 提前了事：
   `MacWindow::drop` 持有状态锁，`close_window` 会再次锁同一个状态。

2. **退役后的原生对象仍会被发消息。**
   `reset_cursor_rects`（`window.rs:2552`）、`make_backing_layer`（`3279`）、
   `view_did_change_backing_properties`（`3285`）、`set_frame_size`（`3290`）等注册在
   窗口/视图类上的方法都无条件 `get_window_state`。把 delegate 置空解决不了这个，
   需要显式区分 Active / Closing / Retired，并让每个原生入口在退役后安全返回
   （默认值 / 调用 superclass / 忽略事件）。仅 `window.rs` 内 `get_window_state`
   就有 42 处调用点，这才是这件事的真实工作量；在一处补 `if raw.is_null()` 不够。

3. **退役表是无界累积，不是复用池。**
   每真正销毁一个 GPUI 窗口就多留一个原生窗口常驻；`Vec<usize>` 只记录地址，
   本身不做 Objective-C retain，将来清空列表也不会释放窗口。它可以定义为
   「有意的泄漏止血」，但不能当作内存增长问题的解决方案。

4. **「只留空窗口、GPU 已释放」的说法不成立。**
   断开的是 `原生对象 → MacWindowState`；未处理
   `NSWindow → contentView → GPUIView → backing CAMetalLayer`
   （`native_view.setWantsLayer(YES)`，`makeBackingLayer` 返回 renderer 的 layer）。
   而 `gpui_apple::metal_renderer::destroy()` 是空实现（`metal_renderer.rs:444`），
   所以「释放了 Rust renderer 的引用」不等于「原生 layer 被回收」，这部分必须实测，
   不能靠注释断言。

结论：**原生销毁作为独立问题继续修，先不动底层。** 主线仍是
「有界复用（隐藏不销毁）＋ 关闭即结束业务会话」，它已在
`crates/core/src/window_close.rs` / `crates/core/src/popup_window.rs` 落地且有现场数据支撑。
若以后重启退役方案，前置条件是上面 1–3 全部补齐（其中 2 需要一次状态机式的生命周期改造），
而不是扩大 `retire()` 的使用范围。

两条容易误判的边界：

- Navop 目前的隐藏路径（`window_close.rs` 的 `orderOut:`）**不会**进入 `MacWindow::drop`，
  所以即使底层退役方案成立，也不会自动解决隐藏窗口自身的资源保留。
- 根 `Cargo.toml` 的 `[patch.crates-io]` 曾指向 `fork-0.3.115`；本地 zed checkout 编译通过
  **不等于** navop 用上了这份改动，集成必须走 gpui-pre 快照 / 新 tag
  （2026-09-27 已切到 `fork-0.3.116`，见 §10.3）。

实验版代码先落在 zed checkout 的本地分支 `experiment/native-window-retire`
（`440483b8ee` 保留作对照，修好的版本见 §10.1 / §10.2 的两个提交），
现已进入发布分支 `publish/gpui-pre-0.3.116`（见 §10.3）。

### 10.1 重做后的状态（2026-09-26 晚）

review 的 4 条全部按上面 1–3 补齐，重做提交 `827b2105a3`
（`gpui_macos: Retire a native window without detaching its state or holding its GPU resources`，
与 `440483b8ee` 相邻，仍在本地 `experiment/native-window-retire` 分支）：

- **不再分离状态。** `windowState` ivar 全程有效，`renderer` 改为 `Option`，加入 `retired` 标记；
  资源改在**关闭之后**于原地释放（`MacWindowState::retire`），因此 `close_window`
  仍能看到完整状态（空指针 `Arc` 路径消失）。
- **入口安全化。** `makeBackingLayer`（回退 super）、`viewDidChangeBackingProperties`、
  `setFrameSize:`（调 super、不调 drawable 尺寸）、`displayLayer:`、`resetCursorRects`、
  `viewDidChangeEffectiveAppearance` 都按语义安全返回；输入/拖拽/标签页/delegate 入口
  在退役后不可达（离屏、非 key、非 first responder、delegate 已置 nil）。
- **释放被证明。** `retire()` 丢弃 renderer（连带 metal layer、drawable pool、纹理）、
  accessibility adapter 与**全部回调**（回调会扣住 GPUI 实体，不清就等于把上面那份
  「业务会话不卸载」的问题挪到底层）；`window_teardown::release_layer` 用
  `setWantsLayer:NO` 断开视图对 metal layer 的持有，并在 layer 仍在时告警。
- **边界明确为「按使用量有界的保留」。** 每次退役记一条日志，累计达到
  `RETIRED_WINDOWS_WARN_THRESHOLD`（32）时告警一次；不设上限、不做淘汰，因为
  淘汰即释放（会崩）。若导航器发现在长会话里窗口翻页量很大，再考虑原生窗口池。

验证状态：zed 侧 `cargo check / clippy / fmt` 全绿；**本机没有 Touch Bar，
崩溃本身既未能复现也未能证明修好**。字段判据是 `window_teardown` 的退役计数日志
与「a retired window's view still holds a layer」告警。

### 10.2 可跑的验证与 CI（2026-09-26 深夜）

用测试把「退役后原生对象存活、renderer 的 metal layer 已释放」变成可核验的判据，
提交 `8af22191af`（`gpui_macos: Probe what retiring a native window guarantees`，
仍在本地 `experiment/native-window-retire`）：

- 探针必须跑在**进程主线程**上：AppKit 建窗/显示窗口时抛的 Objective-C 异常无法被 Rust
  捕获，libtest 又在自己的测试线程上跑用例（实测直接 SIGABRT）。做法是测试重新执行本测试
  二进制、带上 `GPUI_MACOS_TEARDOWN_PROBE`，由 `#[ctor]` 在 libtest 之前于主线程完成探针并
  退出；测试只读子进程退出码 —— 这样 `abort` 也会表现为失败，而不是挂住不返回。
- 探针实际抓到一个真问题：`setWantsLayer: NO` **不会**立刻释放视图的 layer，AppKit 要等到
  该视图的下一次 display cycle，而退役窗口离屏、永远不会再有 —— 于是 layer（连同它的
  drawable pool 与纹理）继续被扣着。`release_layer` 现在在置 `setWantsLayer: NO` 之后
  再把 layer 摘掉，探针断言的正是这个最终状态。
- 跑法：`cargo test -p gpui_macos --lib window_teardown`（本机 arm64 通过；
  同 crate 全部 8 个测试通过，`clippy --all-targets` 与 `fmt --check` 全绿）。

CI：`.github/workflows/macos-window-teardown.yml`（fork 专用，不要提到上游 PR），
在 `macos-15-intel`（x86_64）与 `macos-latest`（arm64）上跑同一个探针。

**明确说明：这个 workflow 复现不了崩溃本身。** 变量不是架构而是 Touch Bar ——
`_NSTouchBarFinder` 只有插着 Touch Bar 时才装了观察者，GitHub 托管 runner（Intel 与
arm 都一样）没有 Touch Bar。因此 CI 能钉住的是「修复所依赖的不变量」，真正的 abort
回归只能在带 Touch Bar 的机器上做（自托管 runner 或人工真机）。

### 10.3 发布与集成（2026-09-27）

退役方案不再是「只在本地的实验」，已经发成 gpui-pre 快照并被 navop 用上：

- **发布基线必须是 `gpui-pre-release`，不是退役提交所在的（纯净）upstream 基线。**
  navop 的 `remote_desktop_view` 用了 fork-only API（`DynamicTexture` /
  `Window::update_dynamic_texture`），它只在 `gpui-pre-release` 上；
  当前 `upstream/main` 里 0 处出现。两个基线在 `crates/gpui_macos` 上逐字节相同，
  所以换基线只是换基座，cherry-pick 无冲突。
- **发布分支 `publish/gpui-pre-0.3.116`**（zed checkout）：`gpui-pre-release`
  → revert 掉旧 100 ms 宽限期（`0c2f3ae37f`，已被退役取代，现场已否证）
  → cherry-pick 三个退役提交（`440483b8ee` → `827b2105a3` → `8af22191af`）。
  分支上 `cargo test -p gpui_macos --lib` = 8 passed / 0 failed（含退役探针）、
  `cargo fmt -p gpui_macos -- --check` 通过；`cargo clippy` 在
  `crates/gpui/src/window.rs:5154` 报基线自带的 `redundant_clone`（upstream 在新版里已修，
  本次未触碰该 crate），只影响在快照上跑 clippy，不影响 navop 构建。
- **快照 `fork-0.3.116`** 已发到 `feigeCode/gpui-pre`（提交 `3f3fd66`）。相对
  `fork-0.3.115` 的 delta 只有版本号、`crates/gpui_macos/src/window.rs`（229 行）、
  新增 `window_teardown.rs`（348 行）与 `gpui_macos.rs` 里的 `mod` 声明 ——
  快照里的两个文件与 zed 源**逐字节相同**。
- **navop 侧**：`[patch.crates-io]` 的 24 处与 `Cargo.lock` 已切到 `tag = "fork-0.3.116"`
  （由 `script/migrate-to-git-fork.py` 改写），`[workspace.dependencies]` 的
  `gpui-pre = "0.3.99"` 是 caret 范围，无需改动。该 tag 在真机上未通过（见 §10.4），
  现为 `tag = "fork-0.3.117"`。
- 真机判据不变（§10.2）：带 Touch Bar 的机器上反复开关窗口，看进程是否存活，
  以及 `window_teardown` 的退役计数日志、「a retired window's view still holds a layer」告警。

### 10.4 真机结论：退役没治住，触发点不在我们的释放路径（2026-09-27）

- 给 Touch Bar 用户测试的 x86 包（`0.19.2-touchbar-retire`，Mach-O UUID
  `06a0be38-9ab5-34a2-a75b-2d9e88d7c831`，与本地 `Navop.app` 逐字节同一份，
  二进制内确认含 `retired a native window` 与 layer 告警串）在 Intel Touch Bar 机
  （`MacBookPro16,2`，macOS 14.8.9）上，**启动 13 秒后仍然 SIGILL**，栈与修复前完全一致：
  `-[_NSTouchBarFinderObservation invalidate]` → `removeObserver:forKeyPath:context:`
  → `-[NSApplication _crashOnException:]`。原生窗口和视图都没被释放，abort 照样发生 ——
  「保留窗口」这条路在真机上被否证（连同已被现场否证的 100 ms 宽限期，共两次）。
- 其它仓库的报告（既不用 GPUI，也不共享我们的窗口生命周期）指向同一结论：
  这是 AppKit 自己的记账问题，不是我们的对象活没了：
  - `kodezine/RustyCAN#95`（winit/egui）：`_NSTouchBarFinder` 对 responder 链上每个
    `NSResponder` 注册 `nextResponder` KVO；**responder 链快速变化时排队的多个
    invalidate 块会二次移除已移除的观察者**，抛 `NSRangeException`——对象是活的，
    所以异常里才印得出类名（`<WinitView>` / `<ElectronNSWindow>` / `<NSView>`）。
  - `longbridge/gpui-kit#3192`（GPUI，M1）：偶发，只在**切换最前台 App** 时中过
    （25 次内 1 次），并直接引用 navop#268。
  - `dashpay/dash-evo-tool#820`：退出前先 `orderOut:` 所有窗口即可规避（shutdown 变体）。
  - `johnlindquist/kit#1550`（Electron，观察对象是 window）、`emilk/egui#2768`（eframe，退出时）。
- 因此真正的致命步骤是 **AppKit 把它自己抛出的记账异常升级为 abort**；
  `-[NSApplication _crashOnException:]` 的编码是 `v24@0:8@16`，唯一参数就是那个
  `NSException`。这一层是唯一能覆盖所有变体（焦点切换、关窗、退出）的拦截点。
- **实现**（fork 侧）：`crates/gpui_macos/src/touch_bar_guard.rs` + 在
  `MacPlatform::new` 里安装。它接管 `-[NSApplication _crashOnException:]`，
  **只吞**「名字是 `NSRangeException` 且 reason 含 `_NSTouchBarFinderObservation`」
  的那一条：那次注销本身已经发生过，没有东西可清理、也没有状态需要靠 abort 保护；
  其余异常原样转发给原实现，照 AppKit 的意愿 abort。每吞一次打一条 `warn`
  （默认日志级别可见），现场可据此统计命中次数。
- **发布**：`fork-0.3.117`（`.gpui-pre/publish` 提交 `06ac137`）。相对 0.3.116 的 delta
  只有版本号 + `gpui_macos.rs`(+1)、`platform.rs`(+4)、新增 `touch_bar_guard.rs`(266 行)，
  三个文件与 zed 源逐字节相同；`cargo test -p gpui_macos --lib` = 11 passed / 0 failed
  （含 3 个守卫测试：过滤条件、真实 `NSException` 读取、只接管一次），
  `cargo fmt -p gpui_macos -- --check` 通过、`cargo clippy -p gpui_macos --all-targets --no-deps` 干净。
  navop 侧 24 处 patch 与 `Cargo.lock` 已切到 `tag = "fork-0.3.117"`。
- **新的真机判据**：同一台 Touch Bar 机上重放 #308（建 SSH 连接 → 输入 → 确定）、
  反复开关弹窗、⌘-Tab 切走再切回、正常退出，看进程是否存活；存活且在
  `~/.config/navop/logs/navop.log`（旧目录 `~/.config/one-hub/logs/`）里出现
  `ignored an AppKit Touch Bar observer exception` —— 说明原异常确实发生过、守卫拦住了它。
  该文件确实收得到 gpui 的 `log` 输出（`tracing-subscriber` 的 `tracing-log` 默认特性
  已在 `init()` 里装好 `LogTracer`，本机日志里有 36 条 `WARN gpui` 可作证）；
  但 `took over …` 那条 `info` 发生在日志系统起来之前，多半会被丢掉，不能拿它判断装没装。
- **未决**：0.3.116 的退役方案（`window_teardown`）现在既非必需（不是它导致崩溃）
  也无害，是否回退，等守卫真机验证通过后再定；`RETIRED_WINDOWS` 无上限累积
  （§10.1 的 P3）同理。

### 10.5 第一版守卫把崩溃换成了卡死：改成让异常根本不发生（2026-09-27）

- 真机反馈（同一台 Intel Touch Bar 机，包 `0.19.2-touchbar-guard`，sha256
  `530d3b081164310671fd0fd88509b6565461bb467d87d1b7de58805135f84803`）：
  「弹窗按确定后不闪退了，直接卡死 app，点不了任何按钮」。吞掉
  `_crashOnException:` 里的异常确实拦住了 abort，但把 abort 换成了界面冻结。
- 机制：吞点选得太靠外。异常那时已经从 `NSDisplayCycleFlush` 里 unwind 出来，
  直接从 `-[NSApplication _crashOnException:]` 返回，等于让 display cycle 半途收场，
  事件循环不再推进 ⇒ 转圈、按钮无响应。用「靠后的兜底」换「靠前的预防」方向错了。
- 新实现（`crates/gpui_macos/src/touch_bar_guard.rs` 重写，`_crashOnException:` 那条彻底删掉）：
  接管 `removeObserver:forKeyPath:context:` —— 抛出路径上最外层的公开方法 —— 当观察者类名含
  `_NSTouchBarFinder` 时**直接返回**，其余一律转发 Foundation（照旧抛、照旧 abort）。
  异常不再发生 ⇒ 没有 unwind、没有半截 display cycle，既不 abort 也不卡死。
  - 补丁范围按运行时枚举，不写死类名清单。key path 是 `nextResponder`，所以被观察对象必然
    是 `NSResponder` 子类（窗口 / 视图 / 字段编辑器），补丁集 = `NSObject` ∪ 所有
    `NSResponder` 子类里**自己实现了该方法**的类。探针当场证明只补 `NSObject` 不够：
    `NSWindow` 自己实现了 `removeObserver:forKeyPath:context:`，窗口上的注销不经过 `NSObject`。
  - 实现签名必须 `extern "C-unwind"`：转发路径上的异常要穿回调用者（AppKit 的 display cycle
    自己会 catch）；`extern "C"` 会让 Rust 在异常穿过守卫时直接 abort。
  - 跳过时打一条 `debug`（默认 `info` 级别看不到，需要 `RUST_LOG=debug`）；
    `install()` 用 `Once`，天然幂等，不会把守卫自己记成「原实现」。
- 验证：`cargo test -p gpui_macos --lib` = **12 passed / 0 failed**，其中 3 个探针跑在
  **子进程主线程**（ObjC 异常不能被 Rust unwind，跑在 libtest 线程会把测试进程一起带走）：
  1. finder observation 注册一次、注销两次 ⇒ 必须不抛；
  2. 普通 `NSObject` 观察者注销两次 ⇒ **必须照旧抛**，子进程应被信号杀掉，测试断言这一点
     —— 这是「作用域没有放大」的证明；
  3. 重复 `install()` 不改变替换结果，每个类保留自己的原实现。
  探针同时断言 `NSObject`/`NSResponder`/`NSView`/`NSControl`/`NSWindow`/`NSTextView`/`NSTextField`
  的该方法都解析到守卫，且 `NSObject` 在被替换集合里。
  `cargo fmt` 与 `cargo clippy -p gpui_macos --all-targets --no-deps` 干净。
- 发布：`fork-0.3.118`（`.gpui-pre/publish` 提交 `f03a8ce`，tag `fork-0.3.118`）；navop 24 处
  patch 与 `Cargo.lock` 已切到 `tag = "fork-0.3.118"`
  （`f03a8ce01c630e4022d728f06ef19c366a7fb9d7`），`cargo check -p one-core --all-targets` 通过。
- **新的真机判据**：先是行为 —— 建连接 → 确定、反复开关弹窗、⌘-Tab 切走再切回、正常退出，
  既不闪退也不卡死。加分项：用 `RUST_LOG=debug` 从终端起进程，日志里应出现
  `skipped the Touch Bar finder's retraction`（默认 `info` 级别看不到）。
  如果仍然出问题，这次会得到**崩溃报告**（而不是卡死）：`.ips` 里若换了类名或换了 reason，
  说明还有别的变体，按同样思路继续收窄。
- **未决**：`window_teardown`（退役方案）与 `RETIRED_WINDOWS` 无上限（§10.1 的 P3）是否回退/补上，
  等这次真机结论后再定。
- **测试包指纹**：`navop-0.19.2-touchbar-kvo-noop-x86_64-apple-darwin.zip`
  （55844353 字节，sha256 `58aa506f2f857bcbe01709872f7b09dc4e5240bd9757776d87e619b0c70917d0`），
  Mach-O UUID `FA4C5EB5-D6C7-323A-9C47-44C115523372`（x86_64），bundle id `com.onetcli.app`、
  ad-hoc 签名、无外部 dylib。二进制里新守卫串各命中 1 次，旧守卫串
  （`ignored an AppKit Touch Bar observer exception`、`took over NSApplication's`）命中 0 次，
  可确认这一版不再有 `_crashOnException:` 那条吞异常逻辑。
  Rosetta 烟测（本机）10 秒存活；测试说明写在
  `~/Downloads/navop-0.19.2-x86_64-touchbar-kvo-noop-测试说明.md`。

### 10.6 红点这条路还没被覆盖：把一次性弹窗也接进统一关闭漏斗（2026-09-27 晚）

- 真机反馈（同一台 Intel Touch Bar 机，包 `0.19.2-touchbar-kvo-noop`，sha256
  `58aa506f2f857bcbe01709872f7b09dc4e5240bd9757776d87e619b0c70917d0`）：
  「点确定不闪退了，点左上角的 x 闪退」。⇒ 守卫只在**我们自己发起的关闭**上生效。
- 触发面确认不窄：`navop#314`（保存连接即崩溃，0.18.5/0.18.6 必现，当天 11 份 `.ips`）
  与 `navop#308` 是同一签名 —— 堆栈仍是
  `NSDisplayCycleFlush → -[_NSTouchBarFinderObservation invalidate] →`
  `removeObserver:forKeyPath:context: → removeObserver:forKeyPath: →`
  `_removeObserver:forProperty: → objc_exception_throw →`
  `-[NSApplication _crashOnException:]`（SIGILL）。另外 #314 的机器试过把 Touch Bar 切成
  功能键模式仍崩 ⇒ 与呈现模式无关。
- 现在的模型是「两条关闭路线的差别」：
  1. **我们这一侧**（保存 / 确定 / 取消按钮、Cmd-W）走 `window.remove_window()` +
     GPUI 延迟回收 ⇒ 现场已经不崩；
  2. **原生红点**走 AppKit 自己的 `windowShouldClose:` → `-[NSWindow close]` ⇒ 窗口是在
     **AppKit 的关闭流程里**被销毁的，此刻窗口的响应者（字段编辑器等）可能已经被 AppKit
     释放，而 Touch Bar 查找器的观察是延迟到下一个显示周期才注销的 —— 守卫再往前一帧也
     救不回一个已经不在的对象。
- 改法（navop 侧，不动 gpui fork）：`install_reusable_popup_close_routes` 收敛成
  `install_popup_close_routes`，并在 `reuse_key` 分支**之外**调用 —— 关闭路线现在对
  **两类弹窗**都装：红点一律先返回 `false` 取消 AppKit 的关闭，再交给
  `close_window_for_reuse` 决定「隐藏留着复用」还是「交给 GPUI 的延迟销毁」。
  一次性弹窗的语义没变（照样销毁），变的只是**销毁时机**：从 AppKit 的关闭流程里挪回
  我们自己的一轮（保存/确定那条已经验证过的路线）。
  ⚠️ **这条只对了一半**：当天真机反馈「确定」确实不崩了，但**红点点仍然闪退** ——
  也就是说「挪回自己的一轮」还不够，见 §10.7。
- 验证：`cargo check -p one-core -p main --all-targets` Finished；
  `cargo test -p one-core --lib` **692 passed / 0 failed**；新增契约测试
  `every_popup_intercepts_the_native_close_route` 锁住「两类弹窗都装了关闭路线，且这条路线
  在 `reuse_key` 分支之外」（本机没有 Touch Bar，行为层面无法复现，结构断言是唯一能自动化
  的防线）。
- ⚠️ 这是**假设驱动**的改法，不是已验证的结论：判据是同一台机器上「点红点关弹窗」不再闪退。
  如果还崩，需要那份 `.ips` 的 `asi` 原文（异常名 / reason / 被观察对象的类名）来定位是
  哪一个变体。
- 旁路发现（独立问题）：应用内更新检查目前拿到 `403 Forbidden (feigeCode/navop)`
  （GitHub Release 接口匿名限流，本机日志多次复现），#314 的机器因此停在 0.18.6 并以为
  「0.18.6 已是最新」。修好之后这条升级路径得先通，否则用户升不上来。

### 10.7 真机判据落在「谁销毁」上：干脆不销毁任何弹窗（2026-09-27 深夜）

- 现场（Intel + Touch Bar，`MacBookPro16,2`，macOS 14.8.9，`0.19.2-touchbar-kvo-noop`）：
  **「确定」不再闪退，点原生红点仍然闪退**。这把 §10.6 的两条路线模型钉住了 —— 变量不是
  「有没有守卫」，而是**是谁发起的销毁**：我们自己那条（`remove_window()` + GPUI 延迟回收）
  已经安全，AppKit 在自己关闭流程里那条还是崩。
- 改法：既然「哪条路线会销毁」就是变量，那就不留任何一条会销毁的路线。
  `crates/core/src/popup_window.rs` 把**所有**弹窗都登记进 `PARKED_POPUPS`（没有复用键的
  一次性弹窗也登记），`close_window_for_reuse` 对弹窗**只隐藏、不销毁**：红点先被 `false`
  取消（AppKit 不参与销毁），随后窗口 `orderOut` 隐藏 + `end_popup_session` 卸载业务 view。
  应用运行期间不再存在「销毁原生窗口」这个动作，AppKit 的延迟注销也就永远踩不到已释放对象。
- 副作用与代价（写在明处）：一次性弹窗没有复用键，隐藏后不会有人重新显示它，下一次打开是
  **新建**窗口 ⇒ 停放窗口数随打开次数增长、直到退出应用。每个停放窗口只是空壳（业务 view
  在关闭时已卸载，见 §4.3），但它仍有自己的 NSWindow / layer。收敛方向已定：
  1. 把热点弹窗（连接表单、端口转发、导入窗口等 `open_popup_window` 调用点）改成
     `open_reusable_popup_window` —— 数量从「打开次数」收敛到「弹窗种类数」，**且不需要任何销毁**；
  2. 只有在还有窗口必须销毁时，才考虑「停放数量上限 + 按我们自己的路线延迟销毁最旧的一个」。
- 计数器口径变化：`live_windows` 现在会随一次性弹窗的打开次数增长（停放窗口也是活窗口），
  这是**预期**，不再是「复用没命中」的信号。判断复用是否生效改看「同一类弹窗第二次打开时
  `opened_windows` 是否不再增长、`live_windows` 是否不变」。
- 验证：`cargo check -p one-core -p main --all-targets` Finished；
  `cargo test -p one-core --lib` **693 passed / 0 failed**。契约测试：
  `popup_windows_are_hidden_and_never_destroyed`（弹窗判断在隐藏之前；销毁动作只允许出现在
  「不是弹窗」与两个失败分支里）、`one_shot_popups_are_registered_as_parked`（一次性弹窗必须
  登记，且必须在 `reuse_key` 的 `else` 分支 —— 一次性 factory 只能调用一次，接到复用路径上会
  panic）、`every_popup_intercepts_the_native_close_route`（关闭路线不能被摘掉）。
- ⚠️ 仍需真机判据（同一台机器）：「点红点关弹窗」既不闪退、也不卡死，而且窗口确实消失了
  （不出现「点了没反应、窗口还留在那里」—— 那说明 `orderOut` 之后又被重新显示或激活）。
  底层修法（让 Touch Bar 查找器那次注销根本不抛）仍在 `fork-0.3.118` 的
  `touch_bar_guard.rs` 里，navop 侧这一层是「即使上游守卫漏了一条路也不崩」的兜底。

### 10.8 守卫自己成了崩溃源：两份新 `.ips` 与 `fork-0.3.119`（2026-09-28）

同一台 Intel + Touch Bar 机器（`0.19.2-touchbar-kvo-noop`，`slice_uuid`
`FA4C5EB5-D6C7-323A-9C47-44C115523372`）交回两份崩溃报告，**都发生在装了 §10.5 那版守卫的
构建里**，而且都不是守卫原本想挡的那个异常：

- **崩溃 A**（`navop-2026-09-28-083919.ips`）：`EXC_BAD_ACCESS` /
  `KERN_PROTECTION_FAILURE`，512 帧互相调用的 `removeObserver:forKeyPath:context:`，栈顶
  撞到栈保护页后 `abort()`。
  根因是**守卫自己的递归**：`fork-0.3.118` 的实现把「同一个替换 IMP」装在 `NSObject` 和
  每个自己实现该方法的 `NSResponder` 子类上，转发目标却按**接收者类**现场查找。
  AppKit 的 `-[NSWindow removeObserver:forKeyPath:context:]` 自己实现了这个方法，并且会
  `[super …]` 交给 `NSObject` 的实现 —— 而那份实现也已被替换：从窗口进入时，替换 IMP 查到
  的是窗口的原始实现，调用它，它又从 `super` 回到我们，于是两者交替递归。
  用 `otool -tvV` 反汇编（release 二进制已 strip，`atos` 无法符号化，只能从帧模式判断）
  在 `+48322721` 处看到 `callq *0x8(%r9)`，正是那次自调用。
- **崩溃 B**（`navop-2026-09-27-165228.ips`）：`SIGSEGV`，发生在
  `-[NSApplication terminate:]` 内部的 Foundation KVO 记账里
  （`_NSKeyValueObservationInfoGetObservances` / `_NSKVONotifyingOriginalClassForIsa`，
  收到的是垃圾 isa `0xd00000020`）。**推断**（未证实）：守卫「凡查找器的注销一律跳过」把
  查找器的 observation 留在被观察对象上，而 KVO 不持有 observer —— 查找器那侧已经丢弃
  observation 时，注册表里留下的是悬空指针，等退出流程遍历它时踩到。

改法（Zed 侧 `crates/gpui_macos/src/touch_bar_guard.rs`，提交 `b73d4c2b25`）：

- **每个类各自的转发器**：`macro_rules! forwarders!` 生成 `forward_0…forward_7` 与
  `forwarder(slot)`；`replace()` 在安装前用 `method_getImplementation` 取该类的原始实现，
  装的是「属于这个类」的转发器，转发目标也就不再依赖接收者是谁。槽位固定 8 个
  （`SLOTS`/`ORIGINALS`/`SLOTS_TAKEN`），装不下时报错并带上类名。
- **默认不装**：只有 `GPUI_MACOS_TOUCHBAR_GUARD` 开了才安装（空 / `0` / `false` / `off`
  视为关）。理由有两条：§10.7 之后弹窗不再销毁，守卫要挡的「视图将死时被注销」已经不复现；
  而崩溃 B 说明带着守卫反而可能引入新的退出期崩溃。要复现老问题，用环境变量把它打开。
- 验证：`cargo test -p gpui_macos --lib` **14 passed / 0 failed**，其中新增
  `a_window_retraction_does_not_call_the_guard_in_a_circle`（窗口发起的注销必须以异常的
  `SIGABRT` 结束，而不是栈溢出的 `SIGSEGV`）、`the_guard_is_off_unless_the_environment_asks_for_it`、
  `installing_twice_leaves_the_runtime_alone`；probe 也覆盖了 `PROBE_OFF` 与槽位一致性。
  clippy 对本 crate 无新增告警（`gpui` crate 里有一处**既有** `redundant_clone` 会中断
  clippy 构建，只能临时 `-A clippy::redundant_clone` 绕过；与本改动无关）。
- 已发布 `fork-0.3.119`（快照提交 `36429a19`，`zed-rev` = `b73d4c2b25`），navop 的
  `[patch.crates-io]` 24 条与 `Cargo.lock` 一并切过去；`cargo check -p one-core -p main
  --all-targets` 通过。
- ⚠️ 发出去给真机的那包是「**不销毁弹窗**（§10.7）+ **守卫关闭**」的组合：即最保守、
  引入最少新机制的形态。老崩溃家族（窗口销毁期被注销）由「不销毁」消除；如果仍偶发，再用
  `GPUI_MACOS_TOUCHBAR_GUARD=1` 从终端启动来验证「修好的守卫」是否有效 —— 这是两条独立的
  防线，**不要**在没有真机证据的情况下同时打开。

### 10.9 守卫不能关：关掉之后「确定 / 取消」又崩了，恢复默认开启（2026-09-28 中午）

- 现场（同一台 Intel + Touch Bar，`0.19.2-touchbar-hide-only-noguard`，`slice_uuid`
  `F6C3E655-…`）：**红点关闭与 RDP 全屏不崩了** —— §10.7 的「不销毁」确实治住了销毁触发的
  那个变体 —— 但**点弹窗的「确定 / 取消」仍然闪退**。
- 这份报告（`navop-2026-09-28-123639.ips`）的 `asi` 栈与最早那份完全同型：
  `-[NSApplication _crashOnException:]` ← `__exceptionPreprocess` ← `objc_exception_throw`
  ← `-[NSObject _removeObserver:forProperty:]` ← `removeObserver:forKeyPath:` ←
  `removeObserver:forKeyPath:context:` ← `-[_NSTouchBarFinderObservation invalidate]` ←
  `___NSTouchBarFinderSetNeedsUpdateOnMain_block_invoke_2` ← `NSDisplayCycleObserverInvoke`
  ← `NSDisplayCycleFlush` ← `CATransaction` ← 主 runloop；`EXC_BAD_INSTRUCTION (SIGILL)`、
  Trap 6。**关键点：这条链里没有一帧与「窗口被销毁」有关**，被注销的 observation 属于一个
  还活着的视图。
- 结论：§10.8 把守卫默认关掉是**假设被真机否掉**。当时的推理是「不销毁窗口 ⇒ 守卫要挡的场景
  消失」，但证据说明这个异常不只出现在视图将死时：同一轮显示周期里两次 `nextResponder` 链变化
  就够了（上游 RustyCAN#96 记录的就是这个）。所以恢复默认开启；递归缺陷已在 §10.8 修掉，
  两者不冲突。
- 改法：`fork-0.3.120`（zed 提交 `7485783f80`，快照 `053628ba`）把开关语义反过来 ——
  **默认安装**，只有 `GPUI_MACOS_TOUCHBAR_GUARD` 显式写成 `0`/`false`/`off`/`no` 才不装；
  空串按「没设」处理（防止包装脚本把守卫静默关掉）。`guard_requested_by` → `guard_disabled_by`
  （语义取反）。
- 验证：`cargo test -p gpui_macos --lib` **14 passed / 0 failed**，
  `the_guard_is_on_unless_the_environment_says_off` 同时断言「什么都不设 ⇒ 守卫确实装上」
  与「设成 `0` ⇒ 确实不装」；`cargo check -p one-core -p main --all-targets` 通过。
- 发给真机的第 5 版组合：**不销毁弹窗（§10.7）+ 守卫默认开启（本节）**。两个变体各有真机
  证据支撑：红点 / RDP 全屏那条由「不销毁」治住，「确定 / 取消」那条由守卫治住。
- ⚠️ 仍未解释：`navop-2026-09-27-165228.ips`（退出应用时 Foundation KVO 记账里
  `KERN_INVALID_ADDRESS`）。它只在带着守卫时出现过，推断是「跳过查找器注销」留下的悬空
  observation，但没有证据；目前没有别的办法通过「确定 / 取消」这条路径，只能带着这个已知风险。
  真机若再出现「退出应用时崩」，用 `GPUI_MACOS_TOUCHBAR_GUARD=0` 启动做对照，并把那份 `.ips`
  交回来。

### 10.10 不再「跳过」而是「只吸收那一个异常」：守卫重写 + `fork-0.3.121`（2026-09-28 下午）

- §10.5 的做法是「凡是查找器发起的注销，一律直接 `return`」。**这太重**：KVO 的契约要求合法
  注册必须可注销，跳过第一次合法注销会留下「已注册、但调用方以为已注销」的观察项 —— 这正是
  `navop-2026-09-27-165228.ips`（退出应用时 Foundation KVO 记账里 `KERN_INVALID_ADDRESS`）
  的头号嫌疑。独立探针证实了这一点：守卫开着时，本该结束通知的那次注销没有生效。
- 于是改成：**每一次注销都真的执行**，只吸收「查找器那一次重复注销」产生的异常。
  - 抛出时只匹配这一种组合：观察者是查找器的、key path 是 `nextResponder`、异常名是
    `NSRangeException`、reason 含 `because it is not registered as an observer`。匹配 ⇒ 记一条
    `warn`（观察者类与地址、被观察对象类与地址、key path、context、异常名与 reason）后吸收，
    这条日志正是真机崩溃报告一直缺的证据（它们的栈止于 raise，没有名字、reason、对象、key path）。
  - 不匹配 ⇒ 原样重抛，让它像从被替换的实现里抛出一样以 `C-unwind` 穿过我们。Cocoa 不是异常
    安全的，网撒太大会把 bug 藏起来而不是修掉。
- 验证：`cargo test -p gpui_macos --lib` **16 passed / 0 failed**；新增
  `the_finders_first_retraction_removes_and_the_duplicate_is_absorbed`（旧版做错的那件事，含
  「通知计数」双向证明：注册期间有一次通知、注销后不再有）、
  `another_key_paths_retraction_is_thrown_back` 与
  `only_the_unregistered_next_responder_retraction_counts`（把网收窄）、以及 `rethrow` probe。
- ⚠️ **单测绿不等于真机可用**：`fork-0.3.121` 的 `@try`/`@catch` 用的是
  `objc2::exception::catch`，它把 Rust 闭包包在 `@try` 里。**navop 的 `[profile.release]` 是
  `panic = "abort"`**（为了省 `__eh_frame` / `__gcc_except_tab`，见 `Cargo.toml:311`），这种
  构建下 ObjC 的 unwind 穿过一个 Rust 帧会变成 `panic in a function that cannot unwind` 并
  直接 abort。独立探针（`/tmp/navop-guard-release-check`，`#[path]` 引入真实守卫源码 + 与 navop
  一致的 release profile）复现：dev 构建能吸收重复注销，换成 release 配置后同一操作退出码
  **134**，`@catch` 根本没执行。**cargo 对 test/bench profile 忽略 `panic` 设置**，所以 16 个
  单测全过和 release 包会死可以同时成立。

### 10.11 把 `@try`/`@catch` 挪到 ObjC 里编译：`fork-0.3.122`（2026-09-28 傍晚）

- 修法：`@try`/`@catch` 由 crate 自己编译的 Objective-C 实现
  （`crates/gpui_macos/objc/gpui_macos_try_remove.m`，`build.rs` 用 `cc` 编译），它的 `@try`
  体里**直接调用原实现**，raise 与 `@catch` 之间没有任何 Rust 帧 —— 于是与构建的 panic 策略
  无关。shim 返回被 retain 的异常对象，Rust 侧照旧判断：是查找器的重复注销就吸收 + 记 warn，
  其余交给 shim 的 `@throw` 原样抛回去（仍以 `C-unwind` 穿过被替换的实现）。
- 同时把 `objc2` 的 `exception` feature 关掉：树里已经没人用它，留着只会把同一个 abort 再请回来。
- 验证（全部在当前树上重跑）：
  - 独立探针 + **navop 的 release profile**：新 shim 吸收重复注销并打出
    `absorbed the Touch Bar finder's duplicate retraction … NSRangeException: Cannot remove an
    observer … because it is not registered as an observer`，通知计数 `before=1 / after=1`
    （注销真的生效），进程正常退出。
  - 同一探针换成 `fork-0.3.121` 的守卫做对照：`panic in a function that cannot unwind` →
    `thread caused non-unwinding panic. aborting.` → 退出码 **134**。探针确实能检出这个问题。
  - `cargo test -p gpui_macos --lib` **16 passed / 0 failed**。
- 已发布 `fork-0.3.122`（快照 `4188a6b2`，`zed-rev` = `283416671d`；已确认快照里带上了
  `crates/gpui_macos/build.rs`、`objc/gpui_macos_try_remove.m` 与 `cc` build-dependency —— 少
  任何一个都会在链接期炸掉），navop 的 24 条 `[patch.crates-io]` 与 `Cargo.lock` 一并切换。
- 发给真机的组合：**不销毁弹窗（§10.7）+ `fork-0.3.122` 的守卫**。
- 遗留：`window_teardown` 的「不销毁」现在是这版守卫的兜底；等守卫在真机存活后可以撤销，
  让原生窗口重新被释放（停放窗口的内存代价随之消失）。


### 10.12 「不销毁窗口」收进一个开关，当前只给 Intel 版 macOS 包打开（2026-09-29）

§10.7 的「不销毁任何弹窗」当时是无条件生效的：所有平台、所有构建都在付「隐藏的原生窗口
一直占着 NSWindow / CAMetalLayer」这个代价，而当时只有 Intel 机型报过闪退。这一版把它
收敛成一个 cargo feature：

- **开关**：`crates/core/Cargo.toml` 的 `macos-touchbar-window-hide`（`default = []`），
  `main/Cargo.toml` 转发。整个链路只读**一个常量**：
  `one_core::window_close::HIDE_WINDOWS_ON_CLOSE = cfg!(all(target_os = "macos", feature = "macos-touchbar-window-hide"))`。
  之所以不做成散落的 `#[cfg]`，是因为 `popup_window.rs`（弹窗注册与关闭路由）、
  `window_close.rs`（关闭漏斗）、`remote_file_editor::editor_window_visibility`（编辑器窗口）
  三处必须**同时**成立：任何一处单独打开，都会出现「窗口被隐藏但没进复用表」或者
  「业务会话已结束却还在等这个窗口」的错配。
- **生效范围**：只有发布流水线为 `x86_64-apple-darwin` 传 `--features macos-touchbar-window-hide`
  （`release.yml` 里的 `extra_features`，其余 target 为空）。ARM macOS / Windows / Linux 拿到的是
  加这套机制之前的形态：关闭即销毁。打包契约测试 `script/test-release-packaging.mjs` 钉住三点 ——
  feature 只出现一次、只挂在 `x86_64-apple-darwin` 判定下、两条编译命令（`cargo zigbuild` /
  `cargo build`）都通过同一个变量消费它。
- **为什么当前不开给 Apple Silicon**（2026-09-30 修正）：**不是**「ARM 没有 Touch Bar」——
  13 英寸 MacBook Pro 的 M1（2020）与 M2（2022）都是 Apple Silicon + Touch Bar，「Touch Bar
  只存在于 x86_64 机型」这个说法是错的。当前只给 Intel 版 macOS 包打开的**唯一**理由是
  「已复现的闪退现场都在 Intel 机器上」（#262 的 `MacBookPro16,2`）。隐藏窗口的代价是真实的
  （窗口不释放，`CAMetalLayer` 与它的缓冲区留在进程里），所以在没有现场证据的机型上先不付这笔
  代价。判据里没有架构条件，Apple Silicon 侧要复现或验证时给对应构建打开同一个 feature 即可，
  不用改代码。
- **顺带补齐「所有打开窗口的地方」**：开关生效时，弹窗一律走复用（`open_reusable_popup_window`
  + 按目标取键，例如 `connection-form:ssh:42`、`table-export:{conn}:{db}.{schema}.{table}`），
  视图内部原先自己 `window.remove_window()` 的 15 个表单/工具栏窗口改成走
  `one_core::window_close::close_window_for_reuse(window, cx)`。源码契约测试
  `secondary_windows_never_destroy_themselves`（`crates/core/src/window_close.rs`）逐文件断言
  这些文件里不再出现 `window.remove_window()`：漏掉任何一个入口，那条入口就会照旧销毁原生
  窗口，#308 的「确定/取消」、#314 的「保存」都是这么漏出来的。
- **仍未解决的**：`fork-0.3.122` 的守卫（§10.11）依然是「隐藏」之外的第二道兜底，两者都保留。
  等守卫在真机稳定，可以反过来撤掉隐藏、让原生窗口重新被释放。
