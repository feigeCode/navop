# Windows 原生 RDP「合成成功但屏幕全空」的根因：`DWMWA_CLOAK` 时序

> **影响**：`backend_preference: windows_native` 的 RDP 会话——连接成功、日志零 WARN，  
> 但 RDP 视图区域**永远是 GPUI 窗口背景色 `#171717`**。既不是远端桌面，也不是浮层自己的黑色。
>
> **改动面**：navop 仓库 6 个文件（`crates/remote_desktop_view` 5 个 + `main/src/main.rs`）+ smoke 工具；  
> **GPUI fork 不需要新增改动**（上一份文档里的 A/B 两处修复仍然必需，那是另一个独立缺陷）。  
> —— 这条只对 §1–§10 成立：§11 的按需 cloak 另需 fork 新增一处只读出口。
>
> ⚠️ **后续修正（同日，两轮）**
>
> - **第一轮**：本文修的只是「**画不出来**」。画出来之后发现合成路径**换不来输入**——  
>   RDP 区域完全点不动。证据见 **§9**，两条路径的**真机双向验证**（命中测试 + 真实点击）见 **§10**。
> - **第二轮**：当时把闸门翻回「默认关闭合成」是把结论下得太死——**常驻 cloak** 确实与  
>   可交互不可兼得，但「浮层盖住远端桌面」这个目标本身做得到。改成**按需 cloak**：只有  
>   GPUI 真的画了浮层内容时才 cloak，浮层一消失立刻取消。合成因此**默认启用**，逃生门是  
>   `NAVOP_RDP_DISABLE_COMPOSITION=1`。见 **§11**；真机三项判据（`DWMWA_CLOAKED` /  
>   `WindowFromPoint` / 真实点击远端开始菜单）**全部通过**，见 **§11.6**。
>
> 补丁：`.workbuddy/tmp/rdp-cloak-fix/*.patch`（`git apply` 即用）
>
> **修正上一份文档**：`docs/gpui-pre-airspace-surface-forwarding.md` §6 的「视觉判据」当时  
> **并没有真正达成**——那批判据全部是日志级的（`cloaked=true` 保持、`0x80004002` 归零、  
> 不退回 plain child），而屏幕区域当时就是空的。本文是那个缺陷的续修。

---

## 1. 症状与唯一可信判据

| 项                      | 值                                                             |
| ---------------------- | ------------------------------------------------------------- |
| 合成状态                   | `stage="composition_attached"` ✔                              |
| cloak 状态               | `cloaked_state=1` ✔                                           |
| bounds / visibility 同步 | 全部成功 ✔                                                        |
| WARN / ERROR           | **0 条**                                                       |
| 诊断 verdict             | `"compositor / Z-order / clipping coverage remains possible"` |
| **屏幕像素**               | **整片 `#171717`（GPUI 窗口背景色）** ✘                                |

`IDCompositionDevice::CreateSurfaceFromHwnd` / `visual.SetContent` / `Commit` 的 HRESULT  
**全部是 `S_OK`**，而那个 surface 里**一个像素都没有**。所以这个故障在日志里是完全不可见的，  
只有取屏幕像素才能看见——**本缺陷全程靠像素判据定位，任何一个日志判据都会给出「已修好」的假结论**。

---

## 2. 为什么必须取像素

`CreateSurfaceFromHwnd` 包装的是**窗口的重定向位图（redirection bitmap）**。合成树只负责  
「把这个位图放到哪一层」；位图本身是空的，visual 就渲染不出任何东西，而所有 DirectComposition  
调用依旧返回 `S_OK`。这正是本缺陷能一路穿过全部日志判据的原因。

三组像素判据：

```bash
# 1) 被合成的引导窗口（GPUI）——取屏幕像素，PrintWindow 看不到 DComp 输出
<py> .workbuddy/tmp/sample_pixels.py "GPUI Native RDP Smoke" --grid 5
# 2) 独立 C++ 探针，完全脱离 GPUI 复现同一序列
bash .workbuddy/tmp/dcomp-probe/build.sh && ./.workbuddy/tmp/dcomp-probe/probe.exe <flags>
```

探针配色是刻意选的，便于区分「有没有像素」：

| 颜色                       | 含义                        |
| ------------------------ | ------------------------- |
| `(23,23,23)` = `#171717` | **父窗口背景** —— 即「什么都没有」     |
| `(0,0,0)`                | 容器自身的 `SS_BLACKRECT` 画进去了 |
| `(0,0,255)`              | 实验性的「cloak 之后再涂一次」的蓝      |
| `(255,0,255)`            | GDI 子窗口的原始颜色（用来识别「画面被冻住」） |

---

## 3. 把变量收敛到一条：探针实验矩阵

统一的初始条件与实际产品路径一致：容器（`Static` + `SS_BLACKRECT`）**以 1×1 创建**、  
加 `WS_EX_LAYERED` + `LWA_ALPHA(255)`、`CreateSurfaceFromHwnd(容器)` + `SetContent` +  
`AddVisual` + `Commit`，然后 resize 到 800×500。

| #  | 序列                                                | `inside_B`   | 结论                                  |
| -- | ------------------------------------------------- | ------------ | ----------------------------------- |
| T0 | 1×1 → resize →（**不 cloak**）→ 涂蓝                   | `(0,0,255)`  | 对照：不 cloak 路径正常                     |
| T1 | 1×1 → resize → 绘制 → cloak → **再涂蓝**               | `(0,0,255)`  | **cloak 之后实时绘制照样落地**，位图没有冻结         |
| T2 | 1×1 → **cloak** → resize → 涂蓝                     | `(23,23,23)` | 空的                                  |
| T3 | 1×1 → **cloak** → resize → `RedrawWindow`         | `(23,23,23)` | **重绘救不回来**                          |
| T4 | 1×1 → resize → 绘制 → cloak → **再 resize** → 涂蓝     | `(0,0,255)`  | 「cloak 状态下 resize」本身**不是**元凶        |
| T5 | 1×1 → cloak → resize → **uncloak + recloak** → 涂蓝 | `(0,0,255)`  | 切换 cloak 状态能**复活** surface          |
| T6 | 1×1 → cloak → resize → **uncloak** → 涂蓝           | `(0,0,255)`  | 同上，单次切换即可                           |
| T7 | 1×1 → resize → 绘制 → cloak → **嵌套 GDI 子窗口**涂蓝      | `(0,0,255)`  | cloak 后**子窗口**绘制也实时落地（RDP 正是画在子窗口里） |
| T8 | 1×1 → resize → 显示 + 绘制一次 → **立即** cloak（无停留）      | `(0,0,0)`    | **修复方案**，且不依赖任何 sleep               |
| T9 | 同 T8，但带嵌套 GDI 子窗口                                 | `(0,0,255)`  | 修复方案在真实结构上成立                        |

### 3.1 矩阵读出来的三条规则

1. **cloak 不冻结位图。** T1/T7 证明 cloak 之后容器与**嵌套子窗口**的绘制都实时进入合成  
   ——所以「合成方案会让 RDP 画面冻住」这个担心不成立。
2. **致命条件是「在窗口被显示/绘制过之前就 cloak」。** T2/T3 里无论之后 resize、  
   `RedrawWindow` 都救不回来；而 T4 说明「已正常 cloak 的窗口之后随便 resize」完全没事。
3. **任何一次 cloak 状态切换都会让 surface 复活**（T5/T6），说明 DWM 在 cloak 那一刻  
   「定格」了位图的有效范围，而状态变化会重新评估它。

> 附带否掉的旧假设：`DWMWA_CLOAK` 的像素语义**不是**「此后不再补新暴露区域」。  
> 早先 A/B/C 三组（`A 1×1+cloak-early → (23,23,23)`、`B cloak-late → (0,0,0)`、  
> `C cloak-late+真实内容 → (0,153,229)`）只说明「迟 cloak 有效」，  
> 真正的原因是**过早**，不是**区域**。T2/T3 是区分这两者的关键。

---

## 4. 根因

```
NativeOverlay::create()            容器以 1x1 创建、隐藏
  ↓
compose_native_window()            set_layered(true) → attach() → set_cloaked(true)   ← ★ 在下
  ↓
set_bounds() / synchronize()       SetWindowPos 到 1600x900（第一次真实尺寸）+ 显示
  ↓
RDP 开始绘制
```

**`DWMWA_CLOAK` 落在了窗口第一次获得真实尺寸并被绘制之前。** 此时重定向位图里没有任何  
属于最终尺寸的像素，DWM 又已停止为它产出内容，于是这个 surface 永久为空：  
合成 visual 贡献零像素 → 屏幕显示 GPUI 背景色 → 日志全绿。

修复只需把 cloak 往后搬一个位置：**等窗口已经在最终尺寸上显示并同步绘制过一次之后再 cloak。**

---

## 5. 修复

### 5.1 新时序

```
compose_native_window()            set_layered(true) → attach() → 记下「还欠一次 cloak」
  ↓
set_bounds() / synchronize()       定位 + 显示（第一次真实尺寸）
  ↓
sync_composition_bounds(Some)      把 bounds 镜像进合成树
  ↓
apply_deferred_cloak()             ★ 检查真的在屏幕上 → RedrawWindow 同步绘制 → set_cloaked(true)
```

`apply_deferred_cloak` 是唯一的 cloak 入口，它做三件事：

1. **`is_actually_visible()` 为假就不 cloak**，把欠账留着——下一次 bounds 或 visibility 同步重试。  
   这一步让「paint 没发生」不可能被误当成「paint 发生了」。
2. **`overlay.redraw()`**：`RedrawWindow(RDW_INVALIDATE|RDW_ERASE|RDW_ALLCHILDREN|RDW_UPDATENOW)`，  
   同步把整个客户区（含子窗口）画一遍。同步是刻意的——T8 证明**不需要任何停留时间**。
3. **`set_cloaked(true)`**，然后清掉欠账标记。

### 5.2 navop 侧改动（5 个文件 + 闸门）

| 文件                                         | 改动                                                                                                                                                                                          |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `view/windows_native_overlay/ffi.rs`       | 声明 `RedrawWindow`                                                                                                                                                                           |
| `view/windows_native_overlay/window.rs`    | 新增 `redraw_overlay_window()`（附根因说明）                                                                                                                                                         |
| `view/windows_native_overlay/lifecycle.rs` | 新增 `WindowsNativeOverlay::redraw()`，**窗口不在屏幕上时直接返回**（不假装画过）                                                                                                                                 |
| `view/windows_native_composition.rs`       | 新增 `cloak_pending` 状态 + `cloak_pending()` / `mark_cloaked()`；`attach()` 置位                                                                                                                  |
| `view/windows_native.rs`                   | `compose_native_window` **不再 cloak**；新增 `apply_deferred_cloak()`；`sync_composition_bounds`（`Some` 分支）与 `sync_composition_visibility`（`true` 分支）各调用一次；另加 `NAVOP_RDP_DISABLE_COMPOSITION` 逃生门 |
| `view/render_contract_tests.rs`            | 新增契约测试 `windows_native_overlay_is_cloaked_only_after_it_is_positioned_and_painted`                                                                                                          |
| `main/src/main.rs`                         | 闸门翻转：默认**不再**关 DirectComposition，改为由 `NAVOP_RDP_DISABLE_COMPOSITION` 把关；同名契约测试 `windows_native_rdp_keeps_direct_composition_unless_the_disable_switch_is_set` 同步改写                          |

> `main/src/main.rs` 的翻转就是上一份文档 §5.1 早已写下的形态。  
> 中间它曾退化成 `NAVOP_RDP_ENABLE_COMPOSITION`（opt-in），原因是合成路径当时交不出画面；  
> 根因修掉之后又回到文档描述的形态，两份文档现在一致。

关键片段（`view/windows_native.rs`）：

```rust
fn apply_deferred_cloak(&mut self, stage: &'static str) -> bool {
    let owed = self
        .composition
        .as_ref()
        .is_some_and(|composition| composition.cloak_pending());
    if !owed || !self.overlay.is_actually_visible() {
        return true;                       // 还没上屏，欠着，下次同步再试
    }
    if let Err(error) = self.overlay.redraw() {         // 同步绘制
        tracing::warn!(?error, stage, ...);
        return false;                                   // → fallback_to_plain_child
    }
    if let Err(error) = self.overlay.set_cloaked(true) {
        tracing::warn!(?error, stage, ...);
        return false;
    }
    if let Some(composition) = self.composition.as_mut() {
        composition.mark_cloaked();
    }
    true
}
```

失败语义保持不变：绘制或 cloak 失败都返回 `false`，由既有调用点  
`fallback_to_plain_child(...)` 退回普通子窗口（而不是留下一个被 cloak 却推不动的窗口）。

### 5.3 smoke 侧改动（`tools/gpui-rdp-smoke`）

| 文件                                                                             | 改动                                                                                                                                                                                                                     |
| ------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/main.rs`                                                                  | **顺带修掉一个把测试变成假的方法论 bug**：原来**无条件**硬编码 `GPUI_DISABLE_DIRECT_COMPOSITION=1`，导致 `SMOKE_RDP_COMPOSE=early/late` 永远拿不到合成 surface —— 所有 smoke 合成实验其实是无效的。改为按 mode 决定，并打印 `presentation: compose_mode=… direct_composition=…` |
| `src/native_overlay_ffi.rs` / `native_overlay/window.rs` / `native_overlay.rs` | 同上三件套：`RedrawWindow` 声明、`redraw_overlay_window()`、`NativeOverlay::redraw()`                                                                                                                                            |
| `src/windows_app/session.rs`                                                   | 与 navop 同构的 `compose()` / `finish_cloak()` / `sync_composition()`；另加 `SMOKE_RDP_CLOAK_IMMEDIATE=1` 反向开关（见 §6.1）                                                                                                        |
| `src/windows_app/mod.rs` / `view/presentation.rs`                              | 模块与调用点接线                                                                                                                                                                                                               |

---

## 6. 验证

### 6.1 因果对照（同一份构建、同一个会话，只切换 cloak 时序）

`SMOKE_RDP_COMPOSE=early` 两种时序各跑一次，取屏幕像素：

| 变体                                 | cloak 落点       | `cloaked_state` | 像素片                                                 |
| ---------------------------------- | -------------- | --------------- | --------------------------------------------------- |
| `SMOKE_RDP_CLOAK_IMMEDIATE=1`（旧行为） | `attach()` 处   | **1**           | **整片 `#111827`**                                    |
| 默认（新行为）                            | 首次 bounds 同步之后 | **1**           | `#A59976` `#F8D2A7` `#DDCCBB` `#10504E` `#C6C6B1` … |

两者**都已 cloak**，唯一差别是时序 → 空屏 vs 真实桌面。这排除了「像素来自别处」的可能。  
日志侧对应行：

```
# 旧行为
composition: stage=early  … layered_applied=true cloak=immediate
composition: stage=compose_immediate … cloaked_state=1
composition: verdict="compositor / Z-order / clipping coverage remains possible"   ← 日志看起来完全正常

# 新行为
composition: stage=early  … layered_applied=true cloak=deferred
composition: stage=composition_sync … layered_applied=true cloaked_state=1
composition: verdict="compositor / Z-order / clipping coverage remains possible"
display: stage=success reason=login_complete desktop=1920x1030
```

### 6.2 编译

```bash
# crate 级：带 feature 检查（11m35s ✔）
python .workbuddy/build/cargo-msvc.py check -p remote_desktop_view --features windows-native-rdp

# 全量：必须用降内存配置，否则 main 的 codegen 峰值会撞提交上限
python .workbuddy/build/cargo-msvc.py build -p main --bin navop \
    --config .workbuddy/tmp/navop-nodbg.toml
```

> 本机编译的两个坑（都不是本次改动引入的）：
>
> 1. `main` 的 codegen 峰值会撞提交上限 → 失败形态是  
>    `rustc-LLVM ERROR: out of memory` / `Allocation failed`，  
>    错误进程退出码被伪装成 `0xc0000409 (STATUS_STACK_BUFFER_OVERRUN)`。**降内存重跑即可，不要 clean。**
> 2. 上一次内存耗尽时正在写 `libdb_view-*.rlib`（1.03 GiB），文件被写坏，  
>    之后每次都报 `E0786 ... failed to mmap file ... os error 1455`。  
>    注意这个错误**看起来**像「页面文件太小」的内存问题，实际是**产物已损坏** ——  
>    重跑无效，必须 `clean -p db_view`（只清这一个 crate，别全量 clean）。

### 6.3 契约测试

`cargo test -p remote_desktop_view windows_native_overlay_is_cloaked_only_after_it_is_positioned_and_painted`  
（源码文本断言，与既有 `windows_native_overlay_is_layered_only_while_it_is_composed` 同风格）：

- `compose_native_window` **不得**出现 `set_cloaked(true)`；
- `attach()` 必须置 `cloak_pending = true`，`new()` 不得置位；
- `apply_deferred_cloak` 内部顺序必须是 `is_actually_visible()` → `redraw()` → `set_cloaked(true)`；
- `sync_composition_bounds` 与 `sync_composition_visibility` 都必须触发它；
- 重绘助手必须用 `RDW_UPDATENOW` + `RDW_ALLCHILDREN`；
- `WindowsNativeOverlay::redraw` 必须对「不在屏幕上」的窗口直接返回。

实测：

```
test view::render_contract_tests::windows_native_overlay_is_layered_only_while_it_is_composed ... ok
test view::render_contract_tests::windows_native_overlay_is_cloaked_only_after_it_is_positioned_and_painted ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 282 filtered out
```

### 6.4 navop 产品侧真机验证（2026-10-10）


`main/src/main.rs` 翻转闸门后，真机（`<RDP 主机>`，`backend_preference: windows_native`）  
跑默认路径，日志给出的正是新时序：

```
04:47:27.731 INFO ... lifecycle: updated Windows native RDP overlay layered state stage="overlay_layered" layered=true
04:47:27.732 INFO ... windows_native_composition: composed the Windows native RDP overlay into the GPUI visual tree stage="composition_attached"
04:47:34.144 INFO ... diagnostics: updated Windows native RDP overlay visibility stage="show" requested_visible=true actual_visible=true
04:47:34.150 INFO ... lifecycle: updated Windows native RDP overlay cloak state stage="overlay_cloak" cloaked=true
04:47:34.151 INFO ... windows_native: cloaked the composed Windows native RDP overlay after positioning and painting it stage="show"
04:47:34.171 INFO ... windows_native_display_integration: display: stage=success reason=LoginComplete width=1568 height=877
```

`attach`（27.732）与 `cloak`（34.150）之间隔了 **6.4 秒**——cloak 不再落在 1×1 阶段，  
而是落在「已定位 + `actual_visible=true`」之后。这正是修复要保证的唯一不变式。

**像素判据（本次改用 `PrintWindow`，因为验证时机器的屏幕处于锁屏状态）：**

`BitBlt` 在锁屏下只能读到安全桌面（`LogonUI.exe` + `LockApp.exe` 在跑，  
前台是 LockApp 的 `Windows.UI.Core.CoreWindow`），所以这一次不能取屏幕像素。  
替代判据是 `PrintWindow(PW_RENDERFULLCONTENT)`——它让窗口自己往给定 DC 重画、不经过  
DWM 呈现，因此锁屏下依然可用；而 `PW_RENDERFULLCONTENT` 对 layered 窗口读的正是它的  
**重定向位图**，也就是 `CreateSurfaceFromHwnd` 包装的那个对象。

| hwnd       | 窗口                                   | `distinct_colors` | 内容区采样                                                                   |
| ---------- | ------------------------------------ | ----------------- | ----------------------------------------------------------------------- |
| `0x25060C` | overlay（`Static`，**合成 surface 的来源**） | **86,933**        | `#7A8267` `#EFCBA2` `#EEDCC8` `#1D5753` `#70826E` `#023137` `#175255` … |
| `0x330626` | RDP 控件（`ATL:...`，1568×877）           | 86,903            | 同上                                                                      |

8.7 万种颜色的**真实远端桌面**（桌面图标、壁纸、任务栏、以及一个因 navop 动态改分辨率  
而弹在远端的「检测到屏幕分辨率发生变化」提示框）。修复前这里是一个像素都没有的  
`#171717`，所以「surface 有内容」这件事本身就是因果判据。

窗口树同时确认合成走的是真实结构：

```
0x60378  Zed::Window            ex=0x00240100            rect=(87,58,1586x936)
  0x25060c Static 'Navop RDP Overlay' ex=0x00080004       rect=(96,108,1568x877)   ← WS_EX_LAYERED
    0x32069c Navop.WindowsRdpHost.Container
      0x330626 ATL:... → UIMainClass → UIContainerClass
        0x6060a IHWindowClass 'Input Capture Window'
        0x1f0736 OPContainerClass 'Output Painter Window'
          0xb0620 OPWindowClass 'Output Painter DX Child'
```

**仍未做**：屏幕级像素判据（解锁后应复核），以及 §8 的 airspace 复核。

---

## 7. 落地步骤

```bash
cd <your-navop-fork>
git apply .workbuddy/tmp/rdp-cloak-fix/navop-remote-desktop-view.patch   # 核心修复
git apply .workbuddy/tmp/rdp-cloak-fix/navop-main.patch                  # 默认启用合成 + 契约测试
git apply .workbuddy/tmp/rdp-cloak-fix/navop-gpui-rdp-smoke.patch        # 可选，仅测试工具
```

| 补丁                                          | 内容                                                                                                            |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `navop-remote-desktop-view.patch`           | **核心修复**：`crates/remote_desktop_view` 全部改动，相对 `7d66a491e`（PR #358 之后）                                         |
| `navop-main.patch`                          | `main/src/main.rs`：闸门翻转 + 契约测试改名改写；**另含一处与本修复无关的既有未提交改动**（`Navop run loop returned` 退出里程碑日志，见上一份文档 §5.3），按需取舍 |
| `navop-gpui-rdp-smoke.patch`                | 测试工具（smoke）改动，含 `SMOKE_RDP_CLOAK_IMMEDIATE=1` 反向开关                                                            |
| `gpui-pre-fork.patch`                       | GPUI fork 的 A/B 两处修复（与上一份文档相同，**本次未改动 GPUI**）                                                                 |
| `docs/windows-native-rdp-cloak-ordering.md` | 本文                                                                                                            |

**GPUI fork 不需要为本次修复再加东西。**

> `Cargo.lock` 与 `~/.cargo/config.toml` 里的 `[patch.crates-io] rustix` 是**本机调试用的  
> 临时改动（不进交付）**：本机任何 Rust 程序的 `stdin(Stdio::piped())` 都以 `os error 231`  
> 失败，rustix 的 build script 因此探测不到 rustc。验证结束后必须还原  
> （`~/.cargo/config.toml.bak-rdp-test`、`git checkout -- Cargo.lock`）。

### 本地验证用的临时手法（收尾要还原）

本机是**就地修改 cargo 的 git 检出**（不碰 `Cargo.toml` / `Cargo.lock`）：

| 项    | 值                                                                                                                 |
| ---- | ----------------------------------------------------------------------------------------------------------------- |
| 检出   | `~/.cargo/git/checkouts/gpui-pre-eda588a4da6ba6c6/9e410c8`                                                        |
| 当前状态 | `crates/gpui/src/window.rs`（4 行）、`crates/gpui_windows/src/directx_renderer.rs`（A/B 修复），**临时 `GPUI_DIAG` 打印已全部删除** |

> **踩坑（本次又踩了一次）：cargo 不认 git 检出里的文件改动。**  
> 改完 `directx_renderer.rs` 后 `cargo build` 只花 4.18s 且报 `Fresh gpui-pre-windows`，  
> 诊断代码根本没编进去。必须显式：
>
> ```bash
> python .workbuddy/build/cargo-msvc.py clean -p gpui-pre-windows     # Removed 22 files, 103.4MiB
> ```
>
> 判据：`cargo build -v` 出现 `Compiling gpui-pre-windows`，以及  
> `target/debug/deps/libgpui_windows-*.rlib` 的 mtime 被刷新。

---

## 8. 尚未验证 / 风险

1. **airspace（issue #310）需要解锁后补一次屏幕级验证。** 本次已确认「合成 surface 交得出  
   画面」（§6.4 的 8.7 万色远端桌面），机制上「GPUI 浮层盖在远端桌面之上」依旧成立  
   （native visual 排在 base 之上、overlay 之下，`set_surface_order` 顺序 `base → native → overlay`）；  
   但「标签右键菜单 / 侧栏能盖住远端桌面」这个**最终视觉判据**需要屏幕解锁才能取到  
   （锁屏下 BitBlt 只能读到安全桌面，而 GPUI 的浮层又不在子窗口里、`PrintWindow` 取不到）。  
   待办：解锁后 `python .workbuddy/tmp/rdp-test/gui.py seq <pid> <prefix> prclick:<x>,<y>` 打开标签菜单，  
   再用 `sample_pixels.py "Navop"` 取屏幕像素。
2. **合成路径现在是默认行为**，`NAVOP_RDP_DISABLE_COMPOSITION=1` 是逃生门（会同时关掉  
   GPUI 的 DirectComposition，否则退回经典路径后的普通子窗口会被 GPUI 合成 visual 盖住）。  
   如果真机上出现任何合成相关回归，先设这个变量回退再排查。
3. **窗口在会话中途 resize**：T4 证明已正常 cloak 的窗口之后 resize 仍然实时出画，  
   所以不需要 re-cloak。若真机上出现 resize 后空白，机制上的解法是  
   「uncloak → resize → 重新 redraw → recloak」（T5 已证明可行）。
4. **`update_session_display_settings` 偶发失败**：真机日志里出现过  
   `Windows RDP host result 4 with HRESULT 0x8000FFFF` → `retry_scheduled` → `success reason=Retry`  
   （自愈，与本次修复无关）。单独记录，不在本次范围内。
5. **`cargo test -p main` 未在本机跑**：本机 5.9GB 物理内存 + `main` 的 codegen 峰值，  
   跑 `main` 的测试二进制风险高、代价约 20–57 分钟。`main.rs` 的契约测试是纯源码文本断言，  
   已用等价脚本（`production_source()` 同一段切片 + 同样 6 条断言）逐条复核通过；  
   仓库 CI 门禁 `cargo test --all` 会真正执行它。

---

## 9. 后续修正：合成路径换不来输入（同日发现）

§5 的修复让合成 surface 交得出画面（§6.4 已验证）。但**画出来之后 RDP 区域完全点不动**，  
这比 airspace 严重得多，所以闸门最终回到**默认关闭合成**。

### 9.1 判据：命中测试根本没落到 RDP 上

合成路径自带的诊断（`NAVOP_REMOTE_DESKTOP_DIAGNOSTICS=1`）会在 RDP 区域中心做全局命中测试，  
日志直接给出答案：

```
stage="composition_verdict" center_x=904 center_y=505
global_hwnd=1507546 global_class=Zed::Window       ← 命中的是 GPUI 主窗口
deepest_hwnd=1507546 deepest_class=Zed::Window
global_in_overlay=false
```

`DWMWA_CLOAK` 让窗口退出系统命中测试：窗口本身仍「可见」（`IsWindowVisible=true`、  
心跳全绿、重定向位图实时更新），但 `WindowFromPoint` 不再返回它。于是所有点击都落到  
GPUI 主窗口，而 GPUI 在 RDP 区域没有任何可交互元素。

### 9.2 RDP 的输入窗口是哪一个

从 overlay 逐层下钻（`ChildWindowFromPointEx`；cloak 不影响几何下钻）：

```
overlay (Static)
 └ Navop.WindowsRdpHost.Container
   └ ATL:...（mstscax 控件）
     └ UIMainClass → UIContainerClass
       ├ IHWindowClass  "Input Capture Window"   ← 输入归它，ex_style=0xA4（含 WS_EX_TRANSPARENT）
       └ OPContainerClass → OPWindowClass "Output Painter DX Child Window"
```

### 9.3 转发矩阵：只有 `WM_MOUSEMOVE` 生效

把鼠标消息分别 `PostMessage` 与 `SendMessage` 投递给 overlay 子树的 6 个候选窗口  
（Overlay / HostContainer / ATL 控件 / UIMainClass / UIContainerClass / IHWindowClass），  
共 12 组；判据是 overlay 重定向位图的像素变化量：

| 动作                               | 结果                                  |
| -------------------------------- | ----------------------------------- |
| `WM_MOUSEMOVE` 给 `IHWindowClass` | **生效**：远端光标移动（1833 px 变化，位于光标处）     |
| `WM_RBUTTONDOWN/UP`（12 组全部）      | **无效**：0 px                         |
| `WM_LBUTTONDOWN/UP`（点远端任务栏开始按钮）  | **无效**：0 px                         |
| 先把真实光标 `SetCursorPos` 到目标点再发     | 同上，仍无效                              |
| `mouse_event` 注入的真实点击            | 无效（注入输入带 `LLMHF_INJECTED`，被 RDP 过滤） |

结论：`IHWindowClass` 的**按钮输入走 RawInput**，依赖窗口处于系统输入路径上；窗口一旦  
cloak 就再也收不到，合成消息也救不回来。**cloak 与可交互在当前 mstsc 上不可兼得。**

> 排查提示：这轮实验里有两批「全 0」是**假阴性**——一次是 navop 窗口被最小化  
> （最小化窗口不处理输入），一次是导航中途进程被关。判据脚本必须先 `ShowWindow(SW_RESTORE)`  
> 并确认 overlay 的屏幕矩形非 `-25600`。

### 9.4 最终闸门

```rust
// main/src/main.rs
if remote_desktop::windows_native_rdp_compiled()
    && std::env::var_os("NAVOP_RDP_ENABLE_COMPOSITION").is_none()
{
    unsafe { std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1"); }
}
```

- **默认**：经典 HWND 子窗口——远端桌面可交互（代价：GPUI 浮层被子窗口盖住，即 #310）。
- **opt-in**：`NAVOP_RDP_ENABLE_COMPOSITION=1`——浮层能盖住远端桌面，但 RDP 区域点不动。
- `remote_desktop_view` 里另有一道 `NAVOP_RDP_DISABLE_COMPOSITION` 强制关闭 view 层合成，  
  供 smoke 工具与排查使用，保留不变。

### 9.5 后续方向（若要两者兼得）

1. **「有浮层时才 cloak」**：无浮层时 uncloak——overlay 的内容与 native visual 完全重合，  
   视觉无差异，而输入恢复正常；GPUI 的 deferred 浮层出现时再 cloak（此时用户要点的是浮层，  
   浮层由 GPUI 接收输入，RDP 短暂不可点无妨）。触发点可由 GPUI 侧提供（`set_surface_order`  
   里已知 overlay 层的内容），也可先由 navop 在打开自家菜单时切换。
2. **浮层改为独立顶层窗口**：GPUI 已有 `WindowKind::PopUp` / `AnchoredPopup`，navop 也已有  
   `crates/core/src/popup_window.rs`。若标签右键菜单走独立窗口，跨窗口 z 序天然盖住原生  
   子窗口，就不再需要合成路径。

## 10. 最终闸门的两条路径：真机双向验证（2026-10-10，同日）

§9 的结论（合成路径换不来输入）当时是从「合成消息转发矩阵全 0」+ 内部诊断  
`composition_verdict` 推出来的。这里补一组**从进程外测得、可重复、两条路径互为对照**的证据，  
并且用**真实鼠标点击**（不是 `PostMessage`）验证默认路径确实可交互。

构建：`target/debug/navop.exe`（`main/src/main.rs` 已翻回默认关闭合成），数据目录用用户真实配置  
`%APPDATA%\Navop`，连接 `我的win`（`windows_native`，<RDP 主机>:3389）。

### 10.1 判据一：命中测试落在谁身上

`WindowFromPoint` **会跳过 cloaked 窗口**，所以它返回的是「系统实际会把这一点的输入投递给谁」，  
正好是本问题的判据。两条路径各取 RDP 区域内三个点：

| 路径                                          | overlay `Static` 的 `ex_style`    | RDP 区域 `WindowFromPoint`               |
| ------------------------------------------- | -------------------------------- | -------------------------------------- |
| **默认**（不带任何变量）                              | `0x4`（**无** `WS_EX_LAYERED`）     | `IHWindowClass` "Input Capture Window" |
| **opt-in** `NAVOP_RDP_ENABLE_COMPOSITION=1` | `0x80004`（**有** `WS_EX_LAYERED`） | `Zed::Window`（GPUI 主窗口）                |

窗口树（两条路径同构，差别只在 overlay 的 `ex_style`）：

```
Zed::Window
└─ Static                                  'Navop RDP Overlay'        ex=0x4 / ex=0x80004
   └─ Navop.WindowsRdpHost.Container
      └─ ATL:00007FFB...
         └─ UIMainClass
            └─ UIContainerClass                                        ex=0x100004
               ├─ IHWindowClass              'Input Capture Window'    ex=0xA4
               └─ OPContainerClass           'Output Painter Window'
                  └─ OPWindowClass           'Output Painter Child Window'
                     └─ OPWindowClass        'Output Painter DX Child Window'
```

也就是说：**默认路径下 RDP 区域的每一个点都由 `IHWindowClass` 接管**，opt-in 路径下整个 RDP  
区域都退回 GPUI 主窗口。这与 §9.1 的 `global_hwnd=Zed::Window` 完全一致，只是换成了外部视角。

### 10.2 判据二：真实点击真的被远端接受

只测命中测试还不够——它只说明「消息会投给谁」，不说明 mstsc 会不会处理。所以在默认路径下做了两次  
**真实**点击（`SendInput`，不是 `PostMessage`），用窗口位图的像素差当判据：

| 操作              | 结果                                 |
| --------------- | ---------------------------------- |
| 点远端弹出对话框的「取消」按钮 | 对话框消失，**237 935 px 变化（16.0 %）**    |
| 点远端任务栏「开始」      | 远端开始菜单弹出，**679 911 px 变化（45.8 %）** |
| 按 `Esc` 收起开始菜单  | 菜单关闭                               |

注意 §9.3 的转发矩阵当时全部是「零像素变化」，唯一差别就是用了 `PostMessage`/`SendMessage` 而非  
真实输入——这与「按钮输入走 RawInput、合成消息无法驱动」的结论互相印证：**验证 RDP 输入必须用  
真实输入，合成消息永远测不出结论。**

### 10.3 默认路径的启动日志（可作为回归基线）

```
INFO  gpui_windows::directx_renderer: Direct Composition is disabled.
WARN  remote_desktop_view::view::windows_native: GPUI window cannot compose the Windows
      native RDP child window; keeping it as a plain child window
      error=DirectComposition is disabled
INFO  remote_desktop_view::view::windows_native_overlay::window: configured Windows native
      RDP owner clipping stage="owner_style" ... clip_children=true changed=true
INFO  remote_desktop_view::view: native RDP: stage=connection-policy desktop_width=1568
      desktop_height=877 ...
INFO  remote_desktop_view::view::windows_native_display_integration: display: stage=success
      reason=LoginComplete generation=1 width=1568 height=877 desktop_scale=125
```


两条特征线：**`Direct Composition is disabled`**（闸门生效）与  
**`keeping it as a plain child window`**（回退成功）。整份日志里 **`cloak` 出现 0 次**——  
这一条最省事：只要日志里没有 `overlay_cloak`，就说明没走合成路径。

opt-in 路径的对照日志则是 `composition_attached` → `overlay_cloak ... cloaked=true` → `display:
stage=success`，**画面照样正常**（§5 的 cloak 时序修复仍然有效），只是输入不通。

### 10.4 两个反直觉的点

**① 窗口可以比屏幕大。** 这台机器屏幕是 1536×864，而 navop 窗口是 1586×936 且位于 (111, 17)，  
右侧与下侧各有约 160 / 90 px 落在屏幕外。`SetCursorPos` 对超出屏幕的坐标会**静默钳制**，  
点击因此落到别处——写自动化点击时必须先把屏幕坐标钳到可视区，再换算窗口相对坐标。

**② 截图里量出来的坐标不能直接当成窗口相对坐标。** 一张 1586×936 的窗口位图被缩放显示后，  
按「肉眼看到的比例」读出的坐标会整体偏小约 **1.43×**。第一轮复验就是被这个坑掉的：  
侧栏点成了 `(35,621)`（实际约 `(51,908)`）、卡片点成了 `(250,167)`（实际 `(400,250)`），  
于是在真实输入明明可用的情况下误判成「navop 完全不响应任何输入」。  
**对策**见 §10.5 的 `crop_grid.py`——先裁剪再按原图坐标网格读数。

### 10.5 复用工具

| 工具                                                                              | 用途                                                                        |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `.workbuddy/tmp/crop_grid.py <in.png> <x0> <y0> <w> <h> <out> [step]`           | 裁剪 + 打上原图坐标网格，消除「读图时图片被缩放」带来的坐标误差                                         |
| `.workbuddy/tmp/click_diag.py`                                                  | 完整性级别 / 输入桌面名 / `IsHungAppWindow` / `WM_NCHITTEST` / `WindowFromPoint` 一览 |
| `.workbuddy/tmp/input_sanity.py`                                                | 自建 Win32 窗口 + 真实注入，判定「本会话的合成输入是否有效」                                       |
| `.workbuddy/tmp/rdp-test/gui.py seq <pid> <prefix> dclick:x,y wait:n shot:name` | 窗口相对坐标驱动（`dclick`/`click` 走 `SendInput`，`pdclick` 走 `PostMessage`）        |

---

## 11. 按需 cloak：浮层与可交互兼得（2026-10-10，同日第二轮）

§9 的「cloak 与可交互不可兼得」只对**常驻 cloak** 成立。§9.5 列的两条后续方向里，  
「有浮层时才 cloak」这一条已经实现，而且比预想的干净——**不需要动两态以外的任何东西**。

### 11.1 判据：怎么知道「现在有浮层」

navop 的菜单浮层全部出自 `gpui_component::menu` 的 `PopupMenu` / `ContextMenuExt` /  
`DropdownMenu`，而它们一律用 `gpui::deferred(...)` 渲染：

| 位置                                              | 证据          |
| ----------------------------------------------- | ----------- |
| `crates/component/src/menu/popup_menu.rs:1351`  | `deferred(` |
| `crates/component/src/menu/context_menu.rs:183` | `deferred(` |
| `crates/component/src/menu/app_menu_bar.rs:285` | `deferred(` |

所以 **GPUI 的 `Frame::deferred_draws` 非空 ⟺ 这一帧画了浮层**。

它必须是**状态**而不是事件：GPUI 按需渲染，浮层静止时根本不重绘，所以「最近一帧画了什么」  
并让它保持不变，恰好就是要的语义——打开菜单那一帧置真、关闭那一帧置假、空闲帧不变。

`deferred_draws` 是 `pub(crate)`，navop 够不到，因此 fork 加一个只读出口：

```rust
// crates/gpui/src/window.rs
pub fn has_deferred_content(&self) -> bool {
    self.deferred_content_present
}
```

赋值点在 `draw` 里 `prepaint_deferred_draws` **之后**（它只把 `element` `take()` 出来、  
不删条目，所以长度仍反映本帧的浮层数量；`prompt` 由窗口自己画，单独计）：

```rust
self.deferred_content_present =
    !self.next_frame.deferred_draws.is_empty() || self.prompt.is_some();
```

### 11.2 两态与切换

| 态           | overlay cloak | 屏幕输出                            | RDP 区域输入         |
| ----------- | ------------- | ------------------------------- | ---------------- |
| **默认（无浮层）** | `false`       | HWND（在客户端内容之上）                  | ✅ 命中测试正常         |
| **有浮层**     | `true`        | DComp visual（在 GPUI overlay 之下） | 由浮层接管（本来就是该有的行为） |

合成 visual 一直挂着、不隐藏：未 cloak 时它被 HWND 盖住，无害。所以切换只动 **cloak 一个位**，  
`sync_bounds` / `sync_visible` 的既有行为完全不用改。

驱动在 `view/render.rs` 的 `render` 里，每帧一次，读的是**上一帧**的值（`render` 早于本帧的  
`prepaint_deferred_draws`，差一帧无影响）：

```rust
self.sync_native_overlay_cloak(window);
// → native.set_overlay_cloak(window.has_deferred_content())
```

### 11.3 改动面

| 文件                                   | 改动                                                                                                 |
| ------------------------------------ | -------------------------------------------------------------------------------------------------- |
| fork `crates/gpui/src/window.rs`     | `deferred_content_present` 字段 + `has_deferred_content()` + `draw` 里赋值                              |
| `view/windows_native.rs`             | `WindowsNativeAdapter` 加 `overlay_cloaked` / `overlay_cloak_painted` 字段与 `set_overlay_cloak(bool)` |
| `view/windows_native_composition.rs` | `attach()` 的 `cloak_pending` 由 `true` 改 `false`：合成不再自己 cloak                                       |
| `view/render.rs`                     | `sync_native_overlay_cloak` + 每帧驱动                                                                 |
| `main/src/main.rs`                   | 闸门翻回**默认启用合成**；逃生门 `NAVOP_RDP_DISABLE_COMPOSITION=1`                                               |

§5 的「`DWMWA_CLOAK` 必须晚于窗口第一次拿到真实尺寸并被绘制一次」仍是硬约束，  
`set_overlay_cloak` 完整照做：`is_actually_visible()` → `redraw()` → `set_cloaked(true)`。

### 11.4 一个必须限流的点

`redraw()` 是 `RedrawWindow(RDW_UPDATENOW | RDW_ALLCHILDREN)`：**同步**重绘，而且  
`ALLCHILDREN` 会把 mstsc 的子窗口一起拖进来。调用点在 GPUI 的 `render` 里，每次开关菜单都做  
既有开销、也有重入风险。所以只在**第一次** cloak 前做（`overlay_cloak_painted` 限流）：  
之后 composition visual 一直在承载会话的帧，不需要再证明一次。

### 11.5 契约测试

- `windows_native_overlay_is_cloaked_only_after_it_is_positioned_and_painted`：`attach`  
  断言改为 `self.cloak_pending = false;`；新增 `set_overlay_cloak` 的  
  「可见性检查 → 一次性重绘 → `set_cloaked`」顺序断言，以及 `if !self.overlay_cloak_painted {`  
  断言（同步重绘是前置条件，不是每次开菜单都要做）。
- `windows_native_rdp_keeps_direct_composition_unless_the_escape_hatch_is_set`（原  
  `..._disables_direct_composition_unless_composition_is_opted_in`）：断言默认**不再**关闭，  
  `GPUI_DISABLE_DIRECT_COMPOSITION` 只由 `NAVOP_RDP_DISABLE_COMPOSITION` 触发。

### 11.6 真机验证（2026-10-10，同日第二轮，全部通过）

同一份构建、同一台机器、同一个 `我的win` 连接，只改变"屏幕上有没有浮层"。  
三项判据互相独立，且都不经过日志：**DWM 的 cloak 属性**（`DwmGetWindowAttribute`  
`DWMWA_CLOAKED`）、**系统命中测试**（`WindowFromPoint`）、**窗口自己的位图**  
（`PrintWindow`）。

| 状态        | `DWMWA_CLOAKED` | RDP 区域 `WindowFromPoint`               | 日志                                   | `PrintWindow` 看到 |
| --------- | --------------- | -------------------------------------- | ------------------------------------ | ---------------- |
| 无浮层（刚连上）  | `0`             | `IHWindowClass` "Input Capture Window" | `overlay_cloak_toggle cloaked=false` | 远端桌面             |
| 右键标签，菜单打开 | **`1`**         | **`Zed::Window`** "Navop"              | `overlay_cloak_toggle cloaked=true`  | **菜单压在远端桌面之上**   |
| 点空白关掉菜单   | `0`             | `IHWindowClass`                        | `overlay_cloak_toggle cloaked=false` | 远端桌面，菜单消失        |

「输入没被牺牲」用的是**真实注入**而不是转发消息：在上表第三行的状态下，  
`SendInput` 点远端任务栏「开始」按钮（窗口相对 `(542,899)`），远端**开始菜单弹出来**  
（`png_diff` 50.08% 像素变化，包围盒 `x[9..1522] y[77..926]`）。

会话建立期间还有一个诚实记录：`overlay_cloak_toggle` 在 12 秒里翻了 **6 次**  
（`false→true→false→true→false`），随后在 `presentation ready` 之后稳定收敛到  
`false`。原因是连接期 navop 自己的"连接中"浮层是 deferred 渲染的，出现/消失各翻一次；  
稳态（菜单开着跨越大量帧）**不再翻转**，说明这个信号要的是"状态"而非"事件"这一点是对的。  
代价只是连接期几次多余的 DWM 重合成，不影响输入。

### 11.7 这台机器上"屏幕截图"看不见 navop —— 判据必须换

上一轮（§10）用 `BitBlt` 读屏幕就能看到 navop，这一轮同样的调用读到的是**桌面壁纸**。  
原因不是窗口不见了，而是**默认路径不再设置 `GPUI_DISABLE_DIRECT_COMPOSITION`**：  
navop 自己的内容改由 DirectComposition 承载，而 `BitBlt` 读的屏幕 DC 不包含 DComp  
visual —— 于是窗口所在区域读出来是"洞"，透出下面的桌面。

同一现象在 WorkBuddy（Electron，全屏、确确实实在最上面）身上也能观察到：它在 Z 序  
里压着 navop，但屏幕 BitBlt 同样读不到它。两者互相印证，不是 navop 的 bug。

**结论（写进工具约定）**：这台机器上验证 navop 一律用  
`PrintWindow(PW_RENDERFULLCONTENT)`（读窗口自己的重定向位图）+  
`WindowFromPoint` / `DWMWA_CLOAKED`（读系统状态），**不要**用屏幕 BitBlt 判断  
"画出来了没有"。另外 `gui.py` 的 `focus()` 会在点击前撤销置顶，遇到有全屏窗口  
（这里是 WorkBuddy）时点击会全部落到那个窗口上 —— 驱动脚本必须**在整个点击期间**  
保持 navop 置顶，见 `.workbuddy/tmp/rdp-test/drive.py`。

### 11.8 同一判据覆盖 dialog：真机观察 + 代码依据（2026-10-10）

真机反馈「**dialog 这些浮层不再被盖住了**」。查下来不是巧合：`gpui::deferred` 的覆盖面  
本来就比 §11.1 列的三处菜单大 —— **gpui-kit 的 dialog 也是 `deferred` 渲染的**。

| 位置                                             | 证据                                                                              |
| ---------------------------------------------- | ------------------------------------------------------------------------------- |
| gpui-kit `crates/base/src/dialog.rs:548`       | `Dialog::render` 的整个宿主是 `deferred(anchored().position(...).child(...))`         |
| gpui-kit `crates/base/src/alert_dialog.rs:193` | `pub struct AlertDialog(Dialog)` —— 报警框在同一文件里就是包着它的元组结构，走同一条路                   |
| navop `main/src/**`                            | 27 处 `window.open_dialog(...)` / `open_alert_dialog(...)` 调用点，全部落到上面那个 `Dialog` |

`has_deferred_content` 的取值是 `!next_frame.deferred_draws.is_empty() || prompt.is_some()`，  
dialog 落进前者，所以 **dialog 一开就 cloak、一关就 uncloak**，与菜单共用同一条路，  
**不需要为它加任何改动**。

日志证据（同一份构建、同一个 `我的win` 连接，`RUST_LOG=info,remote_desktop_view=debug`）：

| 时刻（本地）              | `overlay_cloak`             | 对应形状                       |
| ------------------- | --------------------------- | -------------------------- |
| 15:18:35            | `false`                     | `presentation ready` 之后的稳态 |
| 15:19:34 → 15:19:35 | `true` → `false`            | 一次 1 秒的浮层（菜单/下拉这一类）        |
| 15:20:35 → 15:21:30 | `true`（**约 55 秒**）→ `false` | 一个 dialog 开着 55 秒后关闭       |

稳态（既无菜单也无 dialog）不翻转；55 秒那一档正是「dialog 一直开着」的形状 —— 它证明  
cloak 是**跟着浮层存活**的，而不是"开了就忘了关"或靠定时器兜底。

同一判据覆盖到的浮层类型（`deferred(` 调用点）：菜单（popup / context / app menu bar）、  
**dialog / alert dialog**、popover / dropdown / combobox / select / date picker、tooltip、  
hover popover、touch-selection 编辑菜单，以及 GPUI 自己的 `prompt`。

**遗留的小口径**：tooltip 也走 `deferred`，鼠标停在带 tooltip 的控件上会短暂 cloak 一次。  
远端桌面不会消失（合成 visual 一直挂着，见 §11.2），只是那一瞬间鼠标命中回到 GPUI。  
若实际观感不合适，退路仍是 §9.5 的「浮层改成独立顶层窗口」。
