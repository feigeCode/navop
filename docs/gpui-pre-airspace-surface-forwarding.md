# gpui-pre fork 补丁：Windows 原生 RDP 浮层（airspace）修复

> **影响**：`backend_preference: windows_native` 的 RDP 会话——远端桌面永远盖在 GPUI 之上，
> 标签右键菜单、弹层被吃掉，侧栏被压成一条碎片。上游 issue **#310**。
>
> **改动面**：`feigeCode/gpui-pre` fork 两处（2 个文件，+41/-9 行）。
> navop 侧另有配套改动（已在仓库内）。
>
> 补丁文件：`.workbuddy/tmp/gpui-pre-airspace-fix.patch`（`git apply` 即用）

---

## 1. 结论

原生 RDP 的呈现方式是：把 ActiveX 子窗口挂进 GPUI 的 DirectComposition visual 树，
让它的栅格化变成**位于 GPUI overlay 层之下**的一个 visual（`gpui/src/window.rs:1504` 的
`place_below(id, overlay_surface)` 已经把合成顺序写对了）。但整条链路上有两个 bug 把它堵死：

| # | 位置 | 症状 | 修复 |
|---|---|---|---|
| **A** | `crates/gpui/src/window.rs`，`ManagedPlatformSurface` | 装饰器漏转发 `set_window_content`，调用落到 trait 默认实现直接 bail：`composing an existing window into a surface is not supported by this platform` | 补 4 行转发 |
| **B** | `crates/gpui_windows/src/directx_renderer.rs`，`DirectCompositionPortal::set_visible` 与 `DirectComposition::rebind_portal` | 用 `cast::<IDCompositionVisual3>()?.SetOpacity2(..)` 控制可见性；而合成设备由 `DCompositionCreateDevice`（v1）创建，其 visual 只到 `IDCompositionVisual`，`cast` 必然 `E_NOINTERFACE`（`0x80004002` 不支持此接口） | 可见性改为「内容的有无」表达（`SetContent`）；Visual3 可用时保持原行为 |

**B 的破坏力比看上去大**：navop 把「隐藏失败」解释为「这个 surface 不可用了」，
于是**一个已经成功挂载的合成 surface 被整个退回**，会话降级为普通子窗口 ——
正是你看到的症状。

**GPUI 底层确实需要改**，这两处都是 GPUI 自身的缺陷，不是 navop 用法问题。

**本地验证状态：已通过**（2026-10-09，真机 `<RDP 主机>`，`backend_preference:
windows_native`）。修复后构建 24m06s 完成，§6 判据全部通过 —— `0x80004002` 归零、
`composition_attached` 出现、`cloaked=true` 保持不被撤销、无退回 plain child。
判据脚本与实测对比见 §6。

---

## 2. 故障链（逐跳可复核）

| # | 位置 | 行为 |
|---|---|---|
| 1 | `remote_desktop_view/src/view/windows_native.rs:765` | `window.enable_window_composition().and_then(\|c\| c.create_native_surface())` — 前提是 DirectComposition **开启** |
| 2 | `gpui/src/window.rs:1496` `register_platform_surface()` | 把平台 attachment 包进装饰器 `ManagedPlatformSurface`；紧接 `place_below(id, overlay_surface)` 把 native surface 放到 overlay 层之下 ✔ |
| 3 | `windows_native_composition.rs:56` | `attachment.set_window_content(Box::new(self.window))` |
| 4 | 期望 → `directx_renderer.rs` `attach_window_content` | `CreateSurfaceFromHwnd` + `visual.SetContent(&surface)` + `Commit` ✔（实测这三步**能**成功） |
| 5 | **实际（修复前）** → `gpui/src/platform.rs:116` | trait 默认实现：`bail!("composing an existing window into a surface is not supported by this platform")` |
| 6 | `attach()` 接着调 `set_visible(false)` | → `DirectCompositionPortal::set_visible` → `cast::<IDCompositionVisual3>()` → **`E_NOINTERFACE`** |
| 7 | `WindowsNativePresentationSink::sync_composition_bounds` | 把「镜像失败」当作 surface 不可用 → `fallback_to_plain_child("set_bounds_composition_sync_failed")` → 撤销 cloak 与 `WS_EX_LAYERED` → **退回普通子窗口，airspace 症状复现** |

### 日志证据（真机 `<RDP 主机>`）

修复 A 之后、修复 B 之前（错误从「not supported by this platform」变成 COM 错误）：

```
WARN remote_desktop_view::view::windows_native: failed to compose the Windows native RDP overlay;
     keeping it as a plain child window
     error=不支持此接口 (0x80004002) overlay_hwnd=<n>
```

修复 B 的中间验证态（把「隐藏」降级为非致命后，第二条失败路径暴露出来）：

```
WARN ...windows_native_composition: the portal visual refused to start hidden; composing it anyway
     error=不支持此接口 (0x80004002)
INFO ...windows_native_composition: composed the Windows native RDP overlay into the GPUI visual tree
     stage="composition_attached"                       ← 挂载成功
WARN ...windows_native: failed to mirror Windows native RDP bounds into the composition tree
     error=不支持此接口 (0x80004002)                     ← 又一次 set_visible(false)（bounds 为 None 的分支）
WARN ...windows_native: retiring the Windows native RDP composition surface and presenting the
     session as a plain child window again stage="set_bounds_composition_sync_failed"
```

注意最后那条的顺序：`stage="show" ... requested_visible=true` 出现在 bounds 同步**之后**，
说明那次 bounds 同步走的是「overlay 尚未显示 → 被裁掉 → `None` 分支」，
该分支里的失败同样是 `set_visible(false)` —— **三处失败同一个根因**。

### 为什么 `set_visible` 的 Visual3 依赖一定是错的

- `SetOpacity2` / `SetVisible` / `SetDepthMode` 只声明在 `IDCompositionVisual3`
  （windows-0.58 `DirectComposition/mod.rs:2447-2474`）；
- `IDCompositionVisual2` 只有 `SetOpacityMode` / `SetBackFaceVisibility`（同上 `:2425`）；
- GPUI 的设备来自 `DCompositionCreateDevice`（v1，`directx_renderer.rs:1861`），
  `CreateVisual()` 出来的是 **`IDCompositionVisual`**；只有 `DCompositionCreateDevice3`
  的设备其 `CreateVisual()` 才是 `IDCompositionVisual2`，且只有 v3 设备的 visual 实现
  `IDCompositionVisual3`；
- `SetOffsetX2`（`set_bounds` 用）与 `SetLeft2`（clip 用）**都是 v1 方法**
  （同上 `:2313` / `:1297`），所以几何路径一直是好的 —— 只有可见性路径从未成功过；
- `create_portal_state()` 把 `visible` 初始化为 `true`，而 GPUI 自身几乎不调 `set_visible`，
  于是这行 `cast` 在 navop 接进来之前**从未被执行过**，是一条未经测试的死路径。

---

## 3. fork 侧改动

- 仓库：`https://github.com/feigeCode/gpui-pre.git`
- navop 当前钉的 tag：`fork-0.3.126`（rev `9e410c8f92aa839ac4f855ee5c06ce72e40e65ad`，
  `crates/gpui/Cargo.toml` version `0.3.126`）
- 建议新版本：**`0.3.127`** → tag **`fork-0.3.127`**

### 改动 A：`crates/gpui/src/window.rs`

`impl crate::PlatformSurfaceAttachment for ManagedPlatformSurface` 块内，
`fn set_visible` 之后、`fn platform_handle` 之前（该 rev 下约 1276 行）：

```diff
@@ -1276,6 +1276,10 @@ impl crate::PlatformSurfaceAttachment for ManagedPlatformSurface {
         self.platform_surface.set_visible(visible)
     }
 
+    fn set_window_content(&self, content: Box<dyn Any>) -> anyhow::Result<()> {
+        self.platform_surface.set_window_content(content)
+    }
+
     fn platform_handle(&self) -> anyhow::Result<Box<dyn Any>> {
         self.platform_surface.platform_handle()
     }
```

`ManagedPlatformSurface` 实现了 `set_bounds` / `bounds` / `set_parent_origin` /
`compositor_recreated` / `set_compositor_recreated_callback` / `set_visible` / `platform_handle`
七个方法，**唯独漏了 `set_window_content`** —— 任何绕过 `with_composition_surface`
直接拿 `platform_surface()` 的调用方都会被默认实现挡掉。

### 改动 B：`crates/gpui_windows/src/directx_renderer.rs`

新增 helper（放在 `attach_window_content` 之后）：

```rust
/// Mirrors `visible` onto a portal visual.
///
/// The opacity setters live on `IDCompositionVisual3`, but only the visuals of a
/// device created by `DCompositionCreateDevice3` implement that interface — the
/// device GPUI creates is the v1 one, whose visuals stop at
/// `IDCompositionVisual`. Casting there returns `E_NOINTERFACE`
/// (`0x80004002`), which used to fail every visibility change and, through the
/// callers that treat a failed hide as "this surface is unusable", retired an
/// otherwise working composition surface.
///
/// A portal presents `window_content`, so attaching and detaching that content
/// says the same thing on a device of any version: a portal with no content
/// rasterizes nothing.
fn apply_portal_visibility(state: &DirectCompositionPortalState, visible: bool) -> Result<()> {
    if let Ok(visual3) = state.visual.cast::<IDCompositionVisual3>() {
        return unsafe { visual3.SetOpacity2(if visible { 1.0 } else { 0.0 }) };
    }
    match (visible, state.window_content.borrow().as_ref()) {
        (true, Some(content)) => unsafe { state.visual.SetContent(&content.surface) }
            .context("restoring the composed window content"),
        // Nothing is presented either way, so there is nothing to toggle off.
        (true, None) => Ok(()),
        (false, _) => unsafe { state.visual.SetContent(None::<&windows::core::IUnknown>) }
            .context("detaching the composed window content"),
    }
}
```

`set_visible` 改为调用它（删掉原来的 `cast(...).SetOpacity2(...)`）；`rebind_portal`
里同样的 `cast` 也改为调用它，并把顺序调整为**先恢复内容、再应用可见性**
（隐藏态定义为「没有内容」，顺序反了隐藏会失效）。`rebind_portal` 是合成器重建
（GPU 掉设备）时的路径，不改的话掉一次设备就再也 rebind 不回来。

完整 diff 见 `.workbuddy/tmp/gpui-pre-airspace-fix.patch`：

```
 crates/gpui/src/window.rs                   |  4 +++
 crates/gpui_windows/src/directx_renderer.rs | 45 +++++++++++++++++++++++------
 2 files changed, 40 insertions(+), 9 deletions(-)
```

---

## 4. 落地步骤

本机没有 Zed 检出（`/d/workspace/zed`、`/d/workspace/.gpui-pre` 都不存在），
而 `script/patch-local-gpui-pre.py` / `publish-gpui-pre-fork.py` 的快照流水线需要 Zed checkout，
所以**最短路径是直接改 fork**：

```bash
git clone https://github.com/feigeCode/gpui-pre.git
cd gpui-pre
git apply /path/to/gpui-pre-airspace-fix.patch

# 版本号 0.3.126 -> 0.3.127（crates/gpui/Cargo.toml 及其余 gpui-pre-* crate）
git commit -am "fix(windows): compose existing window content through the portal decorator"
git tag fork-0.3.127
git push origin main --tags
```

navop 侧切过去（脚本会重写 `Cargo.toml` 里 21 条 `[patch.crates-io]` 的 tag 并跑 `cargo update`）：

```bash
script/migrate-to-git-fork.py \
    --fork-url https://github.com/feigeCode/gpui-pre.git \
    --tag fork-0.3.127
# 加 --dry-run 可先预览
```

> ⚠️ fork 的 `main` 是流水线生成的快照，之后若再跑 `publish-gpui-pre-fork.py` 推新快照，
> 直接提交的补丁**会被覆盖**。长期要保住得让改动进 Zed 快照源，或推动上游 Zed 修（见第 7 节）。

### 本地验证用的临时手法（收尾要还原）

因为 fork 还没改，本机是**就地修改 cargo 的 git 检出**（不碰 `Cargo.toml` / `Cargo.lock`）：

| 项 | 值 |
|---|---|
| 检出 | `~/.cargo/git/checkouts/gpui-pre-eda588a4da6ba6c6/9e410c8` |
| 原件备份 | `.workbuddy/tmp/gpui-window.rs.orig` |
| 当前补丁 | `.workbuddy/tmp/gpui-pre-airspace-fix.patch` |

> **踩坑：cargo 不认 git 检出里的文件改动。**
> git/registry 源被当作不可变快照，直接改检出目录里的文件后 `cargo build` 会**完全跳过**
> 对应 crate 的编译。必须显式失效，而且**两个包都要清**（改动跨 `crates/gpui` 与
> `crates/gpui_windows` 两个 crate）：
>
> ```bash
> cargo clean -p gpui-pre -p gpui-pre-windows --offline
> ```
>
> 之后日志里才会出现 `Compiling gpui-pre ...#9e410c8f` 与 `Compiling gpui-pre-windows ...`。

---

## 5. navop 侧配套改动（已在仓库内，你自己提交）

### 5.1 `main/src/main.rs` — DirectComposition 必须保持开启

历史上为了让 ActiveX 子窗口可见，`main.rs` **无条件**设置
`GPUI_DISABLE_DIRECT_COMPOSITION=1`；而 `windows-native-rdp` 是 default feature
（`main/Cargo.toml:136`），等于把整条合成路径变成死代码 —— 这是 issue #310 一直复现的直接原因。

```diff
-    if remote_desktop::windows_native_rdp_compiled() {
+    if remote_desktop::windows_native_rdp_compiled()
+        && std::env::var_os("NAVOP_RDP_DISABLE_COMPOSITION").is_some()
+    {
         unsafe { std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1"); }
     }
```

默认不再关闭；`NAVOP_RDP_DISABLE_COMPOSITION` 退化为诊断开关，用于对照复现经典
HWND swap-chain 路径下的 airspace 症状。契约测试
`windows_native_rdp_keeps_direct_composition_unless_the_diagnostic_switch_is_set`
断言「标记 → 开关 → setter → `gpui_platform::application()`」的先后顺序。

### 5.2 `remote_desktop_view/src/view/windows_native_composition.rs` — attach 的诊断与容错

- `set_window_content` 与 `set_visible` 各自带上 `.context(...)`：两者都经过同一个 attachment，
  而 `E_NOINTERFACE` 是从底下三层（含 DirectComposition 的 `cast`）冒上来的，
  没有 context 根本无法归因 —— 这次的定位正是靠它。
- `set_visible(false)` 降级为**非致命**：隐藏只是为了避免零尺寸 portal 闪一下错误矩形，
  而 surface 初始 clip 为 0 面积、`sync_bounds` 在同一轮就会设好 bounds，
  所以平台拒绝隐藏时不该丢掉已经建好的合成 surface（这正是 bug B 的放大效应）。

### 5.3 `main/src/main.rs` — 退出里程碑日志

`Navop run loop returned; application resources released`：GPUI 的 `run` 返回之后进程才进入
`ExitProcess` 逐个执行 `DLL_PROCESS_DETACH`。有这行才能把「卡在应用/GPU 资源析构」
与「卡在进程退出」分开（另有一个与本修复无关的「关闭应用卡住」问题，见第 8 节）。

---

## 6. 验证判据

### 编译

```bash
python .workbuddy/build/cargo-msvc.py build -p main --bin navop \
    --config .workbuddy/tmp/navop-nodbg.toml
```

本机约束：`CARGO_BUILD_JOBS=2`、`CARGO_INCREMENTAL=0`。`main` 的 codegen 峰值会撞
提交上限 23.2GB（物理 5.9GB + 页文件 17.3GB），编译成功是**概率性**的；
失败形态是 `memory allocation of 2097152 bytes failed` + `exit code 0xc0000409`
（后者是伪装）。降并发重跑即可，**不要 clean**。

### 运行期（真机，`backend_preference: windows_native`）

| 判据 | 期望 |
|---|---|
| `Direct Composition is disabled`（`gpui_windows::directx_renderer`） | **0 次**（历史每次启动必出 1 条，位置固定在 `navop::home_tab::data` 之前） |
| `failed to compose the Windows native RDP overlay` | **消失** |
| `refused to start hidden` | **消失** |
| `composed the Windows native RDP overlay into the GPUI visual tree stage="composition_attached"` | **出现** |
| `failed to mirror ... bounds into the composition tree` / `retiring the ... composition surface` | **消失** |
| `overlay_cloak ... cloaked=true` 之后不出现 `cloaked=false` | 合成没被退回 |
| 视觉：侧栏宽度 | 正常（不再是 10px 碎片） |
| 视觉：标签右键菜单 / 弹层 | 能**盖在远端桌面之上** |

对照组：设 `NAVOP_RDP_DISABLE_COMPOSITION=1` 启动，应复现旧症状（浮层被远端桌面吃掉）。

### 锁屏下的验证手法（本机实测有效）

`SendInput` 在锁屏下无效（事件进安全桌面），`BitBlt` 也只会截到锁屏。但 `PostMessage` 有效：
GPUI 的 `handle_mouse_down_msg` 直接从 `lparam` 取**客户端坐标**
（`gpui_windows/src/events.rs:486`），不读 `GetCursorPos`、也不看前台窗口；
双击由 GPUI 自己的 `click_state.update()` 按时间+位置算出来，所以 post 两组 down/up 即可触发。

```bash
python .workbuddy/tmp/rdp-test/gui.py info
python .workbuddy/tmp/rdp-test/gui.py seq <pid> <out-prefix> pdclick:400,255 wait:12 wait:30
```

像素级验证（浮层是否真的盖住远端桌面）仍然必须解锁才能做。

### 实测结果（2026-10-09，真机 `<RDP 主机>`）

修复后的构建（`Finished dev profile in 24m 06s`，无 OOM）在**默认合成路径**下跑通，
判据全部通过。同一判据脚本对修复前日志（`testA4`）与修复后日志（`testA5`）的对比：

| 判据 | 修复前 | 修复后 |
|---|---|---|
| `Direct Composition is disabled` | 0 | 0 |
| `failed to compose ... overlay` | 0 | 0 |
| `refused to start hidden` | **1**（line 90） | **0** |
| `failed to mirror ... bounds` | **1**（line 883） | **0** |
| `retiring the ... composition surface` | **1**（line 884） | **0** |
| `composition_attached` | 1 | 1 |
| cloak 顺序 | `true@92` → **`false@885`** | `true@92` → 之后**无** `false` |
| `0x80004002`（E_NOINTERFACE）计数 | **2**（90, 883） | **0** |

修复前的完整故障链（同一次会话内 4 秒走完，可直接对号入座）：

```
90  WARN  the portal visual refused to start hidden ... error=不支持此接口 (0x80004002)
91  INFO  composed ... into the GPUI visual tree stage="composition_attached"
92  INFO  ... overlay cloak state ... cloaked=true
883 WARN  failed to mirror Windows native RDP bounds into the composition tree error=不支持此接口
884 WARN  retiring the Windows native RDP composition surface and presenting ... plain child window again
885 INFO  ... overlay cloak state ... cloaked=false
886 INFO  ... overlay layered state ... layered=false
```

修复后：`refused to start hidden` 不再出现、bounds 同步成功、`cloaked` 保持 `true`、
`layered` 保持 `true`，`0x80004002` 归零。`cloaked=true` 保持本身就是核心证据 ——
DWM cloak 生效后该子窗口不再参与屏幕 Z 序，其像素改由 DirectComposition 树输出，
GPUI 的 overlay surface 因而能盖在其上。

复现命令（锁屏、独立数据目录，`pdclick:400,255` 命中「我的win」卡片）：

```bash
# 启动（必须与长 sleep 同一条命令，否则 bash 结束会回收进程树）
NAVOP_REMOTE_DESKTOP_DIAGNOSTICS=1 <py> .workbuddy/tmp/rdp-test/launch_detached.py \
    .workbuddy/tmp/testA5-stdout.log D:/workspace/navop/.workbuddy/tmp/navop-portable8; sleep 10800
# 驱动
<py> .workbuddy/tmp/rdp-test/gui.py seq <pid> <out-prefix> pdclick:400,255 wait:15 wait:35 shot:session
# 判据
<py> .workbuddy/tmp/rdp-test/assert_diagnostics.py .workbuddy/tmp/testA5-stdout.log
```

`assert_diagnostics.py` 会先剥离日志里的 ANSI 转义序列再比对：navop 的 tracing 输出
把字段名与 `=` 分开着色（`cloaked\x1b[0m\x1b[2m=\x1b[0mtrue`），直接子串匹配会静默漏判。

---

## 7. 建议同时提上游

两处都是 GPUI 自身缺陷：

1. `ManagedPlatformSurface` 漏转发 —— 影响所有绕过 `with_composition_surface`
   直接调用 `platform_surface()` 的集成方，不限 Windows。
2. `DirectCompositionPortal::set_visible` 依赖 v1 设备拿不到的 `IDCompositionVisual3` ——
   `set_visible(false)` 在任何 v1 设备上都是坏的，`rebind_portal` 同样。

上游合并后，这份 fork 补丁就可以摘掉。

---

## 8. 不在本修复范围内的问题

- **「关闭应用卡住」**：与本修复无关 —— 合成路径修好之后**依然复现**。2026-10-09 实测：
  RDP 会话建立后向主窗口 `PostMessage(WM_CLOSE)`，窗口在 **11.4s** 后转为隐藏
  （`visible=false`），但进程 **75s** 后仍驻留；`taskkill /F` 返回 1，
  报「无法终止 PID 为 10984 的进程。原因: 没有此任务的实例在运行」。
  日志显示**应用层退出流程完整走完**（RDP 事件订阅销毁 → host 窗口销毁
  `destroy_window_result=1 window_still_alive=0` → `overlay_destroyed` → 两组
  `shutdown completed`），并打印出 5.3 的里程碑 `Navop run loop returned;
  application resources released`；其后进程状态：

  | 观察项 | 数值 |
  |---|---|
  | 线程数 | 59 → **1** |
  | 工作集 | 162MB → 42MB → 25.6MB → 13MB（持续回收，但进程条目始终不消失） |
  | 非 Windows 模块 | `tsbx.dll`（WorkBuddy 沙箱注入）、`ichat_bundle64.dll`、`isgpet_bundle64.dll`、`PicFace64.dll`、`systembeautify_bundle64.dll`、`Resource.dll`（后五个来自搜狗输入法） |

  结论：卡点在 `ExitProcess` 的 `DLL_PROCESS_DETACH`，**不在**应用/GPU 资源析构，
  用户态无解（需重启）。注意本次测试的 navop 是被 WorkBuddy 沙箱注入的（`tsbx.dll`），
  **用户直接双击 exe 启动、不经沙箱时未必复现**；搜狗输入法那几个 TSF 模块是更普遍的嫌疑。
  5.3 的里程碑日志正是用来把「卡在资源析构」与「卡在进程退出」分开的。
- **会话拆除未释放 composition surface**：`WindowsNativeAdapter` 的 `Drop` 只关 host/overlay 窗口；
  `WindowCompositionSurface` 没有 Drop、`remove_surface` 需要 `&Window`。但 `begin_close`
  已把该 visual 置为不可见，属**资源残留、非视觉故障**，优先级低。

---

## 9. 顺带发现的构建环境缺陷（建议单独提）

`crates/core/build.rs` 的 `load_workspace_env_files()` 对**可能不存在**的文件无条件声明
`rerun-if-changed`：

```rust
for file_name in [".env.local", ".env"] {
    let path = workspace_dir.join(file_name);
    println!("cargo:rerun-if-changed={}", path.display());   // 文件不存在时 cargo 判定为已变化
    ...
}
```

Cargo 把「缺失路径」判为已变化 ⇒ `one-core` **每次构建都被判脏** ⇒ 级联重编整个工作区
（本机实测一次 cargo 调用要 25-30 分钟，代价极高）。

- 证据：`target/debug/.fingerprint/one-core-*/run-build-script-build-script-build.json` 里的
  `"RerunIfChanged":{"paths":["../..\\.env.local","../..\\.env"]}`，且 `invoked.timestamp`
  每次构建都刷新。
- 正解：只在 `path.exists()` 时才打印该指令。
