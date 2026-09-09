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

创建为普通 GPUI window，并注册窗口关闭处理。默认关闭路径最终也是：

```rust
window.remove_window();
```

不要使用 `minimize_window()` 或仅隐藏窗口代替关闭。

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
- 某些窗口在使用过程中被隐藏或最小化，而不是走 remove 流程。

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
