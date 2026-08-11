# Windows 原生 RDP ActiveX 嵌入 Navop Tab 实施计划

> Steps use checkbox (`- [ ]`) syntax for tracking. Update this document whenever scope, dependencies, decisions, commands, validation evidence, or status change.

**Goal:** 在 Windows 版 Navop 中使用系统 `mstscax.dll` 提供的 Remote Desktop ActiveX 控件，把真实的微软 RDP 客户端直接嵌入 GPUI 的远程桌面 tab；完整覆盖连接、输入、显示、剪贴板、音频、重连、安全、RD Gateway、资源重定向、多显示器、诊断、打包和发布，同时保留现有 canvas/IronRDP 路径作为跨平台实现和可靠回退。

**Architecture:** Navop 不启动并重挂外部 `mstsc.exe` 顶层窗口，而是在 GPUI 主窗口下创建受控的 Win32 child `HWND`，由一个小型 C++/ATL shim 在该窗口内创建 `MsRdpClient12` ActiveX 控件。Rust 通过窄 C ABI 持有 opaque handle、发送命令并接收结构化事件。`RemoteDesktopView` 使用 presentation 层在现有 canvas 和 Windows native child window 之间选择；native 控件自行绘制并处理输入，不伪装成 framebuffer backend。

**Tech Stack:** Rust 2024、GPUI、`windows` crate、Win32/COM/OLE、C++17、ATL、`cc` crate、`mstscax.dll` / `MsRdpClient12`、Cargo feature、现有 IronRDP/canvas backend、Windows x64/x86 CI 和真机/VM 验证。

**Target Platforms:** Windows 10/11 desktop x64；`i686-pc-windows-msvc` 的 32-bit Navop 进程在受支持 Windows x64 的 WoW64 环境中验证，并按产品现有支持策略决定是否额外覆盖 Windows 10 x86 OS。Windows 11 不存在 x86 OS 版本。Windows ARM64 不在首个 GA 支持矩阵中，必须完成独立 toolchain、COM 注册和安装包里程碑后才能声明支持。

**Primary Document Owner:** Remote Desktop / Windows integration maintainers.

---

## Execution Status

- [ ] Phase A：冻结产品 contract、验证 ATL/MSVC/toolchain、建立 native host 垂直切片。
- [ ] Phase B：完成 GPUI tab 嵌入、生命周期、fallback、凭据、输入、显示和基础媒体能力。
- [ ] Phase C：完成安全、RD Gateway、重定向、多显示器和 overlay/z-order 发布阻断项。
- [ ] Phase D：完成设置与诊断 UI、CI/安装包、真机矩阵、性能稳定性和灰度发布。
- [ ] Phase E：为 RemoteApp、扩展设备/端口、公开虚拟通道和 Windows ARM64 建立独立后续计划。

---

## 1. 已冻结的核心决策

### 1.1 正式实现路线

采用：

```text
Navop GPUI window
└── RemoteDesktopView tab
    └── Win32 child HWND
        └── ATL AxWin host
            └── MsRdpClient12 ActiveX control
                └── system mstscax.dll
```

不采用：

```text
启动 mstsc.exe
→ 查找外部顶层窗口
→ SetParent 重挂到 Navop
```

原因：

- 外部 `mstsc.exe` 的进程、窗口、焦点、DPI、对话框、退出和升级行为不受 Navop 控制。
- `SetParent` 不是受支持的 ActiveX 宿主 contract，会导致输入队列、窗口样式、所有者关系、模态对话框和 DPI awareness 不一致。
- ActiveX 控件本身就是微软提供给桌面容器应用的可嵌入接口，应直接宿主系统控件。

### 1.2 ActiveX 版本与接口边界

- 优先创建 `MsRdpClient12` CoClass。
- `MsRdpClient12` 实现的最高主客户端接口是 `IMsRdpClient10`。
- 不设计或生成不存在的 `IMsRdpClient11` / `IMsRdpClient12` 主接口。
- 高版本功能通过 `IMsRdpClient10`、`IMsRdpClientNonScriptable*`、`AdvancedSettings9`、`TransportSettings4`、事件 dispinterface 和对象集合访问。
- 运行时必须探测 CoClass、接口和属性可用性；不能仅凭编译时 Windows SDK 版本假定目标机器支持全部能力。

### 1.3 Rust/C++ 边界

- 首版不在 Rust 中手写完整 OLE ActiveX container。
- 使用 `AtlAxWinInit` 和 `AtlAxCreateControlEx`/`AtlAxGetControl` 完成容器和 event sink 建立。
- C++ shim 静态编入 Rust crate，默认不新增运行时 DLL。
- Rust 不跨 FFI 保存裸 COM interface pointer；只保存 opaque host handle。
- C ABI 只传固定宽度整数、POD 结构、长度明确的 UTF-8/UTF-16 slice 和 callback 函数指针。
- 所有 COM/ATL 创建、调用和销毁都回到创建控件的 GPUI Windows UI/STA 线程。

### 1.4 与现有 RDP 实现的关系

- 现有 IronRDP/canvas 实现继续保留。
- Windows native 是 presentation/backend preference，不替换跨平台实现。
- Windows native 创建或连接前置初始化失败时，`Auto` 模式回退 canvas。
- 用户显式选择 `Windows Native` 时不得静默回退；应显示可操作错误，并允许一键切换 canvas。
- 非 Windows、VNC 和现有 canvas RDP 行为不得回归。

---

## 2. 全局约束

### 2.1 工程约束

- 新增/改变公共 contract、状态机、线程边界、生命周期、凭据处理和 fallback 行为必须使用 Level 2 TDD：
  1. Red；
  2. Green；
  3. Refactor；
  4. Review；
  5. Completion verification。
- Rust 和 C++ 函数原则上不超过 50 行。
- 单个源文件原则上不超过 300 行；超过时按 lifecycle、configuration、events、redirection 等职责拆分。
- 嵌套深度不超过 3。
- 位置参数不超过 3；复杂输入使用 options/context struct。
- 圈复杂度不超过 10。
- 禁止魔法数字；HRESULT、DISPID、窗口消息、超时和 DPI 常量必须有命名来源。
- Windows-only 代码集中在专用 crate 和窄 presentation adapter 中，避免在跨平台 UI 大面积散布 `#[cfg(target_os = "windows")]`。
- 不修改、不回滚和本功能无关的工作区内容。

### 2.2 安全约束

- 密码、Gateway 密码、PIN、完整 credential blob 不得进入普通日志、事件 payload、错误详情、panic、测试快照或 telemetry。
- `ClearTextPassword` 是 write-only ActiveX 属性；Rust/C++ 临时明文缓冲写入后应尽快清零。
- 默认不允许保存由系统 ActiveX 弹窗获取的密码，除非用户明确启用 Navop 已有安全存储策略。
- 证书错误不得在无用户确认或无明确受管策略时自动忽略。
- 剪贴板、驱动器、打印机、智能卡、麦克风和设备重定向默认遵循最小权限；敏感重定向必须显式启用。
- 错误日志只记录脱敏 endpoint、HRESULT、RDP disconnect code、控件版本、系统版本和 correlation/session ID。

### 2.3 UI 与生命周期约束

- child `HWND` 初次创建时不可见或使用零大小 bounds，避免 tab 首帧闪现。
- child `HWND` 的位置和大小只在 GPUI 完成布局/prepaint 后，按最终 content bounds 更新。
- tab 失活必须隐藏 native child；tab 激活才显示。
- 隐藏或销毁 child 之前必须先把焦点交还 GPUI parent。
- tab 失活只隐藏，默认不主动断开 RDP session。
- `on_activate`、`on_deactivate`、`disconnect`、`destroy` 必须幂等。
- `try_close` 不是唯一清理入口；entity release/drop 和 force-close 路径必须同样安全。
- event sink 在 COM interface 和 host window 释放前执行 `Unadvise`。
- 所有异步回调都带 generation/session ID；旧 session 的事件和 resize completion 必须丢弃。
- ActiveX control 和 GPUI 不得同时处理同一组键鼠事件。

### 2.4 发布约束

- `windows-native-rdp` Cargo feature 首期默认关闭，真机矩阵和发布阻断项全部通过后再讨论默认策略。
- `mstscax.dll` 是系统组件，不随 Navop 安装包分发。
- 精简 Windows、Server Core、Wine、损坏注册或架构不匹配环境必须得到明确 `Unavailable` 诊断。
- x64 和 x86 分别验证对应 COM 注册表视图和系统组件。
- 没有交互桌面的 CI 只能覆盖编译、ABI、mock 和结构 contract；真实 ActiveX 必须在有桌面的 Windows VM/真机执行 smoke。
- Windows ARM64 在独立里程碑完成前不得出现在支持声明中。

---

## 3. 范围与完整功能矩阵

### 3.1 GA 必须完成

| 能力 | 用户行为 | Native 实现方向 | Fallback/限制 |
| --- | --- | --- | --- |
| 嵌入 | RDP 画面位于 Navop tab 内 | GPUI parent 下的 child `HWND` + ATL ActiveX host | 创建失败回退 canvas |
| 生命周期 | 打开、切 tab、关闭、应用退出稳定 | 显示/隐藏、focus handoff、幂等 destroy、STA 销毁 | force-close 由 release 兜底 |
| 基础连接 | host、port、用户名、domain、密码 | `Server`、RDP port setting、`UserName`、`Domain`、write-only password | 脱敏错误 |
| 身份验证 | NLA/CredSSP 默认安全连接 | `EnableCredSspSupport` 和相关 advanced settings | 不安全降级需显式确认 |
| 输入 | 键盘、鼠标、滚轮、组合键、Unicode、基础 IME | ActiveX 直接接收 focus/input | read-only 时阻止输入 |
| 显示 | 初始尺寸、动态分辨率、DPI、窗口缩放 | `DesktopWidth/Height`、`UpdateSessionDisplaySettings` / `SyncSessionDisplaySettings` | 不支持时 reconnect/letterbox |
| 剪贴板 | 文本双向复制粘贴 | clipboard redirection | 可关闭并显示安全状态 |
| 音频播放 | 本地播放/远端播放/禁用 | secured/advanced audio settings | 与现有模型映射 |
| 重连 | 自动重连、手动重连、网络恢复 | ActiveX reconnect events + Navop policy | 手动关闭禁止重连 |
| 状态/错误 | Connecting/Connected/Reconnecting/Disconnected | event sink + HRESULT/RDP code map | 未知码保留数值 |
| backend 选择 | Auto/Windows Native/Canvas | presentation factory | 显式 native 不静默回退 |
| 全屏 | 进入/退出全屏，返回原 tab | container-handled fullscreen 或受控 host transition | 避免游离顶层窗口 |
| overlay | 菜单、对话框、tooltip 不被 native HWND 盖住 | native-aware overlay policy | 未解决不得 GA |
| 打包 | x64/x86 可构建、安装、运行 | MSVC/ATL、静态 shim、系统 mstscax | 不打包系统 DLL |

### 3.2 完整桌面 RDP 能力

| 能力 | 默认值 | 计划阶段 |
| --- | --- | --- |
| 证书严格验证、证书变更警告、系统托管信任流程 | 严格；公开 API 将 `NotifyTSPublicKey` 标记为 unsupported，首个 GA 不提供自定义 TS public-key pinning | Task 16 |
| RD Gateway 主机、认证、usage/profile method | 关闭 | Task 17 |
| 文本剪贴板 | 开启，可关闭 | Task 12 |
| 文件/目录剪贴板 | 与文本剪贴板共用 `RedirectClipboard` 安全开关；首个 GA 不提供虚假的独立 toggle | Task 18 |
| 本地驱动器/指定共享目录 | 关闭，按项授权 | Task 19 |
| 打印机 | 关闭 | Task 19 |
| 智能卡 | 关闭 | Task 19 |
| 音频录制/麦克风 | 关闭 | Task 19 |
| 多显示器 | 关闭 | Task 20 |
| session takeover/session selection | 仅服务器要求时 | Task 20 |
| 网络质量、连接信息、自动重连进度 | 开启诊断 | Task 5/14/22 |
| 控件/系统版本诊断 | 开启，脱敏 | Task 1/22 |

桌面行为支持边界：

| 行为 | 首个 GA 边界 | 责任归属 |
| --- | --- | --- |
| ActiveX connection bar | 默认由 Navop toolbar 替代；若启用系统 connection bar，必须验证 close/minimize/restore | Task 15 capability-gated |
| 系统 credential/certificate/Gateway prompt | 允许系统控件托管，但 parent/owner、取消、超时和关闭状态由 Navop 跟踪 | Task 9/16/17/21 |
| 管理员/console session 与显式 session ID | 不作为首个 GA 承诺；Task 1 证明受支持属性和服务端行为后再 capability-gate | 后续小里程碑，不得假定等同普通连接 |
| Restricted Admin / Remote Credential Guard 等认证变体 | 默认不开放；先完成 Windows SDK/API、安全语义和域环境验证 | Task 9/16 的可选 enterprise capability |
| keyboard hook/Windows 快捷键 | 由 ActiveX keyboard hook mode 与 Navop focus-release shortcut 共同管理 | Task 10 |
| 远端 fullscreen request | Navop container 处理，可取消且有超时；不能产生 orphan window | Task 15 |
| session takeover/selection prompt | 首期可由系统控件托管，但必须有正确 owner、取消、超时和 state transition | Task 20/21 |

### 3.3 后续独立能力

以下功能必须在本计划中记录边界和前置条件，但不混入首个 GA 关键路径：

| 能力 | 原因 | 后续入口 |
| --- | --- | --- |
| RemoteApp | 窗口模型、z-order、任务栏、焦点和关闭语义与整桌面不同 | Phase E / Task 25 |
| USB/PnP/摄像头/位置重定向 | 设备枚举、授权、热插拔和隐私要求高 | Phase E / Task 26 |
| COM/LPT port redirection | 现代使用率低，安全和测试成本高 | Phase E / Task 26 |
| 自定义虚拟通道公开 API | 需要扩展权限、带宽、背压和隔离 contract | Phase E / Task 27 |
| Windows ARM64 | 当前 CI、release、installer 均未覆盖 | Phase E / Task 28 |
| 纯 Rust OLE container | 不增加用户功能，风险和维护成本远高于小型 ATL shim | 仅在 ATL 成为长期阻断时重新评估 |

### 3.4 明确非目标

- 不通过 `SetParent` 嵌入外部 `mstsc.exe`。
- 不把 ActiveX 的绘制结果抓取后伪装成现有 framebuffer backend。
- 不把 RD Gateway 当成 SOCKS5/HTTP proxy；两者配置、认证和失败语义独立。
- 不分发、替换或注册自带 `mstscax.dll`。
- 不在第一版实现新的通用 COM 框架。
- 不因 native 路径删除 IronRDP/canvas。

---

## 4. 目标架构

### 4.1 模块边界

```text
crates/remote_desktop_view
├── RemoteDesktopView
├── RemoteDesktopPresentation
│   ├── CanvasPresentation
│   └── WindowsNativePresentation   [Windows only]
└── connection/status/toolbar UI

crates/windows_rdp_host             [Windows only implementation]
├── Safe Rust facade
│   ├── WindowsRdpHost
│   ├── WindowsRdpOptions
│   ├── WindowsRdpEvent
│   ├── WindowsRdpCapabilities
│   └── WindowsRdpError
├── FFI boundary
│   └── opaque NativeRdpHost*
└── native C++/ATL shim
    ├── host window + AxWin
    ├── MsRdpClient12 configuration
    ├── COM event sink
    ├── lifecycle/thread assertions
    └── redirection/settings adapters

system
└── mstscax.dll
```

建议目录：

```text
crates/windows_rdp_host/
├── Cargo.toml
├── build.rs
├── src/
│   ├── lib.rs
│   ├── ffi.rs
│   ├── handle.rs
│   ├── event.rs
│   ├── options.rs
│   ├── capabilities.rs
│   └── error.rs
└── native/
    ├── windows_rdp_host.h
    ├── host.cpp
    ├── lifecycle.cpp
    ├── configuration.cpp
    ├── display.cpp
    ├── redirection.cpp
    ├── event_sink.cpp
    └── error.cpp
```

### 4.2 Presentation contract

```rust
enum RemoteDesktopPresentation {
    Canvas(CanvasPresentation),
    #[cfg(target_os = "windows")]
    NativeWindows(WindowsNativePresentation),
}

enum RemoteDesktopBackendPreference {
    Auto,
    WindowsNative,
    Canvas,
}
```

Presentation 层拥有：

- native child window 的创建和销毁；
- tab activate/deactivate；
- bounds、visibility、focus、DPI；
- connect/disconnect/reconnect；
- native event 到现有 view state 的映射；
- capability 和 fallback 结果；
- read-only、全屏、overlay 协调。

Presentation 层不拥有：

- 密码长期存储；
- tab 容器；
- 全局设置 UI；
- 与 native 无关的 IronRDP runtime；
- 业务层日志和 telemetry sink。

### 4.3 C ABI 草案

ABI v1 lifecycle prefix 已在 Task 2 第一最小切片的 contract test 后冻结；后续接口只在
各自小切片的 contract test 后分别冻结。复杂参数通过 versioned struct 传递，不增加大量位置参数。

Task 2 第一最小切片已经冻结以下 **ABI v1 lifecycle prefix**。这一版只覆盖
`probe/create/destroy` 和 opaque handle，不包含 parent `HWND`、callback、connect、
credentials、bounds 或 ActiveX/COM 对象：

```c
typedef struct NativeRdpHost NativeRdpHost;

typedef int32_t NavopRdpResult;

#define NAVOP_RDP_ABI_VERSION UINT32_C(1)

typedef struct NavopRdpProbeOptions {
    uint32_t struct_size;
    uint32_t abi_version;
} NavopRdpProbeOptions;

typedef struct NavopRdpProbeResult {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t available;
    uint32_t reserved;
} NavopRdpProbeResult;

typedef struct NavopRdpCreateOptions {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t generation_low;
    uint32_t generation_high;
} NavopRdpCreateOptions;

NavopRdpResult navop_rdp_probe(
    const NavopRdpProbeOptions* options,
    NavopRdpProbeResult* out_result);
NavopRdpResult navop_rdp_create(
    const NavopRdpCreateOptions* options,
    NativeRdpHost** out_host);
NavopRdpResult navop_rdp_destroy(NativeRdpHost** host);
```

ABI v1 lifecycle prefix 规则：

- 所有 versioned struct 的 `struct_size` 固定在 offset 0，`abi_version` 固定在 offset 4。
- `struct_size >= sizeof(current layout)` 表示调用方提供了当前已知前缀以及可能存在的未来尾部字段；实现只访问已知前缀，不访问或修改未知尾部。
- 输出 struct 必须保留调用方传入的 `struct_size`，不能把更大的未来布局缩回当前尺寸。
- 校验顺序固定为 null pointer → `struct_size` → `abi_version`；尺寸不足时不得提前读取 version。
- 新字段只能追加到 versioned struct 尾部；不能重排、复用或改变已冻结字段的宽度。
- `generation_low`/`generation_high` 代替 ABI 中直接暴露的 `uint64_t`，避免 x86/x64 alignment 差异；数值按 `low | (high << 32)` 重组。
- `NavopRdpResult` 固定为 32-bit signed integer；C++ 异常不得跨 C ABI。
- `create` 在可写的 `out_host` 非空时先将 `*out_host` 置空；`destroy` 使用 `NativeRdpHost**`，成功后置空，重复 destroy 可安全返回。

以下是 **尚未冻结的后续扩展方向**。parent `HWND`、callback、connect、credentials、
bounds 和 settings 必须在各自小切片中增加独立 versioned struct/entrypoint，并通过
x64/x86 layout contract；不能无版本地塞回已经冻结的 lifecycle prefix：

```c
typedef struct {
    uint32_t struct_size;
    uint32_t abi_version;
    const uint16_t* host;
    uint32_t host_len;
    uint16_t port;
    const uint16_t* username;
    uint32_t username_len;
    const uint16_t* domain;
    uint32_t domain_len;
    uint32_t flags;
} NavopRdpConnectOptions;

typedef struct {
    const uint16_t* data;
    uint32_t len;
} NavopRdpBorrowedSecret;

typedef struct {
    uint32_t struct_size;
    uint32_t abi_version;
    NavopRdpBorrowedSecret server_password;
    NavopRdpBorrowedSecret gateway_password;
    uint32_t flags;
} NavopRdpCredentialBundle;

typedef struct {
    uint32_t struct_size;
    uint32_t abi_version;
    int32_t x;
    int32_t y;
    int32_t width;
    int32_t height;
    uint32_t dpi;
} NavopRdpBounds;

NavopRdpResult navop_rdp_connect(
    NativeRdpHost* host,
    const NavopRdpConnectOptions* options);
NavopRdpResult navop_rdp_apply_credentials(
    NativeRdpHost* host,
    const NavopRdpCredentialBundle* credentials);
NavopRdpResult navop_rdp_set_bounds(
    NativeRdpHost* host,
    const NavopRdpBounds* bounds);
NavopRdpResult navop_rdp_set_visible(
    NativeRdpHost* host,
    uint8_t visible);
NavopRdpResult navop_rdp_focus(NativeRdpHost* host);
NavopRdpResult navop_rdp_disconnect(
    NativeRdpHost* host,
    uint32_t reason);
```

扩展功能按 settings struct 分组：

- `NavopRdpSecurityOptions`
- `NavopRdpDisplayOptions`
- `NavopRdpClipboardOptions`
- `NavopRdpAudioOptions`
- `NavopRdpGatewayOptions`
- `NavopRdpRedirectionOptions`
- `NavopRdpMultimonOptions`

ABI 规则：

- 所有后续 versioned struct 继续使用 `struct_size`/`abi_version` 固定前缀和 append-only 演进。
- C++ 不保存 Rust 临时 slice pointer。
- `NavopRdpBorrowedSecret` 只在 FFI 调用返回前有效；native 不跨调用保存该 pointer。
- server 和 Gateway secret 使用不同字段，禁止凭据来源不明确或意外复用。
- Rust 输入使用 zeroizing owner；C++ 只建立一次性、可清零的 UTF-16/BSTR buffer，ActiveX property setter 返回后立即 `SecureZeroMemory`，所有失败、异常、取消和重复连接路径执行相同清理。
- crash reporting 默认不得生成或上传包含进程完整内存的 dump；若平台必须保留 dump，先定义 secret scrubbing 和访问控制。
- callback 不直接更新 GPUI entity，只复制为 owned Rust event 并投递到 UI executor。
- callback 发出期间不得重入 destroy。
- callback 进入时增加 in-flight 计数，退出时减少；`Closing` 后不再转发新事件。
- destroy 使用 `NativeRdpHost**`，成功后置空，保证重复调用可检测。
- 错误返回固定 code + HRESULT + extended disconnect reason；错误文本在 Rust 侧本地化。

### 4.4 事件模型

```rust
pub enum WindowsRdpEvent {
    HostReady { generation: u64, capabilities: WindowsRdpCapabilities },
    Connecting { generation: u64 },
    Connected { generation: u64 },
    LoginComplete { generation: u64 },
    Reconnecting { generation: u64, attempt: u32, max_attempts: Option<u32> },
    Reconnected { generation: u64 },
    NetworkStatusChanged { generation: u64, quality: Option<u32> },
    RemoteDesktopSizeChanged { generation: u64, width: u32, height: u32 },
    FullscreenChanged { generation: u64, fullscreen: bool },
    AuthenticationWarning { generation: u64, visible: bool },
    Warning { generation: u64, warning: WindowsRdpWarning },
    FatalError { generation: u64, error: WindowsRdpFatalError },
    LogonError { generation: u64, error: WindowsRdpLogonError },
    Disconnected { generation: u64, reason: WindowsRdpDisconnectReason },
    CloseConfirmed { generation: u64 },
    FocusReleased { generation: u64 },
}
```

事件处理规则：

- event sink 只转换必要参数，不执行长任务。
- 回调进入 Rust 后复制数据，按 `generation` 过滤。
- 状态机拒绝 `Closing`/`Released` 后的事件。
- 同一 ABI version 内保留未知 event kind 和 malformed known payload；既有 kind 的
  payload schema 不扩展，schema 变化必须分配新的 kind。
- callback payload 最大 64 KiB，native dispatch 和 Rust callback 边界都拒绝超限长度。
- `Disconnected` 区分用户关闭、网络、认证、证书、Gateway、服务端策略和未知错误。
- 未知 code 保留原始数值，不能错误归类。

### 4.5 生命周期状态机

```text
Created
  → NativeChildCreated
  → Connecting
  → Active
  ↔ Inactive
  → Reconnecting
  → Active
  → Closing
  → NativeChildDestroyed
  → Released
```

关闭顺序：

```text
标记 Closing
→ generation 失效，拒绝新输入和新事件应用
→ focus_parent
→ hide child HWND
→ RequestClose/Disconnect
→ Unadvise event sink
→ 等待/排空已进入的 callback，禁止新 callback 进入 Rust
→ release ActiveX/COM interfaces
→ destroy AxWin/host child HWND
→ clear credential buffers
→ 标记 Released
→ 允许 TabContainer 移除 tab
```

异常关闭：

- `try_close`：优先请求 graceful close，可短暂等待 `OnConfirmClose`。
- force close/entity release/app exit：直接进入幂等 destroy，不依赖 tab callback。
- 创建中失败：按已获得资源逆序清理。
- C++ 异常不得跨 C ABI；统一转 `NavopRdpResult`。
- 从 callback 内发起 close 时只投递下一 UI tick 的 close command，不能同步 `Unadvise`/release 当前 sink。

### 4.6 线程模型

- 创建、配置、连接、resize、显示、隐藏、focus、disconnect、destroy 在同一 Windows UI/STA 线程执行。
- Task 1 必须确认 Navop Windows 主线程现有 COM apartment；若不是 STA，先制定显式初始化策略。
- 每个 host 记录 owner thread ID，并通过 `OwnerThreadDispatcher`/受控窗口消息队列串行执行全部 COM command。
- 不允许后台 Tokio task 直接调用 COM interface。
- 后台任务只能构造 command，随后投递 UI executor。
- callback 可能发生在 COM 调用栈内；Rust handler 必须避免同步重入 ActiveX。
- 需要调用 ActiveX 的事件响应安排到下一次 UI tick。
- destroy 在 owner thread 上先关闭 callback gate，再 `Unadvise`，随后等待 in-flight callback 归零；等待必须有可观测 timeout，不能阻塞在当前 callback 栈。
- 应用退出顺序固定为：停止创建新 host → 请求所有 host 关闭 → drain owner-thread command/callback → 销毁 host → COM apartment uninitialize → 退出 UI thread。
- owner-thread dispatcher 拒绝或无法执行 cleanup 是 fatal lifecycle bug；不得改在线程错误的地方调用 COM。紧急进程退出时宁可记录并让 OS 回收进程资源，也不做 wrong-thread release。
- Task 7 必须测试 callback 正在执行、投递失败、UI shutdown 和重复 close 四类 race。

### 4.7 Bounds、DPI 与 clipping

转换链：

```text
GPUI logical bounds
→ current window scale factor
→ Win32 physical pixel bounds
→ parent-client coordinates
→ SetWindowPos(child HWND)
→ UpdateSessionDisplaySettings if remote desktop size should change
```

规则：

- 区分 host window physical bounds 和 remote desktop requested resolution。
- 禁止对已经是 physical pixel 的值再次乘 scale factor。
- 记录最后一次已应用 bounds/DPI，避免 resize 风暴。
- resize debounce 只用于远端分辨率更新；本地 child HWND bounds 必须及时跟随布局。
- 100%、125%、150%、200% DPI 全部验证。
- 跨显示器时重新读取目标 monitor DPI。
- GPUI clip/content mask 不会自动裁剪 child HWND，必须用 Win32 region/独立 host 布局策略验证。
- parent/client 坐标使用 `MapWindowPoints` 或等价 Win32 API 统一转换，最终 `SetWindowPos`/`SetWindowRgn` 参数使用 parent client physical pixels。
- process/window DPI awareness context 在创建 child 前固定；运行时以 `GetDpiForWindow(parent)`/等价结果为准，不在同一调用链混用 virtualized coordinates。
- child 创建后不进行跨顶层窗口 `SetParent`；parent 重建时销毁并在新 owner thread/window 下重建 host。

### 4.8 Overlay 与 z-order

Win32 child `HWND` 通常覆盖同一窗口内由 GPU 绘制的 GPUI 内容。以下任一元素如果与 RDP 区域重叠，都可能被盖住：

- tooltip；
- context menu；
- command palette；
- dropdown/select；
- modal/dialog；
- toast；
- tab drag preview；
- connection toolbar overlay。

Task 21 必须选定并验证正式策略，候选包括：

1. native 区域出现 overlay 时临时隐藏/裁剪 child HWND；
2. 需要覆盖 native 区域的交互 UI 使用 owned native popup window；
3. 调整布局，保证关键 overlay 不跨 native 区域；
4. 组合策略。

单纯调用 GPUI `z-index` 不是解决方案。该问题是 GA 阻断项。

---

## 5. 配置与持久化 contract

建议新增：

```rust
pub struct WindowsNativeRdpSettings {
    pub backend: RemoteDesktopBackendPreference,
    pub security: RdpSecuritySettings,
    pub display: RdpDisplaySettings,
    pub clipboard: RdpClipboardSettings,
    pub audio: RdpAudioSettings,
    pub gateway: Option<RdpGatewaySettings>,
    pub redirection: RdpRedirectionSettings,
    pub multimon: RdpMultimonSettings,
    pub reconnect: RdpReconnectSettings,
}
```

向后兼容规则：

- 新字段全部使用 `serde(default)` 或等价 migration。
- 旧连接没有 backend 字段时解释为 `Auto`。
- 非 Windows 读取 `WindowsNative` preference 时显示不可用并使用 canvas，不破坏配置文件。
- 密码和 Gateway 密码复用 Navop 安全凭据存储，不写入普通 JSON。
- 现有 SOCKS5/HTTP proxy 字段保持原语义。
- RD Gateway 使用独立结构，不能复用 proxy 字段。

建议枚举：

```rust
pub enum RdpCertificatePolicy {
    Strict,
    PromptOnUntrusted,
    PinnedServerPublicKeySha256,
}

pub enum RdpAudioPlaybackMode {
    Local,
    Remote,
    Disabled,
}

pub enum RdpAudioCaptureMode {
    Disabled,
    LocalMicrophone,
}

pub enum RdpGatewayUsage {
    Never,
    Detect,
    Always,
    UseServerProfile,
}
```

---

## 6. Capability 探测与 fallback contract

`WindowsRdpCapabilities` 至少包含：

- OS version/build；
- process architecture；
- `mstscax.dll` file/product version；
- `MsRdpClient12` CoClass 是否可创建；
- 最高可 query 的主接口；
- 可用 non-scriptable interface 版本；
- dynamic display update；
- clipboard controller/manual sync；
- drive/device collections；
- Gateway transport settings；
- multi-monitor；
- camera/device/location；
- event sink 建立结果；
- ATL host 初始化结果。

Capability 分类：

- **Hard-required:** AxWin host 可初始化、`MsRdpClient12` 可创建、`IMsRdpClient10` 可 query、基础 non-scriptable interface 可 query、`IMsTscAxEvents` connection point 可 advise、child `HWND` 可创建。
- **Optional:** `IMsRdpClientNonScriptable8`、高版本 advanced/transport settings、dynamic display、manual clipboard controller、Gateway、drive/device collection、multi-monitor、camera/location 和其他 redirection。
- hard-required 缺失使 native backend `Unavailable`；optional 缺失只关闭对应 UI capability，不能使基础连接失败。

Probe contract：

- 不连接任何服务器，不读取或提交凭据，不打开持久系统 prompt。
- 只创建隐藏零大小 parent/control、query interfaces/version/capabilities，随后按正常顺序 `Unadvise`/destroy。
- 同一进程可重复且结果可缓存；OS/control file version 或 process architecture 变化时缓存失效。
- probe 失败返回稳定分类，例如 `ClassNotRegistered`、`RequiredInterfaceMissing`、`AtlHostUnavailable`、`WrongApartment`、`HostWindowFailed`。
- probe 不留下 HWND、COM ref、credential、后台线程或 session side effect。

`Auto` 模式允许 fallback 的时机：

- 控件创建前的 `ClassNotRegistered`；
- `mstscax.dll`/ATL host 初始化失败；
- 必需主接口不可用；
- child host 建立失败；
- 明确标记为 native-unavailable 的兼容性错误。

`Auto` 模式不得在以下时机自动切 backend：

- 已经向服务器提交凭据后发生认证失败；
- 证书不可信；
- Gateway 认证失败；
- 服务端拒绝；
- 用户主动断开；
- session 已连接后发生网络错误。

原因：连接期静默切 backend 可能重复提交凭据、改变证书/重定向策略或产生两个并行 session。

---

## 7. 测试分层

### 7.1 跨平台单元/contract 测试

- backend selection；
- config default/migration；
- fallback 分类；
- lifecycle state machine；
- generation 过滤；
- error code 映射；
- secret redaction；
- bounds/DPI 数学；
- reconnect policy；
- feature-gate dependency contract。

### 7.2 Windows mock/shim 测试

- C ABI version、struct size/alignment；
- create failure 逆序释放；
- callback ownership；
- destroy 幂等；
- wrong-thread rejection；
- event-before/after-destroy；
- resize coalescing；
- password buffer clear；
- event sink advise/unadvise。

### 7.3 Windows 有桌面集成测试

- 创建真实 `MsRdpClient12`，但不连接；
- 创建、show/hide、resize、focus、destroy 100 次；
- tab 快速切换；
- popup/overlay 覆盖；
- 多 DPI/跨 monitor；
- 系统对话框 owner；
- 应用退出和 force close。

### 7.4 RDP 服务器真机矩阵

- Windows 10/11 Pro；
- Windows Server 2016、2019、2022，以及发布时支持的更新版本；
- NLA on/off；
- 本地账号、domain account；
- 自签名、受信任、证书变更；
- RD Gateway；
- 网络断开/恢复；
- 剪贴板、音频、驱动器、打印机、智能卡、麦克风；
- 单显示器、多显示器；
- x64/x86 client。

### 7.5 长稳和资源检测

- 8 小时连接；
- 100 次 connect/disconnect；
- 100 次 tab open/close；
- sleep/resume；
- network flap；
- explorer/DWM restart；
- Windows update 前后 smoke；
- WinDbg/Application Verifier/诊断工具检查 HWND、COM ref、GDI/USER handle 和 heap 泄漏。

---

## 8. 分阶段实施计划

### Task 0: 冻结范围、feature flag、backend selection 与状态模型

**Goal:** 在写 native 代码前冻结产品语义，防止 ActiveX 细节泄漏到整个 remote desktop UI。

**Depends on:** 无。

**Files:**

- Modify: `main/Cargo.toml`
- Modify: `crates/remote_desktop_view/Cargo.toml`
- Modify: `crates/remote_desktop_view/src/view.rs`
- Modify: `crates/core/src/storage/models.rs`
- Add focused tests near the modified modules

**Produces:**

- `windows-native-rdp` feature contract。
- `RemoteDesktopBackendPreference`。
- `RemoteDesktopPresentation` skeleton。
- lifecycle state enum。
- `Auto` fallback 分类函数。
- 配置 migration defaults。

- [x] **Red:** 添加 manifest/feature contract 测试，要求 non-Windows 默认构建不依赖 ATL/Windows native crate。
- [x] **Red:** 添加 backend selection 测试：Windows+Auto+available 选 native；unavailable 选 canvas；显式 native unavailable 返回错误；非 Windows 选 canvas。
- [x] **Red:** 添加旧配置反序列化测试，确认缺少新字段时得到兼容默认值。
- [x] **Green:** 建立 presentation enum/factory skeleton，所有 native 分支先返回 `UnavailableNotBuilt`。
- [x] **Green:** 增加 feature 声明但保持默认关闭。
- [x] **Refactor:** 让 selection、fallback、config migration 成为无 UI 依赖的纯函数。
- [x] **Review:** 检查没有把 ActiveX 术语扩散到 connection 通用模型以外。
- [x] **Verify:** 运行 `rtk cargo test -p remote_desktop_view`、`rtk cargo test -p one-core`、`rtk cargo check -p main`。

**Execution Notes (2026-08-07):**

- Red evidence:
  - manifest contract 首次运行失败为 `remote_desktop_view must declare default features`。
  - presentation contract 首次运行失败为缺少 `RemoteDesktopBackendPreference` 和 `view::presentation`。
  - legacy config 测试首次运行失败为缺少 `RemoteDesktopBackendPreference` 类型及 `backend_preference` 字段。
  - 原计划命令中的 package 名称 `core` 无效；仓库实际 package 名称为 `one-core`。
- Green implementation:
  - 在 `main` 和 `remote_desktop_view` 声明 `windows-native-rdp`，默认 feature 集均不启用它。
  - 新增持久化 preference：`Auto`、`WindowsNative`、`Canvas`；旧 JSON 缺字段时默认为 `Auto`，默认值不写回 JSON，显式值使用 snake_case。
  - 新增 crate-private presentation、platform、availability、error 和 lifecycle contract，以及无 UI 依赖的 selection/fallback 纯函数。
  - 当前 native availability 固定为 `UnavailableNotBuilt`；`Auto` 可回退 canvas，显式 `WindowsNative` 在 Windows native 不可用时返回错误。
  - 编辑已有连接时保留其 backend preference；通用 `RemoteDesktopConnectionOptions` 未加入 native/presentation 字段。
- Refactor/review:
  - 独立只读审查未发现阻塞问题；已抽查 feature、serde、selection matrix、所有 `RemoteDesktopParams` literal、API 可见性和通用 runtime model。
  - presentation 模块暂以带注释的 `allow(dead_code)` 保存 staged contract；后续真实接线时必须移除。
  - lifecycle 本 Task 仅冻结状态集合，不实现状态转移规则；状态机约束由后续 lifecycle Task 补齐。
- Automated verification:
  - `rtk cargo fmt --all -- --check`：通过。
  - `rtk cargo test -p remote_desktop_view`：101 passed。
  - `rtk cargo test -p remote_desktop_view --features windows-native-rdp`：101 passed。
  - `rtk cargo test -p one-core`：425 passed，3 ignored。
  - `rtk cargo test -p remote_desktop`：96 passed。
  - `rtk cargo test -p onetcli_runtime`：70 passed。
  - `rtk cargo check -p main`：0 errors。
  - `rtk cargo check -p main --features windows-native-rdp`：0 errors。
  - manifest 定向测试、presentation 定向测试、legacy config/roundtrip 定向测试及 `rtk git diff --check`：通过。
  - `cargo metadata` 确认 `main/windows-native-rdp` 只转发到 `remote_desktop_view/windows-native-rdp`，且两个 package 的默认 feature 均未包含该 feature。
- Manual verification:
  - 本 Task 不创建 native host，也不需要 Windows 真机验证；Windows ATL/MSVC 和 `MsRdpClient12` smoke 属于 Task 1。
- Known limitations:
  - `windows-native-rdp` 当前是空 feature，尚未依赖 `windows_rdp_host`，factory 因此不会创建 native presentation。
  - feature-off 的 canvas 行为通过“不接线现有 render/runtime”和现有 view 测试保持；真实 backend 切换测试将在 presentation 接线时补充。
- Decision changes:
  - Task 0 本身没有修改根 `Cargo.toml`：virtual workspace 在该 Task 没有需要声明的 package feature。
  - Task 1 为管理临时 `windows-rdp-probe` workspace tool 提前修改根 `Cargo.toml`/`Cargo.lock`；正式 `windows_rdp_host` member、dependency 和 feature 接线仍延后到 Task 2。
  - `crates/remote_desktop_view/src/view/render.rs` 本 Task 不修改，避免在 native host 存在前提前改变 canvas render tree。

**Acceptance:**

- feature off 时行为和当前版本一致。
- 旧配置可无损读取。
- backend 选择和 fallback 语义由测试固定。

**Rollback:** 删除未接入的 skeleton 和 feature 声明即可恢复；不迁移或删除已有配置字段。

---

### Task 1: Windows ATL/MSVC/toolchain 与系统能力 spike

**Goal:** 用最小 console/window spike 证明目标 x64/x86 toolchain 能创建并销毁 `MsRdpClient12`，并冻结真实 Windows SDK 接口定义。

**Depends on:** Task 0。

**Files:**

- Add: `tools/windows-rdp-probe/`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Add: `script/build-windows-rdp-probe.ps1`
- Modify: `script/install-window.ps1`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Record results in this document

**Probe checklist:**

- Visual Studio 2022 C++ desktop workload。
- ATL component：`Microsoft.VisualStudio.Component.VC.ATL`。
- 系统注册的 Remote Desktop ActiveX type library；Windows SDK 不提供可直接包含的 `mstscax.h`，由 MSVC `#import "libid:..."` 生成 `mstscax.tlh`。
- Microsoft 公开文档中的 `MsRdpClient12` CLSID 和 optional `IMsRdpClientNonScriptable8` IID；不手写 COM interface vtable。
- `AtlAxWinInit` 链接依赖。
- `CoInitializeEx`/现有 UI thread apartment 状态。
- x64/x86 CoClass 注册和创建。
- `QueryInterface(IMsRdpClient10)`。
- `QueryInterface(IMsRdpClientNonScriptable8)` 及低版本降级。
- `NotifyTSPublicKey` + `OnReceivedTSPublicKey` 是否可在连接继续前完成 SHA-256 public-key pin decision。
- 文本 clipboard 和文件/目录 clipboard 是否由同一 `RedirectClipboard` 安全边界控制；是否存在可分别禁用 file clipboard 的受支持 API。
- Restricted Admin、Remote Credential Guard、admin/console/session-id 等 enterprise 选项的真实 SDK property 和系统版本边界。
- 控件 `Version` 和 DLL file version。

- [ ] **Red:** 在缺失 ATL component 的干净 Windows runner/VM 运行 probe，记录预期构建失败。
- [x] **Green:** 更新安装脚本或 runner setup，确保 x64/x86 probe 编译和链接。
- [ ] **Green:** 在有桌面 Windows 环境创建隐藏 parent window 和 ActiveX control，读取 version 后销毁。
- [x] **Green:** 明确 `MsRdpClient12 → IMsRdpClient10`，禁止引入 `IMsRdpClient11/12`。
- [ ] **Green:** 在 clean Windows 10/11 x64 WoW64 环境运行 Navop x86 进程，验证 32-bit ATL、32-bit COM 注册表视图、CoClass create/query、connect/disconnect 和安装后 probe；若产品继续支持 Windows 10 x86 OS，再增加对应 VM。
- [x] **Green:** 记录 public-key pin 和文件 clipboard 的 API proof；无法证明时从 GA 承诺降级为系统托管/unsupported。
- [x] **Refactor:** spike 只保留可复用 SDK/ABI 探测代码；删除一次性调试代码。
- [x] **Review:** 检查不依赖机器上的非系统注册、不复制 mstscax DLL。
- [x] **Verify:** 在 GitHub-hosted Windows runner 分别执行 locked `cargo build`，完成 `x86_64-pc-windows-msvc` 和 `i686-pc-windows-msvc` 的 C++/Rust 编译与链接。

上述已勾选项的证据边界：

- public-key/clipboard 的 `[x]` 只表示已依据 Microsoft 公开文档完成静态 API proof 和产品降级决策，不表示对应 Windows runtime capability 已验证。
- Refactor/Review 的 `[x]` 只表示 spike 源码、静态 contract 和发行边界已审查，不表示 Windows MSVC/ATL compile/link、ActiveX create/query/destroy 或 connect/disconnect 已审查通过。

**Execution Notes (2026-08-07):**

- Local/static completed:
  - 在 `tools/windows-rdp-probe/` 增加临时 probe crate，并作为 workspace member 管理，但不加入 `default-members`，因此不会改变默认产品构建入口。
  - 首次 contract Red 固定为 5 个失败；实现和 Windows runner 修复后扩展为 11 个通过的契约测试，覆盖 Windows-only MSVC/ATL build boundary、临时 C ABI 一致性、初始化/逆序清理顺序、DLL version 非致命诊断回退、x64/x86 workflow、系统 type library 导入、公开 GUID、`atls.lib` 静态链接、禁止运行/打包 probe、禁止复制/注册 `mstscax.dll`，以及非 Windows no-op 的固定退出码和输出。
  - 非 Windows 本地可 build/test/run；输出固定为 `windows-rdp-probe status=unsupported reason=requires-windows-msvc-atl`，退出码为 0。该结果只证明 unsupported contract，不证明任何 Windows COM 能力。
  - Windows SDK 不提供 `mstscax.h`。native spike 通过 `#import "libid:8C11EFA1-92C3-11D1-BC1E-00C04FA31489" raw_interfaces_only, named_guids, no_namespace, exclude("UINT_PTR")` 从目标架构对应的系统注册 type library 生成 `mstscax.tlh`；`exclude("UINT_PTR")` 避免 i686 generated declarations 与 Windows headers 重定义。
  - native spike 源码声明并由 contract 断言使用 C++17、ATL AxHost、注册类名 `L"AtlAxWin"`、Microsoft 公开的 `MsRdpClient12` CLSID、generated `IMsRdpClient10`、以公开 IID + `IUnknown` 探测的 optional `IMsRdpClientNonScriptable8`、控件 `Version` 和已加载系统 `mstscax.dll` file version。没有复制第三方 header、没有手写 COM interface vtable；只通过单个临时 `extern "C" int32_t windows_rdp_probe_run(void)` 进入 Rust，没有提前实现 Task 2 的 opaque host、版本化 ABI 或 callback API。
  - `script/install-window.ps1` 限定 Visual Studio 2022 `[17.0,18.0)`，显式安装 Native Desktop 与 `VC.ATL`，并为 Scoop/current-process PATH、extras bucket 和 CMake 增加可重复执行的静态 contract。
  - `script/build-windows-rdp-probe.ps1` 显式探测 `atlbase.h` 和 `vcvarsall.bat`，将 x64/i686 分别映射到 `vcvarsall.bat x64`/`x86`，只执行 locked `cargo build`。`build.rs` 按 `VCToolsInstallDir/atlmfc/lib/{x64|x86}/atls.lib` 验证并静态链接现代 ATL 支持库 `atls.lib`；CI matrix 和 release Windows build 都调用同一个 compile-only gate，probe 不运行、不上传、不进入发行包。
  - C++ source 显式执行 `OleInitialize → AtlAxWinInit → hidden parent → AtlAxWin child → ActiveX control → interface/version inspection`，RAII cleanup 固定为 control/container release、child/parent destroy、`AtlAxWinTerm`、`OleUninitialize` 的逆序释放。源码和 contract 已审查，Windows runner 已完成 compile/link；由于 runner 未执行 probe executable，这些证据仍不能替代 Windows runtime 结果。
  - 静态源码检查确认 Navop 经 `gpui_platform::application()` 构造 Windows platform，`gpui_windows::WindowsPlatform::new` 调用 `OleInitialize(None)`，随后 `run` 进入 `GetMessageW`/`TranslateMessage`/`DispatchMessageW` 消息循环；这只说明现有初始化路径，不证明实际 owner thread/apartment。正式 host 仍须在 Windows 上用 `CoGetApartmentType` 或等价断言确认创建、调用和销毁线程；当前检查也未在该 platform 路径发现对应的显式 `OleUninitialize`，后续生命周期实现必须明确 apartment ownership。
  - 已进行独立只读审查，并按审查结果补充 VS 2022 版本范围、Scoop 幂等标记、UTF-8 cmd code page、生命周期/ABI contract、`VS_FFI_SIGNATURE` 校验，以及保持 probe 成功状态的 `dll_version=unavailable dll_version_reason=<reason>` 非致命诊断。
- Local/static verification:
  - `rtk cargo fmt --all -- --check`：通过。
  - `rtk cargo test --locked -p windows-rdp-probe`：11 passed。
  - `rtk cargo run --locked -p windows-rdp-probe`：退出码 0，输出固定为 `windows-rdp-probe status=unsupported reason=requires-windows-msvc-atl`。
  - `rtk cargo metadata --locked --no-deps --format-version 1`：确认 probe 属于 workspace 且不属于 `workspace_default_members`。
  - `.github/workflows/ci.yml`、`.github/workflows/release.yml`：YAML 静态解析通过。
  - `rtk git diff --check`：通过。
  - 当前 macOS 环境没有 `pwsh`，未执行 PowerShell parser、安装脚本或 Windows probe；这些结果不得由上述静态验证替代。
- GitHub-hosted Windows compile/link verification:
  - Commit：`9cbdea736794c09f93a818d1df3dab92c0dc97b8`。
  - Workflow run：[`31168540082`](https://github.com/feigeCode/navop/actions/runs/31168540082)。
  - x64 job：[`Windows RDP probe (x86_64-pc-windows-msvc)` / `92834877253`](https://github.com/feigeCode/navop/actions/runs/31168540082/job/92834877253)，`Build ATL/MSVC probe` 成功。
  - x86 job：[`Windows RDP probe (i686-pc-windows-msvc)` / `92834877285`](https://github.com/feigeCode/navop/actions/runs/31168540082/job/92834877285)，`Build ATL/MSVC probe` 成功。
  - 两个 job 证明对应 MSVC/ATL C++ 和 Rust 最终链接成功，能从各自注册表视图解析系统 RDP type library、生成并编译 `mstscax.tlh`，并正确链接 `atls.lib`。该 workflow 的无关通用 Windows test job 在目标 probe job 完成后被取消以节省 runner；不能把 workflow 最终的 cancelled 状态解释为 probe 失败。
  - compile-only runner 没有交互桌面，未执行生成的 probe executable；因此不能证明 ActiveX runtime create/query/destroy、COM apartment、控件/DLL runtime version、WoW64 runtime、connect/disconnect、安装包或 GPUI tab 嵌入行为。
- Windows CI/VM still required:
  - 在有交互桌面的 Windows 10/11 x64 native 环境验证 COM 注册、hidden ActiveX create/destroy、`MsRdpClient12 → IMsRdpClient10` QueryInterface、`IMsRdpClientNonScriptable8` capability、控件/DLL 实际版本和最小 connect/disconnect。
  - 在 Windows 10/11 x64 WoW64 下运行 x86 probe/Navop，验证 32-bit ATL、32-bit COM 注册表视图、安装后 create/query/connect/disconnect；Windows 10 x86 OS 是否纳入仍按产品支持策略决定。
  - 在缺失 ATL component 的干净 Windows runner/VM 记录预期 Red，并在执行安装脚本后重复 probe，证明安装步骤的真实幂等性；当前 macOS 环境没有 `pwsh`，因此没有伪造 PowerShell parser 或 installer 结果。
  - 确认 Navop Windows UI thread 的实际 COM apartment，并决定是否需要显式 STA 初始化；GitHub-hosted runner 的 compile-only 成功也不能替代该 UI/runtime 验证。
- Capability decisions/degradations:
  - `MsRdpClient12` 只按其公开主接口 `IMsRdpClient10` 使用；禁止生成或引用不存在的 `IMsRdpClient11/12`。若 CoClass 不可创建，probe 必须报告 unavailable/class-not-registered；fallback 策略留给后续 Task。
  - `IMsRdpClientNonScriptable8` 是可选 capability；低版本系统允许 QueryInterface unavailable，不因此伪装成主控件创建成功。
  - Microsoft 公开 API 将 `NotifyTSPublicKey` 标记为不受支持，因此 Navop 不承诺自定义 TS server public-key SHA-256 pin decision；证书信任继续由系统 RDP 安全模型管理，UI 不得暗示存在自定义 pin。
  - 已证明的公开 clipboard 配置边界是 `RedirectClipboard`；`ManualClipboardSyncEnabled`/`IMsRdpClipboard` 不能证明“保留文本但单独禁用文件/目录 clipboard”。首个 GA 将文本和文件 clipboard 视为同一安全开关，不提供虚假的独立 file clipboard toggle。
  - Restricted Admin、Remote Credential Guard、admin/console 和 child-session 相关属性只作为 capability-gated enterprise 选项，必须等待目标 SDK、OS 和服务器行为验证；不猜测或承诺 arbitrary session-id attach。
- API proof references:
  - [IMsRdpClientAdvancedSettings interface](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclientadvancedsettings-interface)：其 `NotifyTSPublicKey` 属性条目明确标记为不受支持，因此首个 GA 不提供自定义 TS public-key pinning。
  - [IMsRdpClientAdvancedSettings5::RedirectClipboard](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclientadvancedsettings5-redirectclipboard)：公开的 clipboard redirection 总开关。
  - [IMsRdpExtendedSettings::Property](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpextendedsettings-property) 与 [IMsRdpClipboard](https://learn.microsoft.com/en-us/windows/win32/api/mstscax/nn-mstscax-imsrdpclipboard)：公开手动同步能力不能证明“保留文本 clipboard、单独禁用文件/目录 clipboard”，因此 UI 不暴露独立 file clipboard toggle。
  - [IMsRdpExtendedSettings::Property](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpextendedsettings-property) 与 [IMsRdpClientAdvancedSettings6::ConnectToAdministerServer](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclientadvancedsettings6-connecttoadministerserver)：仅作为 Restricted Logon、Redirected Authentication、child/admin session 等 enterprise capability 的属性依据；实际可用性仍须按 SDK、OS 和服务器逐项验证。

**Acceptance:**

- x64/x86 都能编译。
- Windows 10/11 x64 native 进程和 Windows 10/11 x64 WoW64 下的 x86 进程都完成安装后 probe、创建/销毁和最小 connect/disconnect；Windows 10 x86 OS 是否纳入按产品支持策略记录。
- toolchain 安装步骤可重复。
- 分架构记录 COM 注册视图、可用接口、控件版本和 unavailable 诊断。
- 证书 public-key pin、文件 clipboard 和 enterprise auth 的可实现边界有书面结论。

**Failure gate:** 如果 ATL 无法稳定进入现有构建，评估独立静态 native library；如果系统控件在受支持 Windows 上普遍不可创建，停止后续实施并保留 canvas。

**Rollback:** 删除 probe/spike 和未接入的 toolchain 变更，保持 feature off；不得为通过 spike 而分发私有 mstscax DLL。

---

### Task 2: 创建 `windows_rdp_host` crate 与版本化 C ABI

**Goal:** 建立可测试、可审计、与 GPUI 解耦的 native host 边界。

**Depends on:** Task 1。

**Status:** 进行中。ABI v1 lifecycle prefix、event/callback owned-queue、
credentials/zeroize transport、native callback dispatch/quiescent gate 和 Rust facade
lifecycle observability 已实现；真实 COM event sink/event-source detach、ActiveX
credential setter 和完整 Task 2 unsafe/lifecycle review 尚未完成。

**Files:**

- Create: `crates/windows_rdp_host/Cargo.toml`
- Create: `crates/windows_rdp_host/build.rs`
- Create: `crates/windows_rdp_host/src/lib.rs`
- Create: `crates/windows_rdp_host/src/ffi.rs`
- Create: `crates/windows_rdp_host/src/handle.rs`
- Create: `crates/windows_rdp_host/src/options.rs`
- Create: `crates/windows_rdp_host/src/event.rs`
- Create: `crates/windows_rdp_host/src/capabilities.rs`
- Create: `crates/windows_rdp_host/src/error.rs`
- Create: `crates/windows_rdp_host/native/windows_rdp_host.h`
- Create: `crates/windows_rdp_host/native/event_dispatch.cpp`
- Create: `crates/windows_rdp_host/src/native_tests.rs`
- Create initial C++ source files under `crates/windows_rdp_host/native/`
- Modify: root `Cargo.toml`

**Produces:**

- Safe `WindowsRdpHost` facade。
- `NativeRdpHost*` opaque handle。
- ABI version/size contract。
- `probe/create/destroy`。
- owned Rust event bridge。
- deterministic error type。

- [x] **Red:** 写 ABI layout test，覆盖 Rust/C++ struct size、alignment、固定 32-bit result width 和 `abi_version` mismatch。
- [x] **Red:** 写 fake bindings 测试 create failure、null handle、double destroy 和 native 未清空 handle。
- [x] **Red:** 写 callback-after-close、stale generation 和 owned payload 测试。
- [x] **Red:** 接入真实 COM event sink 前，写 native callback dispatch、callback reentrancy、in-flight close rejection/retry 和 owner-thread quiescence 测试。
- [x] **Red:** 写 server/Gateway secret borrowed-until-return、OOM/内部失败/重复 apply、两端 zeroize contract 和禁止新增 full-memory dump collection 的测试。
- [ ] **Red:** 在真实 ActiveX credential setter/connect 引入异步或可取消流程时，补取消竞态、setter 部分成功和 COM 内部副本边界测试。
- [x] **Green:** 使用 `cc::Build` 编译 C++17 shim；非 Windows target 完全不运行 build。
- [x] **Green:** 实现 RAII Rust wrapper，`Drop` 只调幂等 destroy。
- [x] **Green:** C++ 入口捕获异常并转换为固定 result。
- [x] **Green:** callback 转成 owned event queue，不从 callback 直接访问 GPUI。
- [x] **Green:** 实现独立的一次性 credential apply API，server 和 Gateway secret 不进入普通 connect options。
- [ ] **Refactor:** 将 lifecycle、event、error 分文件，保持文件/函数复杂度约束。
- [ ] **Review:** 对所有 `unsafe` 添加 safety invariant；检查 pointer lifetime、string length、thread ownership。
- [x] **Verify（第一最小切片）:** 通过本地 crate/contract tests、C/C++ warning-as-error 配置，以及 GitHub Windows x64/x86 compile/link-only gate。

**Execution Notes (2026-08-07) — 第一最小切片：**

- Red evidence:
  - ABI contract 首先固定 `NAVOP_RDP_ABI_VERSION == 1`、`NavopRdpResult == int32_t`、三个 versioned struct 的 size/alignment/field offset、generation low/high 拆分和 deterministic result code mapping。
  - fake bindings 覆盖 create failure、native 返回 success 但 null handle、close/Drop 重复 destroy、destroy failure，以及 native 返回 success 但未清空 handle。
  - contract review 发现组合校验 helper 可能在 `struct_size` 不足时提前读取 `abi_version`；实现已改为先校验 size，再读取 version，并增加源码 contract 防回归。
  - callback-after-close、owned event、stale generation、server/Gateway secret、credential apply 和 zeroize 仍保持 Red/未实现，不能由当前 lifecycle tests 代替。
- Green implementation:
  - 新增 `crates/windows_rdp_host` workspace crate，提供 `WindowsRdpHost`、`WindowsRdpHostOptions`、`WindowsRdpHostCapabilities`、`WindowsRdpHostError` 和 opaque `NativeRdpHost*` facade。
  - 当前冻结的 ABI v1 只包含 `probe/create/destroy`；所有 structs 使用 `struct_size` offset 0、`abi_version` offset 4 的 append-only prefix。
  - `NavopRdpCreateOptions` 将 generation 拆成 `generation_low`/`generation_high`，避免在 C ABI 中引入 x86/x64 不同的 `uint64_t` alignment。
  - `struct_size >= sizeof(current layout)` 被定义为前向兼容：只访问当前已知前缀，probe 保留调用方输出 size，不触碰未知尾部。
  - create 对可写 `out_host` 先置空，使用 `new (std::nothrow)` 并映射 allocation failure；probe/create/destroy 全部捕获 C++ 异常并返回固定 result。
  - destroy 对 null handle 幂等，成功路径先清空调用方 pointer 再释放对象；Safe Rust `close()` 额外拒绝“native 返回成功但未清空 handle”。
  - Rust facade 使用 `PhantomData<Rc<()>>` 保持 `!Send + !Sync`，为后续 STA/COM owner-thread contract 预留正确默认。
  - 非 Windows stub 不运行 `cc::Build`，probe 稳定返回 `available = false`，create 返回 `Unavailable`，并保持与 native 相同的 size/version 校验顺序和输出 size 语义。
- Refactor/review:
  - 当前代码按 `ffi`、`handle`、`options`、`capabilities`、`error` 分离；`event.rs` 和独立 lifecycle/event queue 尚未创建，因此 Task 2 的完整 Refactor 项仍未勾选。
  - 两轮独立只读审查未发现本切片阻断问题；审查意见已用于增加所有字段的 C++ `offsetof` static assertions、扩展 struct 注释、destroy failure/non-clearing fake tests 和更严格的 CI contract。
  - `Drop` 无法返回 destroy error，当前平凡 native object 的 destroy 会先清空再 delete；引入 COM/event sink/in-flight callback 后必须重新审查销毁错误、callback quiescence 和资源释放保证。
- Automated verification:
  - Commit：`984e921d7b005c8175914ecef50674c3b3e45097`。
  - `rtk cargo fmt --all --check`：通过。
  - `rtk cargo clippy --locked -p windows_rdp_host --all-targets -- -D warnings`：通过。
  - `rtk cargo test --locked -p windows_rdp_host`：18 passed。
  - `rtk cargo test --locked -p windows-rdp-probe --test contract`：11 passed。
  - `rtk git diff --check`：通过。
- GitHub-hosted Windows compile/link verification:
  - Workflow run：[`31176321867`](https://github.com/feigeCode/navop/actions/runs/31176321867)，head SHA 为 `984e921d7b005c8175914ecef50674c3b3e45097`。
  - x64 job：[`Windows RDP probe (x86_64-pc-windows-msvc)` / `92859007370`](https://github.com/feigeCode/navop/actions/runs/31176321867/job/92859007370)，成功编译并链接 `windows-rdp-probe` 与 `windows_rdp_host` test executable。
  - x86 job：[`Windows RDP probe (i686-pc-windows-msvc)` / `92859007371`](https://github.com/feigeCode/navop/actions/runs/31176321867/job/92859007371)，成功编译并链接 `windows-rdp-probe` 与 `windows_rdp_host` test executable。
  - 两个目标 job 完成后，为节省 runner 主动取消了该 workflow 的其他无关 job，因此 workflow 最终 conclusion 为 `cancelled`；这不表示上述两个目标 job 失败。
  - Windows gate 实际执行 locked probe build 和 `windows_rdp_host --no-run`。它证明 MSVC C++ static library、Rust FFI symbols、x64/i686 ABI 能完成 compile/link，不运行 test executable，也不证明 ActiveX runtime。
- Manual verification:
  - 当前开发机是 macOS；本切片没有在本机执行 Windows binary。
  - 本切片不创建 COM apartment、ATL AxWin、`MsRdpClient12`、event sink、child `HWND` 或 RDP session，因此没有可声称通过的 ActiveX create/query/show/connect runtime smoke。
- Known limitations:
  - 当前 `NativeRdpHost` 只保存 lifecycle generation metadata；它不是 ActiveX host，也不是 framebuffer backend。
  - generation 尚未进入 callback/event filtering；没有 explicit `Closing` callback gate、owned event queue、in-flight callback drain 或 callback-after-close 防护。
  - 没有 server/Gateway secret 类型、credential apply、zeroize 或 crash/full-memory-dump policy 实现。
  - `windows_rdp_host` 尚未接入 `RemoteDesktopView`、presentation factory、parent `HWND`、backend fallback 或 GPUI tab。
- Decision changes:
  - 早期草案中的“`abi_version` 首字段”改为固定的 `struct_size`/`abi_version` 双字段 prefix；后续字段 append-only。
  - 早期草案中的 ABI `uint64_t generation` 改成两个 `uint32_t` 字段，Rust/C++ 内部再重组为 `u64`。
  - parent `HWND`、callback、connect、credentials、bounds 和 settings 不提前加入 lifecycle prefix；它们在后续切片中通过独立 versioned contract 演进。

**Execution Notes (2026-08-07) — 第二最小切片：event/callback bridge：**

- Red evidence:
  - 新增 fake native callback bindings，覆盖 callback 注册失败、同步注册回调、按序 owned event、stale generation、unregister 期间 callback、callback-after-close、malformed payload、unregister-before-destroy，以及 unregister/destroy 可重试错误路径。
  - ABI contract 冻结 `NavopRdpEvent` 和 `NavopRdpEventCallbackOptions` 的固定宽度 layout；C++ `static_assert` 与 Rust const assertions 分别覆盖 x64/x86 size、alignment 和 field offset。
  - callback reentrancy 和真实 in-flight callback drain 尚未进入自动化测试；该项必须在 COM event sink 开始 dispatch native events 前完成，不能由当前 fake callback tests 代替。
- Green implementation:
  - 新增 `EventBridge`，callback 只校验 versioned event prefix、复制 borrowed payload、按 generation 过滤并写入 owned Rust queue，不访问 GPUI entity。
  - Rust host 使用 `Open -> Closing -> Closed` gate；关闭时先关闭 Rust queue，再成功 unregister native callback，最后 destroy opaque handle。
  - native shim 保存 callback/context 并提供注册/注销 ABI；注册失败不保留 callback/context，destroy 统一先关闭 callback gate。
  - callback payload、event pointer 和 callback context 均只在 ABI 约定的同步调用生命周期内读取；Rust callback 使用 `catch_unwind` 防止 panic 穿过 C ABI。
- Lifecycle/review:
  - 当前 native shim 不 dispatch 任何 native event，因此成功 unregister 时没有真实 in-flight callback；现有 quiescence contract 在本切片中只依赖这一限制成立。
  - 接入真实 COM connection-point sink 前，必须增加 event-source detach/Unadvise、in-flight callback 计数、close gate 和 drain/wait，并重新审查 callback reentrancy、owner-thread/STA 约束和 COM reference cycle。
  - `Drop` 在 unregister 永久失败时会保守泄漏 `EventBridge`，避免 native 保留的 callback context 变成悬垂指针；registration cleanup 或 destroy 永久失败也可能保留 native allocation。后续完整 lifecycle review 必须决定可观测错误与最终释放策略。
  - `event.rs` 和 `error.rs` 已分离；lifecycle 仍位于 `handle.rs`，所以完整 Refactor checklist 暂不勾选。
- Automated verification:
  - Commit：`b21671a709c923637f109eb129f6476be64d8a5b`。
  - `cargo fmt --all -- --check`：通过。
  - `cargo clippy --locked -p windows_rdp_host --all-targets -- -D warnings`：通过。
  - `cargo test --locked -p windows_rdp_host`：unit tests 26 passed，contract tests 9 passed。
  - `cargo test --locked -p windows-rdp-probe --test contract`：11 passed。
  - `git diff --check`：通过。
  - GitHub workflow run：[`31181437917`](https://github.com/feigeCode/navop/actions/runs/31181437917)，head SHA 为 `b21671a709c923637f109eb129f6476be64d8a5b`。
  - x64 job：[`Windows RDP probe (x86_64-pc-windows-msvc)` / `92875420648`](https://github.com/feigeCode/navop/actions/runs/31181437917/job/92875420648)，成功完成 locked probe build 和 `windows_rdp_host --no-run` compile/link。
  - x86 job：[`Windows RDP probe (i686-pc-windows-msvc)` / `92875420645`](https://github.com/feigeCode/navop/actions/runs/31181437917/job/92875420645)，成功完成 locked probe build 和 `windows_rdp_host --no-run` compile/link。
  - 两个目标 job 成功后，为节省 runner 主动取消了该 workflow 的其他无关 job，因此 workflow 最终 conclusion 为 `cancelled`；这不表示上述两个目标 job 失败。
  - 该证据证明 MSVC/ATL C++、Rust/C++ FFI symbols、callback ABI 和 x64/i686 layout assertions 可编译链接；它不运行 test executable，也不证明 ActiveX runtime、COM apartment、child `HWND`、show/hide/focus、RDP connect 或 GPUI tab 嵌入。
- Manual verification:
  - 当前开发机是 macOS；本切片不创建 COM apartment、ATL AxWin、`MsRdpClient12`、event sink、child `HWND` 或 RDP session。
  - GitHub-hosted runner 只能证明 MSVC/ATL C++、Rust FFI symbols 和 x64/i686 ABI 完成 compile/link；不能替代有交互桌面的 Windows ActiveX runtime smoke。
- Remaining Task 2 scope:
  - credentials/zeroize transport 已在下一最小切片补齐；真实 ActiveX property
    setter、异步 connect/cancel 语义、COM/ActiveX 内部副本边界和完整 crash/full-memory
    dump policy 仍未实现。
  - 真实 COM event sink、callback in-flight drain 和完整 lifecycle review 尚未实现。

**Execution Notes (2026-08-07) — 第三最小切片：credentials/zeroize transport：**

- Red evidence:
  - 首先新增 contract test 冻结独立的 `NavopRdpBorrowedSecret` /
    `NavopRdpCredentialBundle`、server/Gateway 双字段、UTF-16 code-unit length、
    borrowed-until-return lifetime、x64/x86 layout 和 versioned apply entrypoint。
  - 首次定向执行
    `cargo test --locked -p windows_rdp_host --test contract credential_transport_is_versioned_borrowed_and_architecture_specific -- --exact`
    按预期失败，原因是 public header 尚不存在 `NavopRdpBorrowedSecret`；实现是在该
    Red evidence 之后加入。
  - fake bindings 覆盖 server-only、Gateway-only、双 secret 且值不同、空 bundle、
    重复 apply、allocation/internal failure mapping、failure 后 lifecycle 保持 Open，
    以及 Closing/Closed 在调用 native 前拒绝。
- Green implementation:
  - 新增不可 `Clone`/serde 的 `WindowsRdpCredentialBundle`。server password 和
    Gateway password 分别由 `Zeroizing<Vec<u16>>` 持有；接管的 UTF-8 `String`
    在 UTF-16 编码后也由 `Zeroizing` 清理。
  - 自定义 `Debug` 只暴露 absent/redacted 和 UTF-16 code-unit 数，不输出 secret。
    credential 不进入 `WindowsRdpHost` 字段、普通 options、event、error、log、
    panic message、snapshot 或 telemetry。
  - Rust 只在同步 `apply_credentials` 调用期间建立 borrowed UTF-16 C ABI view；
    `len` 是 code units，`len == 0` 使用 null pointer，`len > 0` 要求 non-null，
    native 不得保留 Rust pointer。
  - C++ 在完成 null、`struct_size`、`abi_version`、flags、lifecycle 和两个 borrowed
    slice 的校验后，分别建立 `SensitiveUtf16Buffer` scratch copy；析构通过
    `SecureZeroMemory` 后 `delete[]`，覆盖成功、第二次分配失败和 C++ 异常展开路径。
  - `NativeRdpHost` 私有定义移入 `host_internal.h`，public header 仍只暴露 opaque
    handle；所有 C++ ABI 入口继续保持 `noexcept`/异常转固定 result。
- Automated verification:
  - `cargo fmt --all -- --check`：通过。
  - `cargo clippy --locked -p windows_rdp_host --all-targets -- -D warnings`：通过。
  - `cargo test --locked -p windows_rdp_host`：unit tests 35 passed，contract tests
    13 passed。
  - `cargo test --locked -p windows-rdp-probe --test contract`：11 passed。
  - `git diff --check`：通过。
  - 两轮独立只读审查未发现本切片阻断问题；确认 Rust owner/borrow lifetime、
    C++ RAII wipe、x64/x86 layout、C linkage 和 Open/Closing/Closed gate 一致。
- GitHub-hosted Windows compile/link verification:
  - Workflow run：[`31184912814`](https://github.com/feigeCode/navop/actions/runs/31184912814)，
    head SHA 为 `4cb2ccf26a7b614d11ae06e814f3c97a7752a11c`。
  - x64 job：[`Windows RDP probe (x86_64-pc-windows-msvc)` / `92886928118`](https://github.com/feigeCode/navop/actions/runs/31184912814/job/92886928118)，
    成功完成 locked probe build 和 `windows_rdp_host --no-run` compile/link。
  - x86 job：[`Windows RDP probe (i686-pc-windows-msvc)` / `92886928091`](https://github.com/feigeCode/navop/actions/runs/31184912814/job/92886928091)，
    成功完成 locked probe build 和 `windows_rdp_host --no-run` compile/link。
  - 两个目标 job 成功后，为节省 runner 主动取消其余无关 job，因此 workflow 最终
    conclusion 为 `cancelled`；这不表示上述两个目标 job 失败。
  - 该证据证明 MSVC C++17 `/W4 /WX`、`credential.cpp`、Rust/C++ FFI symbols、
    `SecureZeroMemory` linkage 和 x64/i686 ABI assertions 可编译链接；它不运行 test
    executable，也不证明 native scratch runtime、ActiveX setter、COM apartment、
    child `HWND`、RDP connect 或 GPUI tab 嵌入。
- Security boundary and known limitations:
  - 本切片是 transport-only；`credential.cpp` 明确不调用 `ClearTextPassword`、
    Gateway setter、BSTR、ATL 或 ActiveX。真实 server/Gateway property application
    属于后续 ActiveX 切片。
  - 本切片保证 Rust owner 和 native scratch 按既定 ownership/RAII contract 清理，
    但不声称能清除 allocator、OS、未来 COM/ActiveX 内部或进程其他位置的所有副本。
  - 仓库当前没有完整 crash/full-memory-dump scrub policy。本切片只验证没有新增
    `MiniDumpWriteDump`、full-memory dump 或把 credential 放入普通输出；完整
    crash-dump policy 仍是后续安全工作。
  - C ABI 调用者必须提供在声明长度内可读的 borrowed pointer。普通 C++ `catch (...)`
    不保证捕获非法 pointer 引发的 Windows SEH access violation；safe Rust facade
    只生成指向其活跃 owner buffer 的 pointer。
  - 真实 native OOM/fault injection、第二个 scratch 分配失败后的内存观察、异常路径
    内存观察和 future event payload 脱敏策略尚未由 Windows runtime test 覆盖。
  - 同步 transport 当前没有 cancellation 状态；取消竞态应在真实 setter/connect
    出现异步或部分提交语义时设计并测试，不能用当前 fake result 冒充已经验证。

**Execution Notes (2026-08-07) — 第四最小切片：native callback dispatch/quiescent gate：**

- Red evidence:
  - contract test 首先要求 public callback contract 明确禁止 callback 同步重入
    `NativeRdpHost` entrypoint，并要求 unregister 成功只能发生在 callback gate
    quiescent 之后；首次完整 crate 测试为 unit 35 passed、contract 12 passed /
    2 failed。
  - Windows-only native tests 覆盖 callback 单次同步 dispatch、callback 内重入
    unregister/destroy、错误返回后 handle/callback 保留、wrong-thread dispatch /
    unregister/destroy，以及 event size/version/reserved/generation/payload pointer
    校验。非法 event 不得调用 callback，后续合法 event 仍必须成功。
- Green implementation:
  - `NativeRdpHost` 记录创建线程 ID 和 `callbacks_in_flight`；所有 callback lifecycle、
    credential 和 dispatch entrypoint 在改变状态前校验 owner thread。
  - `event_dispatch.cpp` 在保留 callback/context 前完成 event header、generation、
    payload、gate 和 counter overflow 校验，并用 RAII scope 包围同步 callback，
    保证正常返回或 C++ 异常展开时 in-flight counter 都恢复。
  - unregister/destroy 在 callback in flight 时不阻塞，返回
    `NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT`；callback、context、host 和调用方 handle
    均保持不变，可在 callback 返回后的后续 owner-thread turn 重试。
  - test-only `navop_rdp_test_dispatch_event` 不进入 public header；真实 COM event
    source、connection-point `Unadvise` 和 drain/timeout 仍属于后续切片。
  - Windows probe gate 保留 probe executable 的 compile/link-only 边界，但
    `windows_rdp_host` 已从 `cargo test --no-run` 改为实际执行非 ActiveX native
    host tests。
- Automated verification:
  - `cargo fmt --all -- --check`：通过。
  - `cargo clippy --locked -p windows_rdp_host --all-targets -- -D warnings`：通过。
  - `cargo test --locked -p windows_rdp_host`：unit tests 35 passed，contract tests
    14 passed。
  - `cargo check --locked -p windows_rdp_host`：通过。
  - `RUSTFLAGS='--cfg windows_rdp_host_native' cargo check --locked -p
    windows_rdp_host --tests`：通过；仅额外 type-check Windows-only Rust test module，
    不编译或链接 MSVC C++。
  - `git diff --check`：通过。
  - 独立只读审查未发现本切片的 correctness、ABI、memory/thread safety 阻断问题。
- Windows verification boundary:
  - 当前开发机是 macOS，不能编译或运行 MSVC C++ native tests，也没有 PowerShell；
    上述本地结果只覆盖 Rust/fake/源码 contract。
  - x64/i686 Windows runner 仍必须执行
    `./script/build-windows-rdp-probe.ps1 -Target
    x86_64-pc-windows-msvc,i686-pc-windows-msvc`，确认 `/W4 /WX` compile/link 和
    native test executable 的真实结果。
- 即使该 runner 通过，也不证明 ActiveX create/query/connect、COM apartment、
  child `HWND`、GPUI tab 嵌入或有交互桌面的 runtime smoke。

**Execution Notes (2026-08-07) — 第五最小切片：credential native runtime contract：**

- Red evidence:
  - 现有 Rust/fake 与源码 contract 已覆盖 credential layout、校验顺序、独立的
    server/Gateway borrowed UTF-16 和 RAII wipe 文本，但 Windows-only native test
    模块没有实际调用 `navop_rdp_apply_credentials`。
  - 因此补充 native runtime matrix，不能把 Rust fake 的 `RESULT_OK` 误报成
    C++ scratch copy 已在 Windows 执行；也不把 `RESULT_OK` 解释成 ActiveX
    `ClearTextPassword` 或 Gateway setter 已成功。
- Green implementation:
  - `src/native_tests.rs` 新增 native credential ABI declaration 和 helper，使用
    `size_of::<NavopRdpCredentialBundle>()`，不硬编码 x64/x86 bundle size。
  - native tests 覆盖空 bundle、server-only、Gateway-only、双 secret 的同步成功
    路径；null host/credentials、尺寸不足、ABI mismatch、非零 flags、server/Gateway
    null pointer 加非零长度的拒绝路径；wrong-thread 拒绝后 owner-thread 仍可 apply；
    unregister 关闭 callback gate 后 apply 被拒绝；每条路径在 owner thread 清理 host。
- Refactor/review:
  - 测试只借用仍存活的 UTF-16 backing arrays 到同步 native 调用返回，不把 borrowed
    pointer 跨调用保存；不尝试观察不可由当前 ABI 导出的 `SecureZeroMemory` 内容。
  - 保持本切片 transport-only，不引入 BSTR、ATL、COM、`ClearTextPassword`、
    Gateway setter、`Connect` 或 Task 3 child `HWND`。
- Automated verification:
  - macOS 可执行现有 Rust/fake/contract tests；Windows-only native tests 仅在
    Windows MSVC target 编译/运行。
  - 本机只执行格式化、普通 crate tests、Clippy、cargo check、cfg native test
    type-check 和 diff check；Windows x64/x86 runner 仍需实际执行 native test
    executable，当前没有伪造其结果。
- Manual verification:
  - 当前开发机没有 Windows MSVC、Windows SDK 或 PowerShell，因此没有声称本切片
    已通过 Windows native runtime。
- Known limitations:
  - 未注入 native OOM，未观察 allocator/OS/未来 COM 内部副本的清除；真实
    ActiveX setter、COM event sink/Unadvise/drain 和完整 lifecycle review 仍未完成。

**Execution Notes (2026-08-07) — 第六最小切片：Rust facade lifecycle observability：**

- Red/review evidence:
  - `WindowsRdpHost::is_closed()` 无法区分仍可接收 callback/credential 的 `Open`，
    与 callback gate 已关闭但 unregister/destroy 仍需重试的 `Closing`；该差异会影响
    后续 owner-thread shutdown 调度和失败可观测性。
  - 现有 fake tests 已覆盖 unregister/destroy failure retry，但没有直接冻结
    `Open -> Closing -> Closed` 的公开状态序列，也没有覆盖 `Drop` 连续 unregister
    失败时 callback context 必须继续存活的保守 ownership contract。
- Green implementation:
  - 新增公开只读 `WindowsRdpHostLifecycle::{Open, Closing, Closed}` 和
    `WindowsRdpHost::lifecycle()`；状态只描述 Rust facade ownership/callback admission，
    不声称 ActiveX control 或 RDP session 达到相同状态。
  - lifecycle state type 移入独立 `lifecycle.rs`；`close()` 仍在第一次尝试时关闭
    event gate，失败后保持 `Closing`，成功 unregister/destroy 且 native 清空 handle
    后才进入 `Closed`。
  - 新增 fake regression：当显式 close 和 `Drop` 的 unregister 都失败时，不调用
    destroy，并保守泄漏 `EventBridge`，使 fake native 保留的 callback/context 在
    host drop 后仍可安全同步调用；该行为优先避免 dangling callback context。
- Verification boundary:
  - 本切片只改变 Rust facade、fake tests 和静态 contract，不改变 C ABI/C++ native
    实现，不引入 COM、ATL、ActiveX、`HWND` 或 Windows runtime 声明。
  - macOS 可完整执行该状态机/fake ownership contract；Windows native callback
    source、connection-point `Unadvise`、真实 in-flight drain/timeout 和永久失败后的
    process-level observability 仍属于后续 lifecycle/COM 工作。

**Execution Notes (2026-08-07) — 第七最小切片：cleanup ownership matrix：**

- Red/review evidence:
  - 现有 fake tests 分别覆盖 unregister 或 destroy 的单一失败重试，但没有冻结
    “第一次 unregister 失败、第二次 unregister 成功后第一次 destroy 失败、第三次
    close 完成 destroy”的完整调用序列，也没有断言第二次 unregister 成功后 native
    callback/context 已被清除。
  - registration 失败后的 cleanup 已覆盖 destroy 返回错误，但没有覆盖 destroy 返回
    `OK` 却不清空 opaque handle 的错误实现。
- Green implementation:
  - 新增 cleanup ownership matrix fake regression：第一次 unregister 失败时保持
    `Closing`、不调用 destroy，Rust callback gate 不重新打开；第二次 unregister 成功
    后释放 native callback/context，但 destroy 失败仍保持 `Closing`；后续 close 只重试
    destroy，最终清空 handle 后进入 `Closed`。
  - 新增 registration failure + non-clearing destroy regression：保留原始
    registration error，记录 `register -> destroy` 顺序，并冻结“不确定 native
    handle 是否已释放时保守泄漏 native allocation”的 policy。
  - 强化 `handle.rs` unsafe contract comments，并在 Rust contract tests 中检查
    unregister 成功后的 callback/context quiescence、registration 失败不保留 callback/
    context、原始错误优先级及 `Box::leak(event_bridge)` 的保守 Drop policy。
- Verification boundary:
  - 本切片仍只改变 Rust facade fake backend、source contract 和计划文档；不声称已
    实现真实 COM connection-point `Advise/Unadvise`、异步 callback drain/timeout、
    ActiveX setter 或 Task 3 child `HWND`。
  - macOS 可以执行全部 38 个 Rust unit tests 与 14 个 contract tests；Windows-only
    native C++ runtime、ATL/COM/ActiveX 以及交互桌面 smoke 仍需 Windows 环境。

**Acceptance:**

- fake backend 可在无真实 RDP server 下覆盖 ABI 和生命周期。
- double destroy 不崩溃。
- 错误和异常不跨 ABI。
- server/Gateway secret 在 Rust/C++ 成功和失败路径都按 contract 清零。
- feature off 不编译任何 C++。

**Rollback:** crate 尚未接 UI，移除 workspace member 和 feature dependency 即可。

---

### Task 3: 创建 child HWND 与空 `MsRdpClient12` 控件

**Goal:** 在指定 parent `HWND` 下创建不可见的真实 ActiveX child，并支持 bounds/show/hide/focus/destroy。

**Depends on:** Task 2。

**Files:**

- Modify: `crates/windows_rdp_host/native/host.cpp`
- Modify: `crates/windows_rdp_host/native/lifecycle.cpp`
- Modify: `crates/windows_rdp_host/native/windows_rdp_host.h`
- Modify: `crates/windows_rdp_host/src/handle.rs`
- Add Windows integration tests

**Implementation outline:**

1. assert current thread/apartment；
2. `AtlAxWinInit`；
3. 创建 `WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS` host window；
4. 以零 bounds/hidden 状态创建；
5. `AtlAxCreateControlEx` 创建 `MsRdpClient12`；
6. query `IMsRdpClient10`；
7. 设置 `UIParentWindowHandle`；
8. 返回 capabilities；
9. reverse-order cleanup。

- [ ] **Red:** create with invalid/null parent 返回结构化错误且无 HWND 泄漏。
- [ ] **Red:** wrong-thread 操作被拒绝。
- [ ] **Red:** 连续 create/destroy 100 次后 USER/GDI/COM 资源不持续增长。
- [ ] **Green:** 实现 hidden child + empty control。
- [ ] **Green:** 实现 `set_bounds`、`set_visible`、`focus`。
- [ ] **Green:** 实现所有中途失败的逆序释放。
- [ ] **Refactor:** host window 和 COM control ownership 分离，避免循环引用。
- [ ] **Review:** 检查 window style、parent owner、DPI context、零尺寸首帧。
- [ ] **Verify:** 真机执行 create/show/hide/resize/destroy smoke。

**Acceptance:**

- 控件只出现在指定 child 区域。
- 创建时不闪现。
- 隐藏后不覆盖其他内容。
- parent 关闭时可安全销毁。

**Rollback:** native presentation 仍未启用，禁用 feature 即回到 canvas。

**Execution Notes (2026-08-07) — 第一垂直切片：borrowed parent 与 hidden ActiveX host：**

- Red evidence:
  - 增加独立 versioned `create_with_parent` ABI 的 layout、invalid parent、ABI mismatch、
    wrong-thread、null handle、native failure 与 Rust facade forwarding 覆盖。
  - Windows-only native tests 覆盖错误线程不返回 host，且 caller-owned parent 仍由 caller
    销毁；macOS 使用 `--cfg windows_rdp_host_native` 仅 type-check Rust 侧测试。
- Green implementation:
  - 新增 `uintptr_t`/`usize` borrowed parent ABI；parent 为 caller-owned、non-owning，
    host 仅拥有自身创建的 child。
  - 创建 hidden、zero-sized `AtlAxWin` child，使用
    `AtlAxCreateControlEx` 创建空 `MsRdpClient12`，并 query `IMsRdpClient10`。
  - partial-create failure 与 host destroy 通过 RAII 依次销毁 child window、释放 COM
    references、终止 ATL/OLE 初始化。
- Refactor/review:
  - `NativeRdpHost` 继续只向 Rust 暴露 opaque pointer，ActiveX resources 独立封装。
  - versioned prefix 在读取 `parent_hwnd` 前先校验 `struct_size` 与 ABI version，避免短
    caller layout 的越界读取。
- Automated verification:
  - `cargo fmt --all -- --check`
  - `cargo test --locked -p windows_rdp_host`
  - `cargo clippy --locked -p windows_rdp_host --all-targets -- -D warnings`
  - `RUSTFLAGS='--cfg windows_rdp_host_native' cargo check --locked -p windows_rdp_host --tests`
  - `git diff --check`
- Manual verification:
  - 当前 macOS 环境没有 MSVC、ATL、Windows COM registry 或真实 `HWND`，未执行
    Windows runtime smoke。
- Known limitations:
  - Windows MSVC/ATL compile、`#import` type-library resolution、x86/x64 link、
    ActiveX create/query/destroy 与多 host ATL lifetime 仍需 Windows CI/VM 验证。
  - `UIParentWindowHandle`、connect、GPUI integration 与 repeated create/destroy
    resource-leak smoke 留给 Task 3 后续切片。
- Decision changes:
  - 不扩展既有 16-byte `NavopRdpCreateOptions`；parent create 使用独立 ABI version，
    以保持已发布布局不变。

**Execution Notes (2026-08-08) — 第二垂直切片：ActiveX child presentation controls：**

- 新增固定宽度 `NavopRdpBounds` ABI 与 `navop_rdp_set_bounds`、
  `navop_rdp_set_visible`、`navop_rdp_focus` entrypoints；Rust FFI/facade、
  fake bindings、contract tests 与 native lifecycle-only validation 已同步覆盖。
- bounds 使用 parent client-area physical pixels；`x/y` 可为负，`width/height`
  必须非负，zero-sized bounds 合法。
- show 使用 `SW_SHOWNA`，hide 使用 `SW_HIDE`；隐藏前若焦点位于 child 或其
  descendant，best-effort 将焦点交还 caller-owned parent；focus 只接受 visible
  child，并允许最终焦点落在 child descendant。
- 三个 presentation entrypoint 均要求 owner thread 与 `CallbackState::Open`；
  lifecycle-only host 没有 ActiveX resources，因此 presentation 调用返回
  `UNAVAILABLE`；关闭 gate 后返回 `INVALID_ARGUMENT`。
- 本 macOS 环境仅验证 Rust/contract/cfg type-check；Windows MSVC/ATL/ActiveX
  runtime 尚未验证。`UIParentWindowHandle`、connect、DPI/GPUI integration、
  parent rebuild、z-order/overlay、100x create/show/hide/resize/destroy smoke
  尚未完成，Task 3 整体仍未完成。

**Execution Notes (2026-08-08) — 第三垂直切片：ActiveX UI parent window：**

- `UIParentWindowHandle` 属于 `IMsRdpClientNonScriptable2`，不属于
  `IMsRdpClientAdvancedSettings*`；创建流程在 query `IMsRdpClient10` 成功后，
  从同一个 control query `IMsRdpClientNonScriptable2`。
- `put_UIParentWindowHandle` 直接接收已校验的原生 `HWND`；不经过 `LONG`、`long`
  或其他 32-bit 中间值，保持 x86/x64 pointer-width handle 语义。
- UI parent setter 成功后才将 resources 从 `unique_ptr` 转交给 host；interface query
  或 setter 失败会在 ownership transfer 前返回，并沿既有 RAII 路径销毁 child、
  释放 COM references、终止 ATL/OLE。
- contract test 锁定 control/client query、non-scriptable query、UI parent setter、
  HRESULT 检查与 `resources.release()` 的顺序，并继续禁止 host 销毁 caller-owned
  parent 或重挂载 child。
- 本 macOS 环境没有目标 Windows 注册的 MSTSC type library、MSVC/ATL 或真实
  ActiveX runtime；`mstscax.tlh` 的生成签名、Windows compile/link 与对话框 owner
  runtime 行为仍须 Windows CI/VM 验证。
- connect、DPI/GPUI integration、parent rebuild、z-order/overlay 与 100x
  create/show/hide/resize/destroy resource smoke 尚未完成，Task 3 整体仍未完成。

---

### Task 4: host/port 最小连接垂直切片

**Goal:** 用本地测试服务器完成从 Navop options 到 ActiveX `Connect` 的最小连接。

**Depends on:** Task 3。

**Files:**

- Modify: `crates/windows_rdp_host/src/options.rs`
- Modify: `crates/windows_rdp_host/src/handle.rs`
- Modify: `crates/windows_rdp_host/native/configuration.cpp`
- Modify: `crates/windows_rdp_host/native/lifecycle.cpp`
- Add Windows integration tests

**Produces:**

- host、port、初始 width/height、color depth 设置。
- connect/disconnect/request-close。
- 基础 connected state query。

- [ ] **Red:** invalid host、port 0、超长字段和 embedded NUL 被 Rust 层拒绝。
- [ ] **Red:** connect 调用顺序错误（未创建、正在关闭、重复连接）返回状态错误。
- [ ] **Green:** 配置 `Server`、RDP port、`DesktopWidth/Height`、color depth 后调用 `Connect`。
- [ ] **Green:** 实现 graceful `RequestClose` 和 forced `Disconnect` fallback。
- [ ] **Refactor:** connection options 与 security/redirection options 分离。
- [ ] **Review:** 确认日志不打印完整 endpoint credential material。
- [ ] **Verify:** 连接一台测试 Windows host，能够显示登录界面/桌面并正常断开。

**Acceptance:**

- 最小 session 可连接。
- 关闭不会遗留顶层窗口或进程。
- 重复 connect/disconnect 有确定结果。

**Rollback:** connection API 不接入默认 UI；关闭 feature。

#### Execution Notes (2026-08-08) — host/port 最小连接垂直切片

- **Red evidence:** Rust options tests 覆盖 empty host、embedded NUL、port 0、
  UTF-16 code-unit 上限、supplementary Unicode 与无效 desktop dimensions；
  facade fake bindings 覆盖重复/非法 connection state、closing/closed gate、未知 native
  state/status，以及 native result 不得改变 Rust ownership lifecycle。Windows-only native
  tests 还覆盖 connection ABI header/version/flags、host pointer/length、port、dimensions、
  color depth、owner thread、callback gate 和 output initialization。
- **Green implementation:** 新增独立的 `WindowsRdpConnectionOptions` 与 color-depth enum；
  Rust 只在同步 FFI 调用期间持有并借出 UTF-16 endpoint。固定宽度 C ABI 新增
  `NavopRdpBorrowedUtf16`、`NavopRdpConnectionOptions`、connect、connection-state、
  request-close 与 disconnect entrypoints，并冻结 x86/x64 size/alignment/offset。
  native 路径在调用 `Connect` 前按顺序设置 `Server`、RDP port、desktop
  width/height 与 color depth；重复 connect 返回 invalid-state，already-disconnected
  disconnect 返回成功。
- **Refactor/review:** connection options 与 credential/security/redirection policy 保持
  分离；borrowed host 以 length 为权威，不要求 NUL terminator，不使用 `wcslen` /
  `lstrlenW`，不保留 caller pointer，也没有 endpoint logging。Rust host lifecycle
  继续只表达 ownership/callback admission，不冒充 native RDP connection state。
- **Automated verification:** 在 macOS host 运行 `cargo fmt --all -- --check`、
  `cargo test --locked -p windows_rdp_host`、`cargo clippy --locked
  -p windows_rdp_host --all-targets -- -D warnings`、以及
  `RUSTFLAGS='--cfg windows_rdp_host_native' cargo check --locked
  -p windows_rdp_host --tests`；这些验证 Rust facade、FFI declarations、unit/contract
  tests 与 non-native behavior，但不会编译或链接 Windows C++。
- **Manual verification / known limitations:** 当前环境没有 Windows MSVC/ATL、
  注册的 MSTSC type library、真实 ActiveX runtime 或 RDP test server，因此尚未完成
  Windows C++ compile/link、真实 `Connect`、登录界面/桌面、`RequestClose` event、
  disconnect cleanup 或资源 smoke。特别是 `CComBSTR` length constructor、
  advanced-settings interface 获取方式、`ControlCloseStatus` generated 名称和
  `IMsRdpClient10::RequestClose` 签名，仍须 Windows CI/VM 核验；本记录不勾选或
  声称完成上述 runtime acceptance。
- **Decision changes:** Task 4 先交付可审查、可测试的 ABI/facade 最小切片；默认 UI
  接入与真实 session acceptance 保持关闭，等待 Windows build/runtime evidence。

---

### Task 5: event sink、状态映射与错误诊断

**Goal:** 将 `IMsTscAxEvents` 转为稳定 Rust 事件和用户可理解的脱敏错误。

**Depends on:** Task 4。

**Files:**

- Modify: `crates/windows_rdp_host/native/event_sink.cpp`
- Modify: `crates/windows_rdp_host/native/error.cpp`
- Modify: `crates/windows_rdp_host/src/event.rs`
- Modify: `crates/windows_rdp_host/src/error.rs`
- Add event/error contract tests

**Minimum events:**

- connecting/connected/login complete；
- disconnected；
- auto reconnecting/reconnected；
- fatal error/warning/logon error；
- network status changed；
- remote desktop size changed；
- authentication warning displayed/dismissed；
- enter/leave fullscreen；
- focus released；
- confirm close。

- [x] **Red:** fake event sink 顺序、未知 DISPID、未知 error code、callback after generation change。
- [x] **Red:** secret redaction tests 覆盖 password、Gateway password、username 可配置脱敏和完整 endpoint。
- [x] **Green:** 建立 connection point/event sink 并保存 advise cookie。
- [x] **Green:** 映射 HRESULT、disconnect reason、extended reason、logon code，未知值保留 raw code。
- [x] **Green:** Rust callback 投递 UI queue，按 generation 过滤。
- [x] **Green:** destroy 前 `Unadvise`。
- [x] **Refactor:** 映射表从 UI 文案分离，错误本地化在上层完成。
- [x] **Review:** 检查 callback reentrancy 和 COM ref cycle。
- [ ] **Verify:** 人工制造错误密码、拒绝连接、网络中断、服务器重启并核对状态。

**Acceptance:**

- 状态条不会卡在 Connecting。
- 用户能区分认证、证书、Gateway、网络、服务端拒绝和 native unavailable。
- 日志不泄漏秘密。
- event sink 无悬挂 callback。

**Rollback:** 暂不显示高级诊断，但不得绕过 `Unadvise`。

#### Execution Notes (2026-08-08) — ActiveX event sink 与 connection point 切片

- **Green implementation:** 新增 `IDispatch` event sink，通过
  `IConnectionPointContainer::FindConnectionPoint(__uuidof(IMsTscAxEvents))` 建立
  subscription，保存 `Advise` cookie，并把 17 个最小事件映射到既有固定宽度 Rust
  event kind/schema。`DISPPARAMS::rgvarg` 按 COM 逆序读取，payload 显式 little-endian；
  未知 DISPID 或已知事件的 malformed 参数安全忽略并返回 `S_OK`。
- **Lifetime/reentrancy review:** sink 只持有 non-owning `NativeRdpHost*`，不持有
  control、connection point 或 owning host 引用；资源关系为 host → subscription，
  connection point → sink。destroy 先关闭 callback gate，ActiveX cleanup 再
  `detach()` sink、`Unadvise(cookie)`、释放 connection point/sink，之后才销毁 child
  window 和释放 control。即使 `Unadvise` 失败，sink 已 detach，不再访问 host；
  callback 内重入 unregister/destroy 仍由既有 in-flight gate 拒绝。
- **Native test seam:** Windows-only native tests 通过未写入 public header 的私有入口
  直接调用 `IDispatch::Invoke`，覆盖 known/unknown DISPID、逆序参数、signed raw
  disconnect code、confirm-close by-ref 输出、legacy/modern reconnect、network
  quality，以及 malformed VARIANT 安全忽略。静态 contract 同时冻结
  detach → `Unadvise` → release 和 ActiveX cleanup 顺序。
- **Diagnostic mapping slice:** Rust facade 现已把 Microsoft primary disconnect reason
  与 `ExtendedDisconnectReasonCode` 作为独立 code space 处理；已知 extended category
  优先，未知 extended value 回退 primary reason，所有 signed raw code 均原样保留。
  当前只映射高置信的 user-initiated、authentication、certificate/security、
  server-policy 和 network 类别；无法无歧义落入稳定 public category 的
  server/licensing/internal/protocol 值继续保持 `Unknown`。映射表只产生稳定 category
  与 raw code，不包含 native/UI 文案。Rust callback 集成测试同时覆盖 stale generation
  过滤、queue drain 和 disconnected semantic decode。
- **Production extended reason capture:** production `OnDisconnected` 保持只读取 COM
  event 的一个 primary `VT_I4` 参数，并在同一 owner thread 上同步调用
  `IMsRdpClient::get_ExtendedDisconnectReason`。property getter 成功时，extended raw
  signed code 以 4-byte little-endian payload 投递；getter 失败或 ActiveX resources
  不可用时仍投递 primary disconnect event，不因补充诊断失败而丢失断连通知。sink
  仍只保存 non-owning `NativeRdpHost*`，没有新增 owning client/control/host reference，
  因而不改变既有 COM ownership graph 或 teardown 顺序。
- **Automated verification boundary:** macOS host 已运行 crate tests、Clippy、
  cfg-native Rust test type-check、format check 和 diff check；这些检查不编译 MSVC/
  ATL/type-library C++。head `210c6bb723def7612479eb8fa50b1492fc021c77`
  的 `CI` run `31238248979` 中两个 probe job 均成功：x86_64
  `Windows RDP probe (x86_64-pc-windows-msvc)` / `93054686139`，i686
  `Windows RDP probe (i686-pc-windows-msvc)` / `93054686127`。两个 job 分别执行
  locked `windows-rdp-probe` ATL/MSVC/type-library compile/link 和
  `windows_rdp_host` Windows native host tests，覆盖 production extended-reason
  getter 的双架构编译/链接与 optional payload test；probe executable 本身未运行，
  这些 tests 也不建立 ActiveX RDP session。
- **Still pending at this slice:** HRESULT/logon code 的稳定分类当时尚未补齐；后续
  diagnostic code-space mapping 切片已完成 logon 分类，但 HRESULT 仍待完成。Rust event
  queue 尚未投递到 GPUI UI queue。真实 RDP runtime/manual acceptance（错误密码、拒绝
  连接、网络中断、服务器重启）也未完成。因此 Task 5 整体仍保持未完成；上述
  GitHub-hosted runner 结果只证明 ATL/MSVC/type-library compile/link 和 Windows native
  tests，不得表述为真实 RDP runtime、ActiveX connect/disconnect 或交互桌面验证。

#### Execution Notes (2026-08-08) — owner-thread event reducer contract 切片

- **Dependency boundary:** workspace 现在把 `windows_rdp_host` 注册为
  `remote_desktop_view` 的 optional dependency，并且只由显式
  `windows-native-rdp` feature 启用；默认 feature 仍为空，不改变 canvas 默认构建路径。
- **Green implementation:** 新增纯 Rust `NativeRdpEventState` reducer、
  `NativeRdpEventSource` seam 和 `drain_native_events` owner-thread adapter。adapter
  从 host-owned `EventBridge` queue drain owned raw events，在 Rust 层完成 typed decode，
  同时要求 event generation、source 当前 generation 和 reducer generation 三者一致。
  reducer 保存连接/reconnect 状态、remote size、network quality、authentication warning、
  fullscreen、完整 disconnect reason 以及 warning/fatal/logon signed raw code；confirm-close
  与 focus-released 只产生显式 UI effect，不在 native callback 内访问 GPUI context。
- **Automated verification:** feature-enabled tests 覆盖 stale generation、source/state
  generation mismatch、FIFO owned raw-event drain、malformed/unknown event no-op、连接与
  reconnect 状态转换、disconnect category 与 primary/extended raw code 保留、signed
  diagnostic raw code，以及 close/focus owner-thread effects。`WindowsRdpEvent` 新增统一
  `generation()` accessor，typed 与 unknown raw event 都保留来源 generation。
- **Integration boundary:** 本切片只冻结 callback queue → typed decode →
  generation-filtered reducer 的 contract。`RemoteDesktopView` 尚未持有
  `WindowsRdpHost`，也没有在 render/entity lifecycle 中创建、drain、show/hide、focus
  或销毁真实 child `HWND`；这些属于 Task 6，因此当前不能把“Rust callback 投递 UI
  queue，按 generation 过滤”整体勾选为完成。
- **Still pending at this slice:** HRESULT/logon code 的稳定分类当时尚未完成；后续
  diagnostic code-space mapping 切片已完成 logon 分类，但 HRESULT、真实 GPUI
  entity/event lifecycle、有交互桌面的 ActiveX/RDP session，以及错误密码、服务端拒绝、
  网络中断和服务器重启 manual acceptance 仍未完成；Task 5 整体继续保持未完成。

#### Execution Notes (2026-08-08) — native event diagnostic code-space mapping 切片

- **Green implementation:** `OnWarning`、`OnFatalError` 和 `OnLogonError` 不再只暴露
  裸 `i32`，而是分别解码为 `WindowsRdpWarning`、`WindowsRdpFatalError` 和
  `WindowsRdpLogonError`。每个 value object 都同时保存稳定的 `kind()` 和原始
  signed `code()`，不携带 native/UI 文案；未知值不会被强行归类。
- **Documented mappings:** warning code `1` 保留为 `BitmapCacheCorrupt`；fatal codes
  `0/1..7/100` 分别保留 documented unknown、internal、out-of-memory、
  window-creation、invalid-state、unrecoverable-connection 和 Winsock
  initialization 分类；logon code 覆盖 bad credentials、password change required、
  other、warning、access denied、account restriction 和六个 session-arbitration
  code。signed NTSTATUS 值按其原始 `i32` 形式匹配。
- **Non-exhaustive boundary:** Microsoft 的 `OnLogonError` code list 明确不是 exhaustive，
  因此未列出的 signed code 保持 `Unknown` 并原样保留。fatal code `0` 也和未识别值
  区分为 documented `UnknownError`。
- **Reducer/event verification:** `WindowsRdpEvent` 的 warning/fatal/logon 分支携带
  完整 diagnostic object；owner-thread reducer 同时保存 kind 和 raw code。单元测试
  覆盖 documented、signed NTSTATUS、arbitration、未知极值、typed decode 和
  malformed payload 保留 raw event。
- **Still pending at 2026-08-08:** 截至该切片，native synchronous COM operation 的
  HRESULT 尚未通过 ABI 传到 Rust；当时不能宣称 HRESULT mapping 已完成。GPUI
  entity/UI queue 接线、真实 ActiveX/RDP session 和人工错误密码、拒绝连接、网络中断、
  服务器重启验证也仍未完成，因此 Task 5 checklist 当时保持未完成。后续 ABI 与
  2026-08-10 facade classification 进展见下一节。

#### Execution Notes (2026-08-10) — synchronous HRESULT stable classification 切片

- **ABI/facade boundary:** 在 2026-08-08 之后完成的 native error diagnostic ABI
  已通过 `native/host.cpp::record_last_hresult`、
  `native/windows_rdp_host.h::NavopRdpLastError` 和
  `src/handle.rs` 的 diagnostic mapping，把 synchronous COM operation 的 HRESULT
  作为 raw signed `i32` 传到 Rust。本次 2026-08-10 切片在
  `windows_rdp_host` facade 内补齐 `WindowsRdpHresultKind` 消费路径，不修改
  C ABI/C++。`WindowsRdpHostError::NativeHresult` 继续分别保存 shim result code 与
  typed HRESULT，避免混淆两个独立 code space。
- **Conservative mapping:** 只对高置信、可稳定命名的
  `REGDB_E_CLASSNOTREG`、`E_NOINTERFACE`、`E_INVALIDARG`、`E_OUTOFMEMORY`、
  `E_ACCESSDENIED`、`RPC_E_WRONG_THREAD`、`CO_E_NOTINITIALIZED`、
  `RPC_E_CALL_REJECTED`、`RPC_E_SERVERCALL_RETRYLATER`、`RPC_E_DISCONNECTED`
  以及由 Win32 timeout/cancel 派生的 HRESULT 分类。`E_FAIL`、成功码和未识别
  HRESULT 保持 `Unknown`，所有值仍由 `code()` 原样保留 raw signed code，Display
  继续使用 32-bit hex，不引入 native/UI 文案。
- **Presentation boundary:** `remote_desktop_view` 不再重复匹配 HRESULT 整数，而是
  消费 facade stable kind。`Auto` create-time fallback 仍严格只接受
  `ClassNotRegistered` 和 `NoInterface`；`AccessDenied`、`E_FAIL`、unknown 或其他
  native error 均不会静默切换 Canvas。
- **Local verification:** TDD Red 已确认测试因缺少 `WindowsRdpHresultKind`/`kind()`
  失败；Green 后 `windows_rdp_host` crate tests 109 passed、contract tests
  20 passed、feature-enabled `remote_desktop_view` tests 144 passed。
  `cargo clippy --locked -p windows_rdp_host --all-targets --no-deps -- -D warnings`、
  format check 和 diff check 均通过。
- **Checklist/verification boundary:** 当前勾选只表示 stable facade mapping 与
  unit/contract coverage 已完成，不表示 Windows production ABI/runtime acceptance。
  该 commit 推送后仍须通过 GitHub Windows x86_64/i686 ATL/MSVC probe，才能称为
  Windows build verified。runner 不能替代真实 ActiveX RDP session、交互桌面以及
  错误密码、拒绝连接、网络中断、服务器重启的 Windows VM/真机手工验证。GPUI
  entity/UI queue 接线也仍未完成，因此对应 checklist 和 Task 5 整体保持未完成。

#### Execution Notes (2026-08-10) — GPUI owner-thread native event pump 切片

- **Owner-thread queue integration:** `RemoteDesktopView` 现有 33ms GPUI foreground
  task 会从 host-owned `EventBridge` FIFO drain raw events，并在 `Entity::update`
  owner thread 内执行 typed decode、reducer 更新和 `cx.notify()`。native callback
  只负责复制 payload 并入队，不直接持有或访问 GPUI entity/window context。
- **Generation boundary:** event source generation、`NativeRdpEventState` generation
  与每个 decoded event generation 继续要求三者一致；新 native adapter attach
  会用当前 generation 建立全新 reducer state，旧 generation event 不会改变连接状态、
  close confirmation 或 focus handoff。
- **Close/focus effects:** `CloseConfirmed` 现在保存为 generation-scoped sticky state，
  因此常规 33ms pump 先消费 close event 后，16ms graceful-close poll 仍能观察确认并
  进入 destroy。`FocusReleased` 保存为一次性 pending effect，只在当前 tab active 时
  通过 GPUI window owner thread 把 focus 交回 view；inactive tab 不会夺取当前 tab focus。
- **Tab transition hardening:** activation 在把 tab 标为 active 前先 drain 上一个
  activation 周期遗留 effect；deactivation 在同步 focus-parent/hide 后再次 drain，
  避免已排队的 focus release 跨 tab activation 重放。native child focus 仍由下一
  GPUI UI turn 的 deferred handoff 完成，快速 deactivate 会让该 handoff no-op。
- **Local verification:** TDD Red 先证明缺少 owner-thread pump 与 stale-focus drain
  contract；Green 后 feature-enabled `remote_desktop_view` tests 145 passed，
  `windows_rdp_host` crate tests 109 passed，host contract tests 20 passed。
  `cargo fmt --all -- --check`、`git diff --check` 均通过。带
  `-A clippy::derivable_impls` 的 scoped `remote_desktop_view --no-deps` Clippy
  通过且只有被显式允许的既有 warning；严格 `-D warnings` 仍被与本切片无关的
  `view/frame_sync.rs:44` `clippy::derivable_impls` 基线阻断。
- **Previous Windows runner evidence:** HRESULT commit
  `d179cb86405b1c2107822a6af9c345b23dc99dbb` 的 GitHub Actions run
  `31364335538` 已成功完成：x86_64 与 i686 `Windows RDP probe` ATL/MSVC/type-library
  compile/link 均成功，Windows x86_64 workspace `Test Windows` 也成功。该 run
  只验证对应 commit 的 Windows build/native tests，不包含本 owner-thread pump commit；
  后者在推送后仍须由新的 Windows run 核验。
- **Acceptance boundary:** 上述自动化不建立真实 ActiveX RDP session，也不替代交互
  桌面或 Windows VM/真机 manual acceptance。错误密码、服务端拒绝、网络中断、服务器
  重启以及快速跨 RDP/终端/WebView tab 的人工验证继续 pending；因此 Task 5 的人工
  Verify 与整体 runtime acceptance 仍不勾选。

---

### Task 6: GPUI tab bounds、show/hide、focus 与 DPI 垂直集成

**Goal:** 把真实 child `HWND` 放入 `RemoteDesktopView` tab，并正确跟随布局和 tab 生命周期。

**Depends on:** Task 3、Task 5。

**Files:**

- Modify: `crates/remote_desktop_view/src/view.rs`
- Modify: `crates/remote_desktop_view/src/view/render.rs`
- Modify: `crates/remote_desktop_view/src/view/output.rs`
- Modify: `crates/remote_desktop_view/src/view/resize.rs`
- Add: `crates/remote_desktop_view/src/view/windows_native.rs`
- Reference behavior: `crates/webview/src/lib.rs`
- Reference behavior: `crates/core/src/tab_container.rs`

**Integration rules:**

- 从 GPUI window 获取稳定 parent `HWND`。
- 在 layout/prepaint 后取得真实 content bounds。
- `on_activate`：apply latest bounds → show → optional focus。
- `on_deactivate`：focus parent → hide。
- 未激活 tab 永远不显示 native child。
- parent resize、tab bar resize、sidebar resize、window move/DPI change 都更新 bounds。

- [x] **Red:** presentation mock 测试 activate/deactivate 调用顺序和幂等。
- [x] **Red:** logical→physical bounds tests 覆盖 100/125/150/200%。
- [x] **Red:** inactive tab 收到 resize 时只缓存，不错误 show。
- [x] **Green:** 参考 WebView 的 prepaint bounds 同步实现 native element/adapter。
- [x] **Green:** 实现 focus handoff 和初始 hidden。
- [x] **Green:** native ActiveX 自行接收输入时关闭 canvas input path。
- [x] **Refactor:** bounds conversion 纯函数化；native command 从 render 代码抽离。
- [x] **Review:** 检查 GPUI content mask、tab padding、scroll、window border 坐标。
- [ ] **Verify:** 快速切换 RDP/终端/WebView tab，resize sidebar/window，跨 monitor 拖动。

**Acceptance:**

- native 画面严格位于 tab content bounds。
- 非 active tab 不覆盖当前 tab。
- focus 可进入 RDP，也可通过既定快捷键返回 Navop。
- DPI 不双重缩放。

**Failure gate:** 如果无法可靠取得 parent `HWND` 或 prepaint bounds，停止默认接入并先补 GPUI native-child abstraction。

**Rollback:** 移除 `WindowsNativePresentation` 的 UI factory 接入，保留隔离的 host crate 和 canvas 默认路径。

#### Execution Notes (2026-08-10) — presentation 状态条与显式 Canvas retry 切片

- **Red/contract coverage:** `RemoteDesktopPresentationInitialization` 现在显式保存失败时
  尝试的 backend 和 native child 是否已确认销毁；测试覆盖只有 cleanup 成功后的
  `Failed` 状态才允许显式 Canvas retry、retry 不同步启动第二个 runtime，以及稳定
  fallback taxonomy 到 UI locale key 的映射。
- **Green implementation:** RDP tab 在 native child content bounds 之外显示当前 backend、
  自动 fallback 原因和可用时的“使用 Canvas 重试”操作。native 初始化失败先执行
  `force_close`；只有 child 已安全销毁才开放 retry。retry 会关闭并清空可能残留的 Canvas
  runtime channel，切换 presentation 状态后交给下一次 layout/render 启动唯一 Canvas
  runtime，不在 action callback 内直接创建 session。
- **Bounds hardening:** status row 与画面使用纵向 flex；content 通过
  `ElementExt::on_prepaint` 获取自身真实 bounds，而不是依赖 root children 的
  `first()`/`last()` 顺序。该 bounds 继续作为 native child 的唯一布局输入，状态条不会
  被纳入 Win32 child rectangle。
- **Task 6 contract coverage:** presentation sink 测试覆盖 activate 的
  bounds → show → focus 顺序、deactivate 的 focus parent → hide 顺序、重复调用幂等，
  以及 active/inactive resize 行为。`logical_bounds_to_physical` 测试显式覆盖
  100/125/150/200% scale、负坐标、零尺寸和非法 scale factor。native backend 的
  keyboard、mouse、scroll 与 Canvas child 均由 `uses_windows_native` gate 排除。
- **Coordinate-space review:** GPUI `on_prepaint` 返回顶层 window client 的 logical
  pixel bounds，而 native child 的 parent 是该顶层 window `HWND`；host ABI 接收 parent
  client-area physical pixels。因此当前 `(bounds - (0, 0)) * scale_factor` 是有意的
  parent-client → physical 转换，不应减去 screen origin 或 window frame origin。
  WebView 同样直接使用 GPUI bounds origin。只有未来 parent 改为嵌套 child `HWND` 时，
  `parent_client_origin` 才需要非零值。
- **Automated verification:** macOS host 上
  `cargo test --locked -p remote_desktop_view` 通过 134 个测试；
  `cargo test --locked -p remote_desktop_view --features windows-native-rdp` 通过 144 个
  测试；`cargo fmt --all -- --check` 与 `git diff --check` 通过。
  `cargo clippy --locked -p remote_desktop_view --all-targets --features
  windows-native-rdp -- -D warnings` 被与本切片无关的既有 workspace lint
  （`agent_runtime` large-enum/unnecessary-sort、`ssh` large `Err`）阻断；增加
  `--no-deps` 后仍被既有 `remote_desktop_view/src/view/frame_sync.rs` 的
  `derivable_impls` 阻断。本切片的 Windows/MSVC/ATL 编译链接交由推送后的 GitHub
  Windows runner 核验。
- **Windows runner verification:** commit `676505675f80e6b16666e0589ed315174e01ba9d`
  的手动 Windows workflow run `31374676400` 已成功；x86_64 与 i686 ATL/MSVC RDP
  probe、Windows x86_64 workspace `Test Windows` 均通过。
- **Still pending:** 自动化 DPI 与 inactive resize contract 已完成；真实 Windows
  交互桌面上的快速 tab 切换、sidebar/window resize、视觉 bounds 对齐、per-monitor DPI
  与跨 monitor 拖动仍待人工验证，因此 Task 6 的 **Verify** 项继续保持未勾选。

---

### Task 7: 安全关闭、STA 生命周期与强制关闭兜底

**Goal:** 覆盖普通关 tab、force close、entity release、窗口关闭和应用退出的全部资源释放路径。

**Depends on:** Task 6。

**Files:**

- Modify: `crates/remote_desktop_view/src/view.rs`
- Modify: `crates/remote_desktop_view/src/view/render.rs`
- Modify: `crates/windows_rdp_host/src/handle.rs`
- Modify: `crates/windows_rdp_host/native/lifecycle.cpp`
- Add lifecycle contract tests

- [ ] **Red:** 普通 `try_close` 调用 graceful close 后才允许移除。
- [ ] **Red:** force close 绕过 `try_close` 时由 release/drop 完成 destroy。
- [ ] **Red:** close 和 release 重复调用只有一次真实 native destroy。
- [ ] **Red:** late event/late resize completion 在 generation 失效后被丢弃。
- [ ] **Red:** wrong-thread drop 不直接调用 COM，而是 marshal/投递 UI thread cleanup。
- [ ] **Red:** callback 正在执行时 close 不重入 release；`Unadvise` 后 drain in-flight callback 再释放 sink/control。
- [ ] **Red:** owner-thread dispatcher 投递失败和 UI shutdown race 不会转为 wrong-thread COM call。
- [ ] **Green:** 实现 `Closing` gate、focus parent、hide、request close、timeout、disconnect、unadvise、release、destroy。
- [ ] **Green:** 实现 owner-thread command dispatcher、in-flight callback counter/quiescence 和 app-exit host drain。
- [ ] **Green:** entity release 和 app shutdown 注册兜底。
- [ ] **Refactor:** 单一 `begin_close`/`finish_destroy` 状态机，避免多入口复制。
- [ ] **Review:** 对照 `TabContent::try_close`、`force_close_tab_by_id`、entity release 实际顺序。
- [ ] **Verify:** 连接中关 tab、已连接关 tab、重连中关 tab、应用直接退出各重复 20 次。

**Acceptance:**

- 无 use-after-free、COM wrong-thread、悬挂 HWND。
- 手动关闭不触发自动重连。
- 应用退出不等待无限期。
- 正常退出在 COM apartment uninitialize 前销毁全部 host；紧急退出从不尝试 wrong-thread release。

**Rollback:** 若 graceful close 不稳定，可保留短 timeout 后强制销毁，但不得跳过 focus/hide/unadvise。

#### Execution Notes (2026-08-10) — callback quiescence 与有界关闭重试切片

- **Red evidence:** `WindowsNativeAdapter::force_close` 从 `Result<()>` 迁移为显式
  `NativeDestroyProgress` 后，初始化失败路径仍匹配旧的 `Ok(())`。先更新
  `explicit_canvas_retry_requires_confirmed_native_cleanup_and_defers_runtime_start`
  contract，要求只有 `Destroyed` 才允许显式 Canvas retry，并确认该测试因生产代码
  仍是旧匹配而按预期失败。
- **Green implementation:** `WindowsRdpHost::close` 遇到 native
  `CALLBACK_IN_FLIGHT` 时保持 `Closing`、callback/context 与 opaque handle，不调用
  destroy，也不重新打开 Rust callback admission gate；后续 owner-thread turn 可重试
  unregister，成功后再执行唯一一次 destroy。`WindowsNativeAdapter` 将这一状态映射为
  `NativeDestroyProgress::PendingCallbacks`，只有 `Destroyed` 才结束 presentation
  ownership。普通 `try_close` 先等待 graceful close confirmation，2 秒后进入 force
  close；force close 继续等待 callback quiescence，并有额外 2 秒 hard deadline，
  每 16ms 在 GPUI owner thread 重试，避免无限等待。初始化失败同样只有确认 native
  已销毁时才开放 Canvas retry。
- **Release boundary:** entity release 仍在 owner thread 同步调用 force close；
  `Destroyed` 才取出 native adapter/event state，`PendingCallbacks` 或错误不会冒充
  cleanup 已完成。若 entity 已在异步 close task 重试期间释放，
  `WeakEntity::update` 的错误只表示 entity 已 release，此时由 release fallback 接管。
  app-exit host drain、dispatcher 投递失败和 UI shutdown race 仍需后续独立切片。
- **Automated verification:** macOS host 上 `cargo fmt --all -- --check` 和
  `git diff --check` 通过；`windows_rdp_host` 为 90 unit + 20 contract tests passed；
  `remote_desktop_view` 默认 135 passed，`windows-native-rdp` feature-on 145 passed。
  `windows_rdp_host --all-targets --no-deps -D warnings` Clippy 通过；
  `remote_desktop_view --all-targets --features windows-native-rdp --no-deps` 在仅豁免仓库
  既有 `frame_sync.rs` `derivable_impls` 后以 `-D warnings` 通过。两路独立只读审查
  未发现本切片阻断性的重复 destroy、callback gate、状态迁移或死锁问题。
- **Windows runner verification:** commit
  `4df7548c5f9b3a7353bf5b14d21f800dce47b84d` 的手动 Windows workflow run
  [`31386493350`](https://github.com/feigeCode/navop/actions/runs/31386493350)
  成功。`Windows RDP probe (x86_64-pc-windows-msvc)` 与
  `Windows RDP probe (i686-pc-windows-msvc)` 的 `Build ATL/MSVC probe` 均成功，
  `Test (windows, x86_64-pc-windows-msvc)` 的 `Test Windows` 也成功。
- **Verification boundary:** runner 证明当前 x64/i686 MSVC/ATL probe 可编译链接，
  并证明 Windows x86_64 自动化测试通过；它不证明真实 ActiveX/RDP session、
  交互式 child `HWND`、COM apartment shutdown、应用退出 drain 或连接中关闭 tab。
  连接中、已连接、重连中关闭 tab及应用直接退出各重复 20 次的人工验证仍 pending，
  因此 Task 7 整体继续保持未完成。

#### Execution Notes (2026-08-10) — wrong-thread fail-closed 与 detached owner-thread cleanup

- **Wrong-thread boundary:** commit `4e5bf3db` 将 `WindowsRdpHost::drop` 的
  wrong-thread 路径改为 fail-closed：不 unregister callback、不 destroy native host、
  不释放 callback context，也不在错误线程调用 COM/Win32 cleanup；该路径记录诊断后
  保留完整 ownership，由进程回收资源。
- **Detached cleanup ownership:** commit `a0fd5bb3` 让 view release 与
  post-create 初始化失败路径在同步 `force_close` 无法确认 destroy 时，把完整
  `WindowsNativeAdapter` ownership 移交给 GPUI foreground `cx.spawn` task。task 每
  16ms 在 owner/UI thread 重试；`Destroyed` 或错误但 `native.is_destroyed()` 时完成，
  `PendingCallbacks` 继续等待；2 秒 deadline 后 fail-closed，泄漏整个 adapter，而不是
  drop 其中仍可能被 callback 引用的 bridge/control/host。显式 Canvas retry 仍只在
  确认 `Destroyed` 后开放。
- **Local automated verification:** macOS host 上
  `cargo fmt --all -- --check`、`git diff --check`、默认
  `remote_desktop_view` 135 tests、`windows-native-rdp` feature-on 145 tests 均通过；
  `remote_desktop_view --all-targets --features windows-native-rdp --no-deps` 在仅豁免
  仓库既有 `frame_sync.rs` `derivable_impls` 后以 `-D warnings` 通过。最后的
  Windows-only warning 修复后，目标 contract
  `windows_native_close_waits_for_confirmation_and_keeps_a_release_fallback` 再次通过。
- **Windows runner verification:** commit
  `a0fd5bb316837b049c25c9d9ad44688330225ea2` 的手动 Windows workflow run
  [`31392708100`](https://github.com/feigeCode/navop/actions/runs/31392708100)
  成功。`Windows RDP probe (x86_64-pc-windows-msvc)` 与
  `Windows RDP probe (i686-pc-windows-msvc)` 的 `Build ATL/MSVC probe` 均成功，
  `Test (windows, x86_64-pc-windows-msvc)` 的 `Test Windows` 也成功。
- **Remaining Task 7 scope:** 本次 runner 证明 x64/i686 MSVC/ATL probe 可编译链接，
  且 Windows x86_64 自动化测试通过；它不证明真实 ActiveX/RDP session、真实
  connect/disconnect、错误密码/拒绝连接/网络中断/服务器重启、COM apartment
  shutdown 或应用退出 drain。app-exit host drain、owner-thread dispatcher 投递失败、
  UI shutdown race，以及连接中/已连接/重连中关 tab和应用直接退出各重复 20 次的人工
  验证仍 pending，因此 Task 5、Task 7 与整个计划均不得标记完成。

#### Execution Notes (2026-08-10) — application-exit Native RDP drain

- **Red evidence:** 主程序 source contract
  `confirmed_and_update_quit_paths_await_all_application_resource_shutdown` 先要求共享退出
  helper 启动并等待 Native RDP drain，再等待 SSH shutdown，最后才调用 `cx.quit()`；
  初次运行因缺少 `remote_desktop_view::shutdown_windows_native_rdp(cx)` 而按预期失败。
  host registry 的 `OwnerLost` contract 在 API 尚不存在时先编译失败；应用级 controller
  contract 随后先因 missing owner 在 deadline 后没有 terminal completion 而失败。
  独立审查发现 Detached owner 若 cleanup task 永不回报仍可能无限等待后，又先增加
  deadline convergence contract，并确认它因缺少 detached `OwnerLost` fallback 而失败。
- **Green implementation:** 新增 application-owned Native RDP shutdown controller：
  native adapter 创建成功后立即向 `WindowsRdpShutdownRegistry` 注册完整
  token/generation，并在 attach、初始化失败、普通 tab close、entity release 和
  detached cleanup 路径始终成对转移 adapter 与 registration。首次 drain 关闭 admission，
  捕获 pending registrations，并设置全局 2 秒 deadline；GPUI foreground 每 16ms 在 owner
  thread 请求 force close。deadline 到期后，仍由 live view 持有的完整 adapter 先
  `Box::leak` quarantine，再记录 `TimedOutLeaked`；owner metadata 丢失或 Detached cleanup
  未回报时记录独立的 `OwnerLost`，不冒充 adapter 已 destroy 或已确认 leak，从而保证应用
  drain 有界收敛。迟到 completion 通过 registry tombstone 返回 `AlreadyTerminal`，不会
  重复计数或重分类。Windows feature/target 下若 application controller global 根本不存在，
  shutdown entrypoint 现在返回显式 `controller_unavailable` fail-closed report，而不是把
  零计数默认报告误当成成功；该状态纳入 `incomplete()`，并由主程序退出日志单独记录，且不
  虚构 registration 或 terminal outcome。
- **Application exit order:** `shutdown_application_resources_and_quit` 成为 production
  唯一调用 `cx.quit()` 的 helper。正常确认退出保持
  `close_all_tabs().await → Native RDP drain → SSH shutdown → GPUI quit`；更新安装路径复用
  同一 application-resource helper，并保持 Native RDP → SSH → quit 的顺序。重复调用继续
  join 同一幂等 registry/service shutdown lifecycle；缺少 SSH global 只能跳过 SSH，
  不能跳过已经完成的 Native RDP drain。
- **Refactor:** controller 按 public report、Windows registry/owner state、bounded drain
  polling 拆为三个职责文件，分别为 96、181、238 行；drain loop 又拆为 admission、
  snapshot、owner classification、view polling 和 terminal-report helper，保持源文件与
  函数规模门禁。
- **Local automated verification:** macOS host 上，refactor 前的完整验证为
  `windows_rdp_host` 106 passed、host contract 20 passed、
  `remote_desktop_view` default 137 passed、feature-on 147 passed、`main` 424 passed，
  `cargo check -p main` default/feature-on 均通过。strict
  `windows_rdp_host --all-targets --no-deps -D warnings` Clippy 通过；
  `remote_desktop_view --all-targets --features windows-native-rdp --no-deps` 在仅豁免
  仓库既有 `frame_sync.rs` `derivable_impls` 后以 `-D warnings` 通过；`main`
  `--all-targets --no-deps` 在仅豁免既有
  `iter_overeager_cloned`/`derivable_impls`/`needless_lifetimes` 后以 `-D warnings`
  通过。最终提交前完整重跑结果为：`windows_rdp_host` 127 passed、
  `remote_desktop_view` default 138 passed、feature-on 148 passed、`main` 424 passed；
  `cargo check -p main` default/feature-on 均通过；上述三个 strict Clippy 命令均通过；
  `cargo fmt --all -- --check` 与 `git diff --check` 通过。过程中额外补上
  controller-global 缺失的 Red/Green contract，并通过精确 cfg 消除普通非 Windows
  build 中该 fail-closed constructor 的 dead-code warning。
- **Independent review:** 三路独立只读审查确认 registry accounting、stale/duplicate
  terminal completion、main 退出顺序与 Windows cfg 边界没有阻断问题；最终 spot review
  再次确认 Detached owner 只在全局 deadline 后记录 `OwnerLost`，drain 可收敛，且迟到
  `Destroyed`/`TimedOutLeaked` completion 只得到 `AlreadyTerminal`。审查剩余问题聚焦
  owner-thread dispatcher delivery failure、GPUI/UI shutdown task cancellation 与
  platform-driven quit race，属于 Task 7 下一自动化切片。
- **Verification boundary:** 本切片尚未证明 Windows-only GPUI/COM 编译链接、真实
  ActiveX session、COM apartment shutdown 或 platform-driven quit race；推送后的 GitHub
  Windows runner 只用于补充 x64/i686 MSVC/ATL probe 与 Windows x64 自动化证据。错误密码、
  server refusal、网络中断、服务器重启、快速 RDP/terminal/WebView tab 切换、resize、
  per-monitor DPI/跨 monitor、连接中/已连接/重连中关 tab以及应用直接退出各重复 20 次仍
  pending。因此 Task 5、Task 6、Task 7 与整个计划继续保持未完成。

#### Execution Notes (2026-08-11) — dispatcher rejection fail-closed convergence

- **Red evidence:** 先加强
  `windows_native_shutdown_uses_locked_gpui_context_contracts`，要求 detached/view-owner
  terminal update 不再丢弃 `AsyncApp::update_global` 的 dispatcher delivery result；
  生产代码仍静默忽略该 `Result` 时，目标 contract 按预期以
  `AsyncApp terminal updates must preserve dispatcher rejection` 失败。
- **Observable delivery:** 新增稳定内部结果
  `WindowsNativeRdpTerminalDispatch::{Delivered, Rejected}`。detached terminal update
  与 view-owner-lost update 都捕获 `cx.update_global(...)` 的 `Result`、记录 token 与
  generation，并把 dispatcher rejection 显式返回给调用方；普通非 Windows production
  build 通过精确 cfg 不引入 dead-code warning。
- **Fail-closed convergence:** application drain 在每次 owner poll 前保存 registry 的
  最新 `fail_closed_report`。任一 missing、Detached 或 View owner terminal update 被
  dispatcher 拒绝时，drain 立即返回该保守报告；仍 active 的 registration 因而只分类为
  `OwnerLost`，不会冒充 `Destroyed`，也不会继续无意义地等待下一次 16ms poll。独立只读
  审查发现初版 `poll_registration` 的 Detached/View match arm 尾部误加分号，会丢弃
  delivery result 并造成 Windows-only 类型错误；提交前已移除分号，并增加 contract 防止
  两个分支再次静默丢弃 rejection。
- **Detached cleanup boundary:** detached owner-thread cleanup 在确认 `Destroyed`，或先
  leak 完整 adapter 再确认 `TimedOutLeaked` 后，观察 terminal delivery rejection 并记录
  错误；它不会重试 native destroy，不会从 rejection 路径调用 `force_close`，也不会增加
  wrong-thread COM/Win32 cleanup fallback。
- **Local automated verification:** macOS host 上
  `cargo fmt --all -- --check`、`git diff --check`、`remote_desktop_view` default
  140 passed、feature-on 150 passed，`cargo check -p main` default/feature-on 均通过；
  `remote_desktop_view --all-targets --features windows-native-rdp --no-deps` 在仅豁免
  仓库既有 `frame_sync.rs` `derivable_impls` 后以 `-D warnings` 通过。两路独立只读审查
  随后确认 `AsyncApp::update_global` 返回类型、cfg、`#[must_use]` 消费、报告分类与
  no-wrong-thread-cleanup 边界；审查发现的 Windows-only match-arm 阻断问题已修复并由
  focused contracts 复验。
- **Previous Windows runner evidence:** 前一 application-exit drain HEAD
  `49950a760bb6568dab77040e6622adb907c633f9` 的 GitHub Actions run
  [`31448900925`](https://github.com/feigeCode/navop/actions/runs/31448900925) 已全部成功：
  x64/i686 `Windows RDP probe` 的 `Build ATL/MSVC probe` 与
  `Test (windows, x86_64-pc-windows-msvc)` 的 `Test Windows` 均成功。该证据仅证明该
  HEAD 的 Windows 编译链接和自动化测试，不证明真实交互式 ActiveX/RDP runtime 行为；
  当前 dispatcher-rejection 提交仍需推送后单独由同类 Windows runner 核验。
- **Remaining Task 7 scope:** GPUI/UI shutdown task cancellation、
  platform-driven quit race，以及真实连接中/已连接/重连中关闭 tab与应用直接退出各重复
  20 次仍 pending。Task 7 与整个计划继续保持未完成。

#### Execution Notes (2026-08-11) — platform-driven quit fallback 与 Windows dispatcher compile correction

- **Red evidence:** `main` 的 platform-driven `on_app_quit` observer 原先只启动 SSH
  fallback，没有同步关闭 Native RDP admission；新增
  `platform_quit_fails_closed_native_rdp_before_ssh_without_recursive_quit` 后，生产源码缺少
  RDP fallback 时 contract 按预期失败。`remote_desktop_view` 同步增加 source contract，
  要求 platform quit 入口必须调用 `begin_drain`，返回 completed 或 conservative
  report，并禁止 `spawn`、owner polling、wrong-thread force-close 或虚构
  `Destroyed`。此外，dispatcher-rejection 提交
  `cd3ffe9665b8c47d64d061f9a6844fce19092637` 的 GitHub Actions run
  [`31450193531`](https://github.com/feigeCode/navop/actions/runs/31450193531)
  暴露了本地 macOS cfg 无法发现的真实 Windows-only 编译错误：Detached owner helper
  尾部分号把 terminal dispatch 丢成 `()`；同时实现错误地把
  `AsyncApp::update_global` 当作 `Result`。
- **Correction to the previous execution note:** GPUI 当前锁定版本的
  `AsyncApp::update_global` 返回 closure 的 `R`，不是可观察的 `Result<R, _>`；上一节
  “捕获 `cx.update_global(...)` 的 `Result`”以及“独立审查确认其返回类型”的描述不正确。
  当前实现改为先用 quit-aware 的 `AsyncApp::try_read_global` 同步拒绝 app 已进入
  quitting 或 controller 已不可读的更新，再在没有 `await`/调度让出的情况下执行
  `update_global`；delivery 通过 `Option` 转换为
  `WindowsNativeRdpTerminalDispatch::{Delivered, Rejected}`。Detached owner helper 的
  尾部分号也已移除，恢复 terminal dispatch 返回值。run `31450193531` 中 i686 probe
  与 Windows x64 常规自动化测试成功，但 x64 Native RDP probe 编译失败，所以该 run
  整体为 failure，不能作为 dispatcher-rejection HEAD 的成功证据。
- **Platform-driven quit behavior:** 新增
  `fail_closed_windows_native_rdp_for_platform_quit`。GPUI 已决定退出、只给 quit
  observer 很短固定预算时，该入口同步调用 `begin_drain` 关闭 admission：registry
  已完成则返回稳定 completed report，否则返回保留既有 progress 的 conservative
  fail-closed report。pending registration 只在返回报告中分类为 `OwnerLost`；入口不
  修改 registry terminal state，不写 tombstone，不宣称 `Destroyed`，也不启动 task、
  owner polling、native force-close、destroy、quarantine 或 unregister。`main` 在
  启动/等待 SSH fallback 前先调用并记录该 RDP report；platform callback 不递归进入
  full application shutdown helper，也不调用 `cx.quit()`。
- **Windows-only behavior test:** 新增
  `platform_quit_fail_closed_report_preserves_progress_without_mutating_pending_registration`：
  两个 synthetic registration 开始 drain 后，先记录一个 `Destroyed`，再执行 platform
  quit fallback；报告必须保留 `destroyed = 1` 并把另一个 pending registration
  保守报告为 `owner_lost = 1`，同时 registry 仍保持 `Draining`、一个 active pending
  registration、且稳定 terminal `report()` 仍为 `None`。该测试受
  Windows + `windows-native-rdp` cfg 约束，必须由 Windows x64 RDP probe job 编译运行。
- **Local automated verification:** macOS host 上
  `cargo fmt --all -- --check`、`git diff --check`、`windows_rdp_host` unit 107 passed
  加 contract 20 passed、`remote_desktop_view` default 140 passed、feature-on 150
  passed、`main` 425 passed；`cargo check -p main` default/feature-on 均通过。
  `remote_desktop_view --all-targets --features windows-native-rdp --no-deps` 在仅豁免
  仓库既有 `frame_sync.rs` `derivable_impls` 后以 `-D warnings` 通过。两路独立只读
  核验没有发现当前六个生产/contract 文件的已确认阻断问题，并确认 CI 的
  `Windows RDP probe (x86_64-pc-windows-msvc)` 会显式启用 feature、编译并运行上述
  Windows-only test；i686 probe 继续覆盖 x86 ATL/MSVC host build 与 host tests。
- **Verification boundary:** 待推送 HEAD 的 GitHub runner 只能证明 x64/i686
  MSVC/ATL 编译链接和自动化测试，不能证明真实交互式 ActiveX/RDP session、child HWND
  行为、COM apartment teardown，或连接中/已连接/重连中关闭 tab与应用直接退出的真实
  race。上述四类场景各重复 20 次仍 pending；GPUI/UI shutdown task cancellation 也仍是
  Task 7 的下一自动化切片。因此 Task 7 与整个计划继续保持未完成。

#### Execution Notes (2026-08-11) — Windows-only GPUI test-support correction

- **Runner evidence:** platform-quit fallback HEAD
  `3e69053704452f03cf1b98ed0ea5182d6c981d10` 的 GitHub Actions run
  [`31452350636`](https://github.com/feigeCode/navop/actions/runs/31452350636)
  已结束：i686 Native RDP probe 与 Windows x64 常规自动化测试成功，但 x64 Native RDP
  probe 在编译 `remote_desktop_view` 的 Windows-only lib test 时失败。错误集中在
  `#[gpui::test]` 展开的 `gpui::run_test`/`gpui::TestAppContext` 不可见，以及测试闭包中
  `&mut gpui::App` 的 `update_global` trait method 未进入作用域；因此该 run 整体为
  failure，不能作为 platform-quit fallback HEAD 的完整 Windows 成功证据。
- **Root cause and correction:** `gpui` 的测试 API 受 `test-support` feature 门控；
  `remote_desktop_view` 原先没有像仓库其他 GPUI crate 一样在 dev-dependency 中启用该
  feature。现在增加
  `gpui = { workspace = true, features = ["test-support"] }`，并在 Windows-only test
  module 中导入 `gpui::BorrowAppContext as _`，使 `TestAppContext` 上的
  `update_global` 方法来自其实际定义 trait。行为测试本身保留，不以删除或降级测试来
  绕过 Windows 编译证据。
- **Local automated verification:** 修复后 macOS host 上
  `cargo fmt --all -- --check`、`git diff --check`、`remote_desktop_view` default
  140 passed、feature-on 150 passed，且
  `cargo check --locked -p main --features windows-native-rdp` 通过。Windows-only
  `#[gpui::test]` 的实际编译运行仍必须由修复提交推送后的 x64 Windows runner 核验；
  该 runner 证据不能替代真实 ActiveX/RDP runtime 与四类 20 次手工 race 场景。

#### Execution Notes (2026-08-11) — deterministic MSVC RDP type-library import

- **Runner evidence and root cause:** Windows-only GPUI test-support HEAD
  `cc5f57ee6a591a84e6bebfc00f4c9bfd89ffed88` 的 GitHub Actions run
  [`31453583139`](https://github.com/feigeCode/navop/actions/runs/31453583139)
  已结束。i686 Native RDP probe 与 Windows x64 常规自动化测试成功，但 x64 Native
  RDP probe 在 feature-on `remote_desktop_view` test graph 编译
  `active_x_host.cpp` 时因 `OUT_DIR/mstscax.tlh` 不存在而失败，因此该 run 整体为
  failure。完整 x64 graph 通过其他依赖统一激活 `cc 1.2.65` 的 `parallel` feature；
  原先 `event_sink.cpp` 和 `active_x_host.cpp` 各自执行相同 MSVC `#import`，两个并行
  compiler process 因而竞争生成和消费同名 `mstscax.tlh`/`mstscax.tli`。standalone
  host/probe graph 没有激活该并行路径，所以 i686 与较小构建图不能证明该 race 不存在。
- **Deterministic generation:** 新增单一 `native/mstscax_import.cpp`，集中保留
  `raw_interfaces_only`、`named_guids`、`no_namespace` 与 i686 所需的
  `exclude("UINT_PTR")`。build script 先用单 source
  `try_compile_intermediates()` 同步执行 importer；即使 `cc` 启用 `parallel`，其
  多对象并行分支也不会用于该单对象调用。importer 返回后显式检查
  `OUT_DIR/mstscax.tlh`，随后才启动包含全部 host translation units 的 archive
  compile。`event_sink.cpp` 与 `active_x_host.cpp` 现在只 include 已生成的 header，
  不再并行执行 `#import`。intermediate importer object 不进入 host archive，也不通过
  单独 `compile()` 引入额外 Rust static-library link metadata。
- **Contract and local verification:** 新增 source/build contract，冻结唯一 importer、
  `try_compile_intermediates()`、generated-header 检查、先生成后编译以及两个 consumer
  禁止 `#import` 的顺序和边界。macOS host 上
  `cargo fmt --all -- --check`、`cargo test --locked -p windows_rdp_host`、
  `cargo test --locked -p windows-rdp-probe`、
  `cargo test --locked -p remote_desktop_view`、feature-on
  `remote_desktop_view` tests、`cargo check --locked -p main --features
  windows-native-rdp` 与 `git diff --check` 均通过。两路独立只读核验未发现
  link-metadata 污染、x86/i686 contract 回退、`UINT_PTR` 兼容约束丢失或复制/注册/
  提交 `mstscax.dll`/generated headers 的阻断问题。
- **Verification boundary:** 本地非 Windows 验证不能证明 MSVC `#import` 的实际生成
  位置、双架构注册表 type-library 解析或最终 ATL link；当前实现会在路径假设不成立时
  以明确的 generated-header assertion fail closed。该提交推送后必须由新的 Windows
  x64/i686 probe 以及 x64 feature-on GPUI tests 核验。runner 成功也只证明
  MSVC/ATL/type-library compile/link 与自动化测试，不证明真实交互式 ActiveX/RDP
  session、child HWND/focus、COM apartment teardown、platform quit race 或四类各
  20 次人工关闭场景；Task 7 与整个计划继续保持未完成。
- **First follow-up runner correction:** deterministic import HEAD
  `f24a4b9b388849515b861d69f94c04a9efb6541c` 的 run
  [`31455057780`](https://github.com/feigeCode/navop/actions/runs/31455057780)
  中，i686 Native RDP probe 已成功，x64 graph 也已越过原先的
  `mstscax.tlh` C1083 race，但随后在 Windows-only GPUI shutdown test 编译处失败：
  `drain.rs` 的测试子模块仍导入 `AppContext`，而 `update_global` 实际由
  `BorrowAppContext` 提供。修复仅把该测试作用域导入改为
  `gpui::BorrowAppContext as _`；两路独立只读审计确认相邻 Windows/feature-on
  代码没有第二处同类 trait/import 错配。macOS host 上 feature-on
  `remote_desktop_view` 150 tests、format 与 diff checks 通过；真实 Windows
  compile 仍由下一次 runner 核验。

#### Execution Notes (2026-08-11) — shutdown task cancellation fail-closed preservation

- **Prior Windows runner verification:** Windows-only GPUI trait-import correction HEAD
  `b74e37d6e5c9a201e634d63a68efbfcbefb03176` 的 GitHub Actions run
  [`31455979893`](https://github.com/feigeCode/navop/actions/runs/31455979893)
  已整体成功。x64/i686 `Windows RDP probe` 与 Windows x64 常规自动化测试 job
  均成功；其中 x64 probe 显式执行
  `cargo test --locked -p remote_desktop_view --features windows-native-rdp --target
  x86_64-pc-windows-msvc`，因此编译运行 Windows-only Native RDP GPUI tests。i686
  probe 继续覆盖 x86 ATL/MSVC host build 与 `windows_rdp_host` tests，但不编译
  `remote_desktop_view` 的 feature-on tests；Windows x64 常规 `cargo test --all`
  同样不启用该 feature。
- **Cancellation contract and regression:** 当前锁定 GPUI `Task` 的 drop 会立即取消
  task，只有 `detach` 才允许继续运行；`shutdown_windows_native_rdp` 在创建 async
  drain task 前同步调用 `begin_drain`。新增 Windows-only
  `dropping_shutdown_task_preserves_fail_closed_registry_state`：注册一个 synthetic
  pending generation，调用 shutdown 后在向测试 executor 让出前立即 drop 返回的
  task，再 `run_until_parked`。断言 admission 保持关闭、registry lifecycle 保持
  `Draining`、原 registration 仍 active/pending，且没有 stable terminal report。
- **Fail-closed projection boundary:** 同一测试随后调用 platform quit fallback，唯一
  pending registration 必须在返回 report 中投影为 `OwnerLost`，同时
  `requested = 1`、`destroyed = 0`、`timed_out_leaked = 0`、`incomplete = true`。
  fallback 后 registry 仍为 `Draining`、registration 仍 active、stable report 仍为
  `None`，证明 conservative `OwnerLost` 只存在于返回报告中，没有经
  `record_terminal` 写回 tombstone 或伪造 native cleanup。
- **Implementation decision:** 当前 production entrypoint 已在可取消 future 之外同步
  完成 drain admission，因此本切片只增加 regression test；没有添加 Task drop guard、
  cancellation-time async cleanup、wrong-thread COM/Win32 cleanup，也没有把 cancellation
  伪造成 `Destroyed`、`TimedOutLeaked` 或 registry terminal `OwnerLost`。独立只读审查
  确认该 fail-closed 状态机边界成立，并建议继续把“drop 前未向 executor 让出”和后续
  已 poll cancellation/platform quit race 作为不同自动化时序分别验证。
- **Local automated verification:** macOS host 上
  `cargo test --locked -p remote_desktop_view` 140 passed，
  `cargo test --locked -p remote_desktop_view --features windows-native-rdp` 150 passed，
  `cargo check --locked -p main --features windows-native-rdp`、
  `cargo fmt --all -- --check` 与 `git diff --check` 均通过。由于测试受 Windows +
  `windows-native-rdp` + `cfg(test)` 约束，本地两组测试不会编译该新增 case；提交推送后
  必须由 x64 RDP probe 的 feature-on test 命令补充真实 Windows 编译运行证据。
- **First follow-up runner correction:** cancellation test HEAD
  `bd57e70d86410da9012dd76657d453f5e10ce5b1` 的 run
  [`31457661050`](https://github.com/feigeCode/navop/actions/runs/31457661050)
  中，i686 Native RDP probe 成功，但 x64 feature-on test graph 在编译新增测试时失败：
  测试错误地对 application-facing、只公开 scalar counts 的
  `WindowsNativeRdpShutdownReport` 调用了 host registry report 才有的
  `owner_lost_registrations()`。修复移除该越层断言；`owner_lost = 1` 加上 fallback
  前后 registry 中同一个 registration 仍 active/pending、stable terminal report 仍为
  `None`，已经覆盖本切片所需的 projection-without-mutation 语义。该 run 整体不能作为
  cancellation HEAD 的成功证据，修复提交仍需新的 Windows runner 核验。
- **Corrected Windows runner verification:** public-report API correction HEAD
  `0c8ecf3db9deb30b8769fc746086515be854c580` 的 GitHub Actions run
  [`31458324056`](https://github.com/feigeCode/navop/actions/runs/31458324056)
  已整体成功：x64 Native RDP probe 9m57s、i686 probe 2m42s、Windows x64 常规测试
  19m54s，Icon audit 与 matrix preparation 也成功。x64 probe 的 feature-on
  `remote_desktop_view` test command 已在真实 Windows/MSVC target 上编译运行修正后的
  cancellation regression；因此 run `31457661050` 只保留为失败修正记录，不再是当前
  HEAD 的待验证缺口。
- **Remaining Task 7 scope and verification boundary:** Windows runner 成功最多证明
  x64/i686 MSVC/ATL build、x64 feature-on Native RDP tests 与 Windows x64 常规自动化
  tests；不证明真实交互式 ActiveX/RDP session、child `HWND` 视觉/焦点/Z-order、COM
  apartment teardown、已启动/已 poll drain task 与 platform quit 的全部 race，也不替代
  连接中/已连接/重连中关闭 tab及应用直接退出各重复 20 次的人工验证。下一自动化切片继续
  覆盖已 poll task cancellation 与 quit-aware controller loss；Task 7 与整个计划继续保持
  未完成。

#### Execution Notes (2026-08-11) — polled shutdown cancellation and controller-loss preservation

- **Polled cancellation regression:** 新增 Windows-only
  `dropping_polled_shutdown_task_preserves_progress_for_platform_quit`。测试注册两个
  synthetic registration，在 shutdown admission 同步进入 `Draining` 后先把其中一个记录为
  真实 `Destroyed`，再用一次 `BackgroundExecutor::tick()` 只执行 drain task 的首次
  runnable poll。该 poll 读取最新 registry snapshot，并停在 16ms bounded poll timer；
  测试确认 task 尚未 ready 后 drop task，覆盖“task 已启动且已 poll”而不是上一切片的
  “首次 poll 前取消”。取消后 registry 仍为 `Draining`，另一个 registration 仍 active /
  pending，stable report 仍为 `None`，且 `Detached` owner sentinel 保持原样。
- **Executor timing decision:** 当前锁定 GPUI test scheduler 的 `tick()` 最多运行一个
  runnable task，因此适合精确停在首次 drain poll 的 timer wait；不能使用
  `run_until_parked()` 建立该前置条件，因为后者会排空 runnable 并推进到后续 timer。
  controller-loss case 在首次 poll 后移除 controller global，再推进一个
  `WINDOWS_NATIVE_RDP_DRAIN_POLL_INTERVAL`，随后用
  `BackgroundExecutor::run_until_parked()` 驱动 timer 唤醒后的 continuation；测试先用
  `Task::is_ready()` 确认 drain 已完成，再用 `futures::executor::block_on(task)` 仅消费
  ready result，从而让 task 在下一次 snapshot read 观察 controller 缺失。测试不依赖
  production-only hook，也不增加 task drop guard 或 cancellation cleanup。
- **Fail-closed report distinction:** 已 poll task 在 controller 丢失时返回首次 poll 保存的
  最新 conservative report，因此保留真实 `destroyed = 1`，把仍 active 的 registration
  投影为 `owner_lost = 1`，并保持 `controller_unavailable = false`；这表示 controller
  曾存在且 drain 已取得可信 progress。测试随后直接调用 platform quit fallback；由于
  global 此时确实不存在，该独立入口返回零计数且
  `controller_unavailable = true` 的 unavailable-controller report。两个报告不能互换，
  也都不会把 conservative projection 写回 registry terminal state。
- **Detached-owner boundary:** 两个测试都使用 `WindowsNativeRdpOwner::Detached` 作为
  owner sentinel，使首次 poll 确定进入 stalled detached-owner 路径。deadline 未到时该
  路径只保持 pending，不触发 native force-close、destroy、unregister 或 quarantine；
  platform fallback 同样只读并投影 fail-closed report。测试在 task cancellation、
  controller loss 和 fallback 后检查被保留/移出的 controller，确认 pending registration、
  active count、owner metadata 和 stable report 均未被错误修改。
- **Local automated verification:** macOS host 上
  `cargo test --locked -p remote_desktop_view` 140 passed，
  `cargo test --locked -p remote_desktop_view --features windows-native-rdp` 150 passed，
  `cargo check --locked -p main --features windows-native-rdp`、
  `cargo fmt --all -- --check` 与 `git diff --check` 均通过。独立只读核验确认
  `Task::is_ready`、`App::remove_global`、`TestAppContext::executor`、
  `BackgroundExecutor::tick/advance_clock/run_until_parked` 的 API/cfg 可见性，首次
  poll/timer 时序、cached report accounting 与 Detached owner no-cleanup 边界没有已确认
  阻断。
  由于两个新增测试受 Windows + feature-on + `cfg(test)` 约束，本地 macOS 测试不会编译
  它们；必须由推送后的 x64 Native RDP probe 真实编译运行。
- **First Windows runner correction:** GitHub Actions run `31460111705`（commit
  `98cabed5adb49caa200236ca89fe41db887cab4b`）的 i686 ATL/MSVC probe、Windows x64
  常规 tests、Linux tests、macOS tests、Icon audit 与 matrix preparation 均成功，但 x64
  Native RDP probe 在编译新增 Windows-only regression 时失败：
  `BackgroundExecutor` 不提供 `block_on`。该 run 只作为真实 Windows 编译暴露测试 API
  误用的失败修正记录，不是当前切片的成功证据。修复改为
  `advance_clock -> run_until_parked -> Task::is_ready ->
  futures::executor::block_on`，并把 `futures` 声明为该 crate 的直接 dev-dependency；
  修复后的真实 Windows 编译运行证据见下一项。
- **Successful Windows runner verification:** GitHub Actions run
  `31461407370`（commit `14fb89000749f4e0fb9c5095f536d420c3cc4862`）整体
  `success`。Windows RDP probe x64 成功（10m2s），在真实
  `x86_64-pc-windows-msvc` + ATL 环境编译并运行 feature-on
  `remote_desktop_view` tests，因而实际覆盖本切片新增的两个 Windows-only GPUI
  regression；i686 probe 成功（2m47s），只证明 `i686-pc-windows-msvc` architecture /
  ATL/MSVC build，不声称运行受 x64 约束的 regression。Windows x64 常规 tests 成功
  （20m43s），Linux tests 成功（7m41s），macOS tests 成功（13m25s），Icon audit 成功
  （1m32s），matrix preparation 成功（2s）。成功 run：
  `https://github.com/feigeCode/navop/actions/runs/31461407370`。
- **Remaining Task 7 scope and verification boundary:** 上述 GitHub runner 证据最多
  证明 x64/i686 MSVC/ATL build、x64 feature-on GPUI tests 与 Windows x64 常规自动化；
  不证明真实 ActiveX/RDP session、child `HWND` 视觉/焦点/Z-order、COM apartment
  teardown、全部 platform-driven quit race，或连接中/已连接/重连中关闭 tab与应用直接
  退出各重复 20 次。Task 7 与整个计划继续保持未完成。

---

### Task 8: presentation/backend 选择、capability probe 与 fallback

**Goal:** 正式接通 Auto/Windows Native/Canvas，并让 native unavailable 可诊断、可回退。

**Depends on:** Task 7。

**Files:**

- Modify: `crates/remote_desktop_view/src/view.rs`
- Modify: `crates/remote_desktop_view/src/view/render.rs`
- Modify: `main/src/home/home_tabs.rs`
- Modify: `crates/core/src/storage/models.rs`
- Modify: relevant settings/global state modules
- Add selection/fallback tests

- [ ] **Red:** capability probe cache、版本变化失效、explicit native no-fallback。
- [ ] **Red:** Auto 只在 pre-connect native-unavailable 分类回退。
- [ ] **Red:** 认证/证书/Gateway/服务端错误不得自动开第二个 canvas session。
- [ ] **Green:** app startup 或首次使用时执行轻量 probe。
- [ ] **Green:** presentation factory 创建 native，失败时按 contract fallback。
- [ ] **Green:** UI 显示当前 backend 和回退原因。
- [ ] **Green:** 提供“使用 Canvas 重试”动作。
- [ ] **Refactor:** capability cache 和 presentation lifecycle 分离。
- [ ] **Review:** 确认 fallback 不重复提交凭据、不产生双 session。
- [ ] **Verify:** 模拟 class not registered、接口缺失、显式 native、feature off。

**Acceptance:**

- Auto 在正常 Windows 优先 native。
- 系统组件不可用时仍可用现有 canvas。
- 显式选择有可预测语义。
- 非 Windows 无回归。

**Rollback:** 将默认 preference 保持 Canvas/feature off，不删除 native 代码。

#### Execution Notes (2026-08-10) — production presentation lifecycle wiring slice

- **Red evidence:** 新增纯函数 presentation factory 测试，先冻结非 Windows/显式
  Canvas 不 probe、不 create，Windows Auto available 只 create 一次且顺序稳定，
  已知 create-time native unavailable 只允许 Auto 回退，未知 create error 和显式
  Windows Native create error 都保留原始错误且不回退。新增 presentation runtime
  state 测试，冻结只有 `Canvas` 状态允许启动现有 canvas backend，`Pending`、
  `Native`、`Failed` 全部 fail closed。公开 endpoint parser 前新增 host/IPv4/
  bracketed IPv6、missing port、empty host 和 invalid port 测试。production source
  contract 继续冻结 render 中 presentation selection 先于 pending canvas start、
  proxy 检查先于 native host create、native create 后
  bounds → endpoint → options → connect → attach → `Native` 的顺序、post-create
  failure 不得转 Canvas，以及 activate/deactivate/attach 的 deferred-focus active
  gate。
- **Green implementation:** `RemoteDesktopView` 现在为 RDP 从 `Pending` 开始，为
  VNC 直接进入 `Canvas`；`start_runtime` 在最终入口检查 presentation state，避免
  native 和 canvas 同时创建 session。Windows feature-on production path 使用
  capability probe 和 pure factory 创建 `WindowsNativeAdapter`；Auto 只对
  `REGDB_E_CLASSNOTREG`/`E_NOINTERFACE` 对应的已知 create-time unavailable reason
  回退 Canvas，显式 Windows Native 永不静默降级。native host 创建成功后应用
  physical bounds，解析 destination，构造 host/port/初始尺寸/`Bpp32` options 并调用
  `connect`；layout、endpoint、options 或 connect 任一失败都会 force-close 已创建
  host、进入 `Failed`，不会再创建 canvas session，也不会在后续 render 重试。
  `parse_destination` 从 canvas backend 的私有 helper 提升为
  `remote_desktop` 公共 API，保持既有 destination 语义。
- **Proxy boundary:** 现有连接模型中的 SOCKS/HTTP proxy 不等同 RD Gateway；native
  path 在创建 child/session 前检测到 proxy 即返回专门的
  `ProxyUnsupported`，classifier 明确返回 `None`，Auto 和显式 Native 都 fail
  closed，并显示本地化的“改用 Canvas”提示。当前没有静默忽略 proxy 后直接 native
  连接，也没有把 server/proxy password 当作 Gateway password。
- **Tab lifecycle:** view 保存显式 `tab_active`。activate 先标记 active，再
  apply bounds/show，并在下一 UI turn focus；deactivate 先清 active，再 focus
  parent/hide。若 native adapter 在 tab 已 active 后才 attach，同样立即 activate，
  再 deferred focus；两个 deferred closure 都会重新检查 `tab_active`，快速切 tab
  不会让已失活 child 抢回 focus。
- **Refactor/review:** presentation selection/create contract 与 capability cache
  继续分离；native generation 使用进程内单调 allocator。两轮独立只读审查未发现
  可确认的高/中严重度双 session、fallback timing、cleanup、borrow/type 或
  tab-focus 问题。审查发现非 Windows build 会因移除 staged module 的
  `dead_code` allowance 产生 warning，现已恢复为带原因的 module-scoped allowance；
  production 使用点仍由 Windows cfg 和 source contract 覆盖。
- **Automated verification:** macOS host 上
  `rtk cargo fmt --all --check` 通过；`remote_desktop` tests 为 98 passed；
  `remote_desktop_view` 默认 tests 为 130 passed，feature-on tests 为 140 passed；
  `windows_rdp_host` tests 为 107 passed；`windows-rdp-probe --test contract` 为
  11 passed；`one-core` 为 425 passed / 3 ignored；`main` 默认和
  `windows-native-rdp` feature check 均通过；`rtk git diff --check` 通过。
  `remote_desktop_view` 默认/feature-on 的 `--no-deps` Clippy 在只豁免仓库已有
  `frame_sync.rs` `derivable_impls` 后通过，`windows_rdp_host --no-deps` Clippy
  通过。未带豁免的 workspace-dependent Clippy 仍被本切片之外的既有
  `agent_runtime` `large_enum_variant`/`unnecessary_sort_by`、`ssh`
  `result_large_err` 以及 `frame_sync.rs` `derivable_impls` 阻断，不能记录为全量
  Clippy clean。
- **Manual/Windows verification pending:** 本地 macOS 构建不能编译 MSVC/ATL/
  mstscax type-library C++，也不证明 Windows-only GPUI borrow/type/link。GitHub
  Windows x64/x86 runner 将在本提交推送后执行；其成功最多证明 MSVC/ATL/
  type-library/C++/Rust compile/link 和非交互 native tests，不证明有交互桌面的
  ActiveX create/show/focus/connect。真实 Windows desktop smoke、RDP server、
  DPI/tab focus 和 fallback negative matrix 仍未完成。
- **Known limitations:** 本切片的 native basic connect 只接入 endpoint、初始 display
  size 和 color depth；username/domain/password、NLA/CredSSP、credential prompt 和
  安全 buffer lifecycle 仍属于 Task 9，不能声称 native authentication 已完成。
  native callback queue → GPUI 普通 event/UI reducer 尚未 production 接线，close
  poll 仍是当前 destructive native event drain owner。当前 backend/fallback reason
  UI、“使用 Canvas 重试”动作、authentication/certificate/Gateway/server failure
  全部 negative cases 以及 capability cache 版本变化失效仍待后续 Task 8/9 切片。
  因此本记录不勾选 Task 8 整体完成。

---

### Task 9: username/domain/password、NLA/CredSSP 与凭据安全

**Goal:** 完成生产可用身份验证，不泄漏凭据，不进行隐式不安全降级。

**Depends on:** Task 8。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop connection form/settings files
- Modify: `crates/windows_rdp_host/src/options.rs`
- Modify: `crates/windows_rdp_host/native/configuration.cpp`
- Modify: `crates/windows_rdp_host/native/lifecycle.cpp`
- Add credential/security tests

**Contract:**

- username、domain 可持久化。
- password/Gateway password 只通过安全凭据存储获取。
- 默认启用 NLA/CredSSP。
- 不支持或失败时显示原因；只有用户显式切换策略才允许兼容模式。
- 系统 credential prompt 的 parent 是 Navop window。
- 默认禁止 ActiveX 自行长期保存凭据。

- [ ] **Red:** password 不出现在 `Debug`、serde JSON、event、error、log snapshot。
- [ ] **Red:** UTF-16 password buffer 写入 ActiveX 后被清零。
- [ ] **Red:** NLA failure 不自动关闭 CredSSP 重试。
- [ ] **Green:** 映射 `UserName`、`Domain`、write-only `ClearTextPassword`。
- [ ] **Green:** 设置 `EnableCredSspSupport`、prompt policy、credential saving policy。
- [ ] **Green:** prompt owner 使用 `UIParentWindowHandle`。
- [ ] **Refactor:** 使用 `SecretString`/zeroizing wrapper，缩短明文生命周期。
- [ ] **Review:** 安全审查 credential source、FFI copy、C++ BSTR 生命周期、日志。
- [ ] **Verify:** 本地账号、domain 账号、错误密码、密码过期、NLA on/off。

**Acceptance:**

- 默认安全策略连接成功。
- 凭据失败有明确诊断。
- 无秘密泄漏。
- 不安全兼容模式有警告且按连接保存。

**Rollback:** 若无密码注入时仍可系统 prompt，暂时只支持 prompt；不得降低安全默认值。

---

### Task 10: 键鼠、IME、快捷键与只读模式

**Goal:** 让 ActiveX 原生输入与 Navop 快捷键、焦点和 read-only 语义一致。

**Depends on:** Task 9。

**Files:**

- Modify: `crates/remote_desktop_view/src/view/input.rs`
- Modify: `crates/remote_desktop_view/src/view/render.rs`
- Modify: `crates/remote_desktop_view/src/view/windows_native.rs`
- Modify: `crates/windows_rdp_host/native/host.cpp`
- Add focus/input tests

**Input ownership:**

- native active/focused：ActiveX 处理键盘、鼠标、滚轮、IME。
- canvas：保留现有 GPUI input translation。
- read-only：native child 不获得交互输入，或 host window 在 message boundary 丢弃输入；不能仅隐藏 toolbar。
- Navop 保留一组明确的 focus-release/tab navigation shortcuts。
- host window 明确处理/验证 `WM_MOUSEACTIVATE`、`WM_SETFOCUS`、`WM_KILLFOCUS` 和 accelerator translation；不能依赖偶然的默认窗口过程行为。
- Task 1/10 冻结 ActiveX keyboard hook mode 与 Navop global shortcut 的优先级：安全 focus-release shortcut 始终可用，其余组合键在 RDP focused 时按用户选项发送远端。
- IME/TSF composition window 必须跟随 child bounds 和当前 DPI；失活/隐藏时结束或迁移 composition，不能留在其他 tab 上。

- [ ] **Red:** native presentation 激活时 canvas sender 不收到同一 input。
- [ ] **Red:** read-only 下远端无键鼠输入，仍允许复制/查看和 Navop 导航。
- [ ] **Red:** focus released event 能把焦点交回 tab/container。
- [ ] **Red:** `WM_MOUSEACTIVATE`/focus message、accelerator、IME composition 和 tab deactivate 顺序 contract。
- [ ] **Green:** 实现 active presentation input routing。
- [ ] **Green:** 处理 mouse activate、tab focus、release focus key、Alt/Windows/组合键策略。
- [ ] **Green:** 验证 Unicode 输入、中文/日文 IME composition、dead key、AltGr。
- [ ] **Refactor:** 将快捷键策略写成表驱动 contract。
- [ ] **Review:** 检查全局快捷键与远端快捷键冲突，避免截获密码输入。
- [ ] **Verify:** 物理键盘、不同布局、鼠标滚轮、高精度触控板、IME、read-only。

**Acceptance:**

- 无双重输入。
- 主要键鼠和 IME 工作。
- 用户总能返回 Navop。
- read-only 真实阻止远端输入。

**Rollback:** 首期可限制高级 IME/特殊组合键并明确标记，但普通键鼠和 focus release 必须通过。

---

### Task 11: 动态分辨率、缩放、DPI 与 resize 稳定性

**Goal:** 在窗口和显示器变化时保持清晰画面、正确鼠标映射和稳定远端分辨率。

**Depends on:** Task 10。

**Files:**

- Modify: `crates/remote_desktop_view/src/view/output.rs`
- Modify: `crates/remote_desktop_view/src/view/resize.rs`
- Modify: `crates/remote_desktop_view/src/view/windows_native.rs`
- Modify: `crates/windows_rdp_host/native/display.cpp`
- Add display math and Windows integration tests

**Strategy:**

- 本地 child bounds 每帧/每次 layout 变化及时更新。
- 远端 display update debounce/coalesce。
- 优先 `UpdateSessionDisplaySettings`/`SyncSessionDisplaySettings`。
- 能力不支持时按 policy 使用 `Reconnect(width,height)` 或固定远端分辨率 + smart sizing/letterbox。
- 设定最小/最大尺寸和合法 orientation/scale factor。

- [ ] **Red:** logical/physical/client-coordinate conversion property tests。
- [ ] **Red:** resize storm 只应用最新 generation/size。
- [ ] **Red:** 跨 monitor DPI change 不双重缩放。
- [ ] **Green:** 实现 child `SetWindowPos` 和远端 display update 两条独立路径。
- [ ] **Green:** 处理 zero/minimized bounds，不发送非法远端尺寸。
- [ ] **Green:** 能力不足时使用明确 fallback。
- [ ] **Refactor:** bounds math、debounce policy、COM call adapter 分离。
- [ ] **Review:** 检查旋转、scale factor、scrollbar、fullscreen 恢复尺寸。
- [ ] **Verify:** 100/125/150/200%，4K，跨显示器拖动，快速 resize/minimize/restore。

**Acceptance:**

- 画面清晰，无持续抖动和 resize loop。
- 鼠标位置正确。
- inactive tab 不触发无意义远端 resize。
- 不支持动态分辨率时有稳定降级。

**Rollback:** 固定初始分辨率并显示重新连接提示；不得产生不断 reconnect。

---

### Task 12: 文本剪贴板

**Goal:** 提供可关闭、可诊断的双向文本剪贴板。

**Depends on:** Task 11。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop connection form/settings files
- Modify: `crates/windows_rdp_host/native/redirection.cpp`
- Modify: `crates/windows_rdp_host/src/options.rs`
- Add clipboard tests

- [ ] **Red:** clipboard disabled 时不会向 ActiveX 开启重定向。
- [ ] **Red:** 文本 round-trip 覆盖 Unicode、CRLF、空字符串、大文本边界。
- [ ] **Red:** 剪贴板内容不进入日志/telemetry。
- [ ] **Green:** 映射 clipboard redirection 开关。
- [ ] **Green:** 在支持的控件版本上探测 manual clipboard controller；不把它作为基础同步的硬依赖。
- [ ] **Green:** UI 显示剪贴板已开启/关闭和安全提示。
- [ ] **Refactor:** 模型中区分文本和文件 capability；若底层只有统一开关，UI 必须按统一安全边界呈现。
- [ ] **Review:** 检查远端断开、tab inactive 和 reconnect 后状态。
- [ ] **Verify:** 中英文/emoji/多行/大文本双向复制粘贴。

**Acceptance:**

- 默认策略与产品安全决定一致。
- 用户可按连接关闭。
- 文本复制稳定且不泄漏。

**Rollback:** 遇到系统版本兼容问题时可默认关闭并提示，不影响连接。

---

### Task 13: 音频播放

**Goal:** 把现有音频播放模型完整映射到 ActiveX。

**Depends on:** Task 11。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop audio settings UI
- Modify: `crates/windows_rdp_host/native/configuration.cpp`
- Modify: `crates/windows_rdp_host/src/options.rs`
- Add audio option mapping tests

**Modes:**

- 在本机播放。
- 留在远端播放。
- 禁用。

- [ ] **Red:** 三种现有 model 值到 native setting 的 mapping test。
- [ ] **Red:** 旧配置 migration 不改变现有用户意图。
- [ ] **Green:** 在 Connect 前设置 audio redirection mode。
- [ ] **Green:** UI 在 native/canvas 两个 backend 使用同一业务枚举。
- [ ] **Refactor:** backend-specific 数值只存在 adapter。
- [ ] **Review:** 检查 reconnect 后设置仍生效。
- [ ] **Verify:** 测试系统声音、语音、静音切换和设备变化。

**Acceptance:**

- 三种模式行为明确。
- 不因 backend 不同出现配置语义漂移。

**Rollback:** native 不支持某模式时返回 capability warning 并禁用该选项，不静默替换模式。

---

### Task 14: 自动重连、手动重连与状态恢复

**Goal:** 用统一策略协调 ActiveX 内建重连事件和 Navop UI，不创建重连风暴。

**Depends on:** Task 5、Task 9、Task 13。

**Files:**

- Modify: `crates/remote_desktop_view/src/view.rs`
- Modify: relevant connection status/toolbar files
- Modify: `crates/windows_rdp_host/native/configuration.cpp`
- Modify: `crates/windows_rdp_host/src/event.rs`
- Add reconnect policy tests

**Rules:**

- 网络断开可自动重连。
- 认证、证书、Gateway policy 和用户关闭不自动重连。
- 同一时刻只有一个 reconnect owner；优先使用 ActiveX 内建机制并由 Navop展示状态。
- 手动重连会增加 generation，旧事件失效。
- reconnect 恢复 display、clipboard、audio、redirection 和 visibility 状态。

- [ ] **Red:** 用户关闭不重连；认证失败不重连；network failure 按 backoff 重连。
- [ ] **Red:** ActiveX auto-reconnect 与 Navop timer 不会同时发起。
- [ ] **Red:** generation change 丢弃旧 reconnect completion。
- [ ] **Green:** 映射 auto reconnect events 和 retry UI。
- [ ] **Green:** 实现 cancel/manual reconnect。
- [ ] **Green:** 网络恢复后同步 display settings。
- [ ] **Refactor:** reconnect policy 纯函数化，event handler 只执行决策。
- [ ] **Review:** 检查 sleep/resume、server reboot、长断网。
- [ ] **Verify:** 拔网/禁网、服务器重启、Wi-Fi 切换、sleep/resume。

**Acceptance:**

- 不出现无限快速重连。
- 用户能看到 attempt、原因并取消。
- 手动关闭始终终止 session。

**Rollback:** 关闭自动重连，仅保留手动按钮；不得保留双 owner。

---

### Task 15: 全屏、窗口切换与最小化/恢复

**Goal:** 在 Navop 窗口和 ActiveX full-screen 行为之间建立可预测 contract。

**Depends on:** Task 11、Task 14。

**Files:**

- Modify: `crates/remote_desktop_view/src/view/render.rs`
- Modify: `crates/remote_desktop_view/src/view/windows_native.rs`
- Modify: relevant app/window fullscreen modules
- Modify: `crates/windows_rdp_host/native/host.cpp`
- Modify: `crates/windows_rdp_host/native/event_sink.cpp`
- Add fullscreen state tests

**Preferred policy:**

- 优先 container-handled fullscreen。
- Navop 自己控制主窗口全屏和 child bounds。
- ActiveX full-screen request 通过事件通知容器，不创建难以控制的游离体验。
- 退出全屏恢复原窗口、tab、sidebar 和 focus。

- [ ] **Red:** enter/leave/minimize events 驱动单一 fullscreen state。
- [ ] **Red:** 快速切换和重复事件幂等。
- [ ] **Green:** 配置 container-handled fullscreen 和 event handlers。
- [ ] **Green:** 进入全屏时保存 layout，退出时恢复。
- [ ] **Green:** 最小化/恢复时 hide/show 和 display sync 正确。
- [ ] **Refactor:** fullscreen state 不直接散布 Win32 calls。
- [ ] **Review:** 检查多显示器、connection bar、系统快捷键。
- [ ] **Verify:** toolbar、快捷键、远端 request、最小化、Alt+Tab、锁屏恢复。

**Acceptance:**

- 不产生 orphan top-level RDP window。
- 能可靠退出全屏。
- tab 和窗口状态恢复正确。

**Rollback:** 第一版隐藏 ActiveX connection bar 并仅允许 Navop 全屏按钮；仍需保证退出路径。

---

### Task 16: 证书验证、认证警告与系统托管信任策略

**Goal:** 建立严格、可解释、可审计的服务器身份验证策略。

**Depends on:** Task 1 的 public-key API 降级结论、Task 9、Task 15。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop security settings UI
- Modify: `crates/windows_rdp_host/native/configuration.cpp`
- Modify: `crates/windows_rdp_host/native/event_sink.cpp`
- Modify: `crates/windows_rdp_host/src/event.rs`
- Add certificate policy/redaction tests

**Policy:**

- 默认 `Strict`。
- 不可信/名称不匹配/过期证书显示 ActiveX 系统警告或 Navop 受控提示。
- Task 1 已确认公开 `NotifyTSPublicKey` 属性被标记为 unsupported；首个 GA 只提供系统 `Strict`/系统提示，不存储或展示自定义 TS public-key pin。
- `OnReceivedTSPublicKey` 事件的存在不覆盖 unsupported 的配置入口，也不构成可承诺的 pre-connect pinning contract。
- 系统认证警告窗口必须正确 owner 到 Navop。

- [ ] **Red:** Strict 遇到自签名证书不自动继续。
- [ ] **Red:** certificate data 日志只保留允许字段。
- [ ] **Red:** UI 和持久化模型不展示或保存自定义 TS public-key pinning capability。
- [ ] **Green:** 配置 authentication level/相关 non-scriptable capability。
- [ ] **Green:** 处理 authentication warning displayed/dismissed、owner、取消、超时和关闭状态。
- [ ] **Refactor:** 证书策略与 ActiveX 对话框 adapter 分离。
- [ ] **Review:** 安全审查系统信任流程、domain/Gateway endpoint 区分及日志脱敏。
- [ ] **Verify:** 受信任、自签名、过期、名称不匹配、证书轮换、中间人场景。

**Acceptance:**

- 默认不绕过证书错误。
- 用户能看懂风险和 endpoint。
- UI 不展示或暗示自定义 TS public-key pinning。

**Rollback:** 只保留系统 ActiveX 安全对话框和 Strict；不得加入全局忽略开关。

---

### Task 17: RD Gateway 与现有 SOCKS5/HTTP proxy 策略

**Goal:** 支持微软 RD Gateway，并明确它与现有通用网络代理的关系。

**Depends on:** Task 9、Task 16。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop connection form/settings files
- Modify: `crates/windows_rdp_host/native/configuration.cpp`
- Modify: `crates/windows_rdp_host/src/options.rs`
- Add gateway mapping/security tests

**Contract:**

- `RdpGatewaySettings` 独立于 SOCKS5/HTTP proxy。
- 支持 Gateway host、usage method、credential source、username/domain/password 和系统 prompt。
- server credentials 与 Gateway credentials 可相同或不同。
- native backend 直接使用 `TransportSettings*`。
- 如果产品要在 native backend 同时支持 SOCKS/HTTP proxy，必须先证明 mstscax 有受支持路径；否则 UI 明确互斥/不支持，不做透明 socket hook。

- [ ] **Red:** Gateway config serde/migration 和 secret redaction。
- [ ] **Red:** proxy 与 Gateway 组合按 capability matrix 返回明确结果。
- [ ] **Green:** 映射 `TransportSettings`/`TransportSettings4` 的 host、usage、profile/credential properties。
- [ ] **Green:** 处理 Gateway credential prompt 和错误分类。
- [ ] **Green:** UI 分开显示“RD Gateway”和“网络代理”。
- [ ] **Refactor:** endpoint、server auth、Gateway auth 三类错误模型分离。
- [ ] **Review:** 安全审查 Gateway 证书和凭据生命周期。
- [ ] **Verify:** Gateway always/detect/never、相同/不同账号、错误证书、错误密码、不可达。

**Acceptance:**

- RD Gateway 可生产使用。
- 用户不会把它误认为 HTTP/SOCKS proxy。
- 组合不支持时在连接前阻止并解释。

**Rollback:** 保持 Gateway feature capability-gated；不影响直连和 canvas。

---

### Task 18: 文件/目录剪贴板能力验证与安全策略

**Goal:** 在文本剪贴板之外验证系统 ActiveX 对文件/目录复制的真实控制边界；只有能提供准确安全语义时才对用户开放。

**Depends on:** Task 1 的 file-clipboard API proof、Task 12、Task 16。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop clipboard/security settings UI
- Modify: `crates/windows_rdp_host/native/redirection.cpp`
- Add file clipboard integration tests

**Security contract:**

- 不提供独立 file clipboard toggle；文件/目录能力继承统一 `RedirectClipboard` 开关。
- 当前计划中的统一剪贴板默认开启，因此 UI 必须明确该开关可能同时允许文本和文件/目录传输；用户关闭时两者一起关闭。
- drive/shared-folder redirection 仍使用独立授权，不得与 clipboard redirection 混为一谈。
- 显示来源和目标风险。
- 大文件/目录、取消、重连和路径冲突有明确行为。
- 不把文件内容加载到 Navop 日志或普通内存快照。
- 如果 Windows runtime 证明目标控件不支持可靠文件/目录复制，则将该能力标记为 unsupported；不得用未文档化 hook 伪造独立控制。

- [ ] **Red:** 统一 clipboard disabled/allowed policy tests，固定关闭时文本和文件/目录都不可重定向。
- [ ] **Red:** Unicode 文件名、长路径、目录、多个文件和冲突场景。
- [ ] **Red:** contract test 禁止新增独立 file clipboard toggle，并固定其与 `RedirectClipboard` 共用安全边界。
- [ ] **Green:** 统一 clipboard 开关文案明确包含文本和文件/目录；runtime 不支持文件传输时显示 unsupported。
- [ ] **Green:** UI 显示进行中、失败和安全开关。
- [ ] **Refactor:** clipboard redirection、drive redirection 与 shared-folder redirection 的 capability/policy 分离。
- [ ] **Review:** 检查路径遍历、symlink/reparse point、超大文件和取消。
- [ ] **Verify:** 本地↔远端复制单文件、多文件、目录、大文件、中文路径。

**Acceptance:**

- UI 和文档准确表达统一 `RedirectClipboard` 安全边界，不提供虚假的独立 file clipboard 开关。
- 统一开关关闭时文本和文件/目录都不会重定向；开启时用户已被告知可能包含文件/目录传输。
- ActiveX 无可靠文件复制路径时：功能明确标记 unsupported，文本剪贴板和 session 不受影响。

**Rollback:** 保留文本剪贴板；文件/目录复制标记 unavailable/unsupported，不使用未文档化 hook。

---

### Task 19: drive/shared-folder、printer、smart-card 与 audio capture 重定向

**Goal:** 完成常用企业资源重定向，并实施最小权限和逐项 capability。

**Depends on:** Task 17、Task 18。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop redirection settings UI
- Modify: `crates/windows_rdp_host/native/redirection.cpp`
- Modify: `crates/windows_rdp_host/src/capabilities.rs`
- Add mapping and Windows integration tests

**Sub-capabilities:**

1. 指定本地目录/驱动器；
2. 动态驱动器；
3. printer；
4. smart card；
5. microphone/audio capture。

- [ ] **Red:** 每项默认关闭、独立授权、serde migration。
- [ ] **Red:** 指定目录不能因 backend mapping 误变成“全部驱动器”。
- [ ] **Red:** unsupported capability 不静默开启。
- [ ] **Green:** 通过 advanced settings、drive collection 和 redirection objects 映射。
- [ ] **Green:** shared folder 复用现有业务模型但不扩大授权。
- [ ] **Green:** 配置 microphone/audio capture，处理设备不存在。
- [ ] **Green:** 重连后恢复授权。
- [ ] **Refactor:** 每类 redirection 使用独立 adapter/option，不创建巨型函数。
- [ ] **Review:** 安全审查路径、printer data、smart-card PIN、麦克风隐私。
- [ ] **Verify:** 本地磁盘/目录、USB disk hotplug、printer、smart-card test environment、microphone。

**Acceptance:**

- 每项可单独开启/关闭。
- UI 显示真实 capability。
- 敏感权限有警告。
- 不会将全部本地资源意外暴露给远端。

**Rollback:** 逐项 capability-gate；某项失败不阻断基本 RDP。

---

### Task 20: 多显示器与 session takeover/session selection

**Goal:** 支持多显示器桌面和服务端要求的 session 选择/接管流程。

**Depends on:** Task 15、Task 19。

**Files:**

- Modify: `crates/core/src/storage/models.rs`
- Modify: remote desktop display settings UI
- Modify: `crates/windows_rdp_host/native/display.cpp`
- Modify: `crates/windows_rdp_host/native/event_sink.cpp`
- Modify: `crates/windows_rdp_host/src/event.rs`
- Add multimon/session flow tests

**Multimon rules:**

- 默认关闭。
- 使用 `UseMultimon` 和 remote monitor capability。
- 检查本地 monitor layout 是否满足控件/服务端要求。
- 离开多显示器恢复单 tab bounds。
- 连接前发现 topology 不支持：阻止 multimon，提示并以单显示器继续，不能先创建第二个 session。
- 已连接单显示器的普通 resize：使用 Task 11 dynamic display update，不切换为 multimon。
- 已连接 multimon 后发生显示器热插拔/不支持布局：冻结当前 session，提示“保持现状 / 断开后以单显示器重连”；不得静默并行重连。
- 用户确认降级时增加 generation，先关闭旧 session，再创建单显示器 session；不允许两个 session 同时持有同一凭据和资源重定向。

- [ ] **Red:** single/multimon state transitions 和 unsupported monitor layout。
- [ ] **Red:** monitor disconnect/reconnect 不产生 orphan fullscreen window。
- [ ] **Red:** 连接前拒绝、普通 resize、已连接热插拔三类失败策略各自有确定结果且不创建双 session。
- [ ] **Green:** 探测 monitor count/bounding box/layout match。
- [ ] **Green:** 配置多显示器并处理进入/退出事件。
- [ ] **Green:** 对 session selection/takeover 事件或系统提示设置正确 owner 和 UI state。
- [ ] **Refactor:** monitor topology snapshot 与 ActiveX adapter 分离。
- [ ] **Review:** 检查不同 DPI、主显示器切换、显示器热插拔。
- [ ] **Verify:** 2+ 显示器、不同缩放、排列变化、断开显示器、已有 session。

**Acceptance:**

- 支持的 topology 可用多显示器。
- 不支持时连接前给出说明并回到单显示器。
- 已连接后 topology 变化由用户确认是否断开并单显示器重连，不静默创建第二个 session。
- session 接管提示不会藏在 Navop 后面。

**Rollback:** 首个稳定版可保留单显示器默认，多显示器 feature capability-gated。

---

### Task 21: overlay、dialog parent、clipping 与 z-order 正式方案

**Goal:** 解决 native child window 与 GPUI overlay 的根本冲突，达到可发布质量。

**Depends on:** Task 6、Task 15、Task 16、Task 17、Task 20。

**Files:**

- Modify: `crates/remote_desktop_view/src/view/windows_native.rs`
- Modify: relevant GPUI overlay/menu/dialog integration modules
- Modify: `crates/windows_rdp_host/native/host.cpp`
- Add native overlay coordinator module if required
- Add visual/manual regression checklist

**Required scenarios:**

- tooltip；
- context menu；
- connection options dropdown；
- modal confirmation；
- certificate/Gateway/credential system dialog；
- command palette；
- notification/toast；
- tab drag；
- sidebar overlay；
- fullscreen toolbar。

**Win32 contract:**

- child 和 clipping region 使用 parent client physical-pixel coordinates；DPI change 后重新计算 region。
- `SetWindowPos`/`SetWindowRgn` 或选定的等价策略必须在 GPUI prepaint/layout commit 后应用。
- 系统 credential/certificate/Gateway/session prompt 使用 Navop window 的 owner/`UIParentWindowHandle`。
- overlay token active 时，host 忽略/协调 `WM_MOUSEACTIVATE`、`WM_SETFOCUS`，避免 ActiveX 抢回 focus。
- hide/clip 期间处理 `WM_KILLFOCUS` 和 IME/TSF composition；恢复时不能无条件偷走用户当前 focus。
- parent top-level window 重建或 DPI awareness context 变化时销毁并重建 child，不跨 owner `SetParent`。

- [ ] **Red:** overlay coordinator state tests：open→hide/clip/native popup→restore；nested overlay；tab deactivation。
- [ ] **Red:** system dialog owner test，确保 modal dialog 位于 Navop 前面。
- [ ] **Red:** physical-pixel clipping region、DPI change、parent window rebuild 和 focus/IME message 顺序 contract。
- [ ] **Green:** 选定并实现正式组合策略。
- [ ] **Green:** overlay 打开时阻止 ActiveX 抢回 focus。
- [ ] **Green:** overlay 关闭后仅在 tab 仍 active 且用户需要时恢复 child/focus。
- [ ] **Green:** 对 content clipping 使用真实 Win32 region/bounds，不依赖 GPUI clip。
- [ ] **Refactor:** overlay coordinator 使用 token/guard，避免不同 popup 互相错误恢复。
- [ ] **Review:** UI/Windows platform 双人审查。
- [ ] **Verify:** 对 required scenarios 逐项录屏/截图验证，覆盖 100/150/200% DPI。

**Acceptance:**

- GPUI 关键菜单和对话框不被 RDP 画面盖住。
- 系统对话框不会游离或藏到主窗口后。
- nested overlay 和快速关闭无闪烁/焦点死锁。

**Release gate:** 未达到 acceptance 时 `windows-native-rdp` 不得默认开启或标记 GA。

**Rollback:** feature 保持 opt-in；必要时 overlay 打开期间临时隐藏 native child，优先正确性。

---

### Task 22: 设置 UI、诊断 UI、locales 与持久化

**Goal:** 让全部 native 功能可发现、可配置、可解释，并保持旧连接兼容。

**Depends on:** Task 8、Task 9、Task 10、Task 11、Task 12、Task 13、Task 14、Task 15、Task 16、Task 17、Task 18、Task 19、Task 20、Task 21。

**Files:**

- Modify: remote desktop connection form files
- Modify: remote desktop settings/toolbar/status files
- Modify: `crates/core/src/storage/models.rs`
- Modify: locale resources under existing i18n directories
- Add UI/state/serialization tests

**UI sections:**

1. Backend：Auto / Windows Native / Canvas。
2. Security：NLA/CredSSP、certificate policy、prompt behavior。
3. Display：resolution、dynamic resize、scale、fullscreen、multimon。
4. Local resources：clipboard text/file、audio play/capture、drives/folders、printer、smart card。
5. RD Gateway。
6. Reconnect。
7. Diagnostics：backend、capabilities、OS/control version、last error、copy sanitized report。

- [ ] **Red:** 所有新字段 serde default/round-trip/secret exclusion。
- [ ] **Red:** capability-driven disable/tooltip 行为。
- [ ] **Red:** backend 切换不丢失跨 backend 通用设置。
- [ ] **Green:** 实现分组 UI 和安全默认值。
- [ ] **Green:** 连接中只允许安全的 runtime setting；其余标记“下次连接生效”。
- [ ] **Green:** 增加中英文 locales，不在代码硬编码用户文案。
- [ ] **Green:** sanitized diagnostic report 可复制。
- [ ] **Refactor:** form state、persistent model、runtime options 使用显式转换。
- [ ] **Review:** UX、安全、i18n 审查。
- [ ] **Verify:** 新建/编辑/复制连接，重启应用，旧配置，非 Windows UI。

**Acceptance:**

- 所有 GA 能力都有可发现的配置或明确自动策略。
- 不支持项根据 capability 禁用并解释。
- 旧配置兼容。
- 诊断报告不含秘密。

**Rollback:** 高级项可隐藏在 experimental section，但安全、backend、fallback 和诊断不可缺失。

---

### Task 23: x64/x86 CI、MSI/EXE/ZIP 打包与真机矩阵

**Goal:** 把 native 依赖纳入可重复构建、安装和发布流程。

**Depends on:** Task 22。

**Files:**

- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/build-windows-msi.yml`
- Modify: `.github/workflows/upload-r2.yml` if artifact contract changes
- Modify: `script/install-window.ps1`
- Modify: `script/test-release-packaging.mjs`
- Modify: `installer/windows/navop.wxs` only if new runtime files exist
- Modify: `.cargo/config.toml` if required
- Add Windows smoke scripts/tests

**Build contract:**

- x64/x86 MSVC。
- ATL component installed。
- C++ warning-as-error。
- static C++ runtime/link strategy明确。
- shim 默认静态进入 `navop.exe`。
- 不打包 `mstscax.dll`。
- 如果实际产生额外 DLL，ZIP、portable ZIP、MSI、Burn EXE、R2 asset contract 全部同步。

- [ ] **Red:** packaging contract 在 native artifact 缺失/意外 DLL 出现时失败。
- [ ] **Red:** x86/x64 ABI/layout tests 分别运行。
- [ ] **Green:** CI feature-on check/test。
- [ ] **Green:** release x64/x86 build 和安装包 smoke。
- [ ] **Green:** 有桌面 self-hosted runner/VM 执行 create/show/hide/destroy。
- [ ] **Green:** clean Windows 10/11 x64 WoW64 安装后用 Navop x86 主进程验证 32-bit COM 注册视图、`MsRdpClient12` create/query、connect/disconnect、fallback 和卸载；若支持 Windows 10 x86 OS，再执行同等矩阵。
- [ ] **Green:** 记录精简系统、Server Core、Wine 为 unsupported/unavailable 的预期诊断。
- [ ] **Refactor:** probe/smoke 脚本可在开发机和 CI 共用。
- [ ] **Review:** installer、license、系统组件分发和 updater contract。
- [ ] **Verify:** clean Windows 10/11 x64 native、同系统 WoW64 x86 进程，以及产品声明支持时的 Windows 10 x86 OS，完成安装、升级、卸载；执行 `ClassNotRegistered`/受限系统、显式 Native 不回退、Auto 回退、回退后不提交凭据/不创建双 session 的负例。

**Acceptance:**

- x64/x86 release artifacts 可安装运行。
- x64/x86 都通过安装后真实 CoClass/interface probe、create/destroy 和 connect/disconnect。
- native feature 和 fallback 正例/负例在干净系统表现正确。
- 不遗漏 runtime file，不错误打包系统 DLL。
- CI 失败能明确定位 ATL/toolchain/ABI 问题。

**Rollback:** 发布构建关闭 feature，保持现有 Windows artifact；无需回滚数据模型。

---

### Task 24: 性能、泄漏、灰度、发布、回滚与最终验证

**Goal:** 完成生产发布所需的稳定性证据、灰度策略和回滚开关。

**Depends on:** Task 23；Gate C 已通过。

**Files:**

- Modify: this plan with execution evidence
- Modify: `CHANGELOG.md`
- Modify: user/developer documentation
- Modify: diagnostics/telemetry modules as approved
- Add release checklist and long-run scripts

**Metrics:**

- native host create latency；
- connect/login latency；
- resize rate和失败；
- reconnect count；
- native unavailable/fallback rate；
- disconnect/error category；
- HWND/GDI/USER handle delta；
- process memory after repeated sessions；
- crash/hang rate；
- 不采集 host、username、clipboard、文件名、密码。

- [ ] **Red:** kill switch/feature override test，确保无需发新版本即可回到 canvas（若产品配置系统支持 remote flag）。
- [ ] **Red:** telemetry redaction/schema tests。
- [ ] **Green:** 运行 8h session、100 connect/disconnect、100 tab open/close、sleep/resume、network flap。
- [ ] **Green:** 运行 x64/x86、DPI、Gateway、证书、重定向、多显示器矩阵。
- [ ] **Green:** 建立灰度：internal → opt-in preview → small percentage/explicit default → GA。
- [ ] **Green:** 编写用户排障：系统组件不可用、切换 Canvas、日志位置、诊断报告。
- [ ] **Green:** 编写开发文档：STA、ABI、ActiveX version、Windows test setup。
- [ ] **Refactor:** 删除 spike/debug code，固定公开 contract。
- [ ] **Review:** Rust、Windows、UI、安全、release 五类最终审查。
- [ ] **Verify:** 执行第 13 节全部命令、真机矩阵、fallback 负例、回滚演练和长稳/泄漏矩阵；在本计划附日志摘要、系统/控件版本和制品校验结果后才勾选完成。

**Acceptance:**

- release gates 全通过。
- 没有已知 blocker 级 crash、hang、overlay、focus、credential 或泄漏问题。
- fallback 和 kill switch 可用。
- CHANGELOG、用户文档、支持矩阵和排障完整。

**Rollback:**

1. remote/config kill switch 将 Auto 解释为 Canvas；
2. 下一补丁版本默认关闭 feature；
3. 保留配置字段和 canvas 数据兼容；
4. 不自动删除或迁移用户连接；
5. 收集脱敏 diagnostics 后再修复 native path。

---

## 9. Phase E 后续独立里程碑

这些任务不阻断“嵌入完整 Windows 桌面 RDP”的首个 GA，但必须按顺序另建设计/实施计划，不得直接在当前 host 中临时堆叠。

### Future Epic 25: RemoteApp 独立计划

**Prerequisites:** Task 21 overlay/z-order、Task 15 fullscreen、稳定的 event sink。

**Required scope:**

- `ITSRemoteProgram*` capability 和 launch contract；
- RemoteApp window displayed/result events；
- 本地窗口、taskbar、focus、activation、minimize、close；
- 多 RemoteApp window；
- connection bar；
- 错误/退出语义；
- 与整桌面 tab 的 UX 选择。

- [ ] 编写 RemoteApp architecture decision record。
- [ ] 证明 GPUI/Win32 window ownership 和 z-order。
- [ ] 再决定是在 tab 中组合窗口还是使用受控 native top-level windows。

### Future Epic 26: USB/PnP/摄像头/位置与 COM/LPT redirection 独立计划

**Prerequisites:** Task 19 redirection 权限模型和真机设备实验室。

**Required scope:**

- DeviceCollection/PnP 枚举和逐设备授权；
- dynamic devices/hotplug；
- camera config collection；
- location 2D/3D；
- USB 设备支持边界；
- COM/LPT；
- 隐私提示、管理员策略、审计；
- 设备断开/重连和多 session 冲突。

- [ ] 先定义支持设备清单，不能宣传 ActiveX 未提供的通用 USB passthrough。
- [ ] 建立真实硬件 matrix 和隐私审查。

### Future Epic 27: 自定义虚拟通道公开 API 独立计划

**Prerequisites:** 稳定 extension permission model、bounded IPC、session lifecycle。

**Required scope:**

- channel name 和数量限制；
- create/options/send/receive；
- 消息 framing、最大尺寸、背压、取消；
- extension 权限和 per-connection 授权；
- 恶意/崩溃 extension 隔离；
- session close cleanup；
- 与 canvas/IronRDP 虚拟通道能力对齐。

- [ ] 不直接把 raw COM callback 暴露给 extension。
- [ ] 先建立有界、版本化、可取消的 host API。

### Future Epic 28: Windows ARM64 独立支持计划

**Prerequisites:** ARM64 build runner、系统 ActiveX 注册验证、installer/updater asset contract。

**Required scope:**

- `aarch64-pc-windows-msvc` build；
- ATL ARM64 toolchain；
- ARM64 `mstscax.dll`/CoClass/interface 验证；
- native shim ABI；
- MSI/EXE/ZIP；
- updater/R2；
- 真机 Windows on ARM；
- x64 emulation 不得冒充原生 ARM64 支持。

- [ ] 完成 CI/release/installer 全链路后再更新支持声明。

---

## 10. 关键风险登记

| 风险 | 后果 | 缓解/阻断项 |
| --- | --- | --- |
| native HWND 覆盖 GPUI overlay | 菜单/对话框不可见 | Task 21 GA blocker |
| GPUI clipping 不裁剪 child HWND | 画面越界 | Win32 bounds/region 验证 |
| tab deactivate 未隐藏 | 覆盖其他 tab | Task 6 contract |
| 隐藏前未归还 focus | 键盘困在 hidden child | focus_parent → hide |
| ActiveX 与 GPUI 双输入 | 重复键鼠 | presentation 单一 input owner |
| logical/physical pixel 混用 | 模糊、偏移、鼠标错误 | Task 11 property tests |
| COM apartment 不一致 | crash/hang/RPC error | Task 1 spike + UI-thread assertion |
| callback 重入 destroy | use-after-free/deadlock | event queue + Closing gate |
| 未先 Unadvise | 销毁后 callback | Task 5/7 order contract |
| force close 绕过 try_close | 资源泄漏 | entity release/drop 兜底 |
| password 进入日志/缓冲残留 | 凭据泄漏 | zeroizing + redaction tests |
| x86/x64 注册表视图差异 | 某架构不可用 | 两架构真机 probe |
| CI 无真实 desktop | 编译通过但运行失败 | desktop VM smoke |
| 精简系统/Wine 无控件 | native 创建失败 | capability + canvas fallback |
| ActiveX 对话框 owner 错误 | 对话框藏在后台 | `UIParentWindowHandle` + Task 21 |
| Windows update 改变行为 | 回归 | 记录 OS/control version + smoke |
| native 初始化失败破坏 tab | 用户无法连接 | Auto pre-connect fallback |
| backend 切换重复凭据提交 | 安全/双 session | fallback time boundary |
| resize storm | CPU/网络/重连循环 | coalesce + generation |
| 敏感 redirection 默认开启 | 数据泄漏 | default off + per-item consent |
| 引入 native 破坏非 Windows | 跨平台回归 | feature isolation + CI |

---

## 11. 验收门与发布等级

### Gate A：技术可行

- [ ] Task 1-4 完成。
- [ ] x64/x86 创建、连接、销毁成功。
- [ ] 未使用外部 `mstsc.exe`/`SetParent`。

### Gate B：可供内部使用

- [ ] Task 5-14 完成。
- [ ] tab 生命周期、凭据、输入、DPI、剪贴板、音频和重连可用。
- [ ] feature 默认关闭。

### Gate C：Preview

- [ ] Task 15-22 完成。
- [ ] 证书、Gateway、常用重定向、多显示器可用。
- [ ] overlay/z-order blocker 已解决。
- [ ] 用户可显式 opt-in。

### Gate D：GA

- [ ] Task 23-24 完成。
- [ ] x64/x86 安装与真机矩阵通过。
- [ ] 长稳、泄漏、安全和回滚审查通过。
- [ ] 支持文档完整。

---

## 12. 每个 Task 的完成模板

每完成一个 Task，在对应章节补充：

```markdown
**Execution Notes (YYYY-MM-DD):**

- Red evidence:
- Green implementation:
- Refactor/review:
- Automated verification:
- Manual verification:
- Known limitations:
- Decision changes:
```

不得只勾选 checkbox 而不记录关键证据。

---

## 13. 最终验证命令

以下命令使用当前仓库真实 package/feature 名称。所有命令应在仓库根目录运行；后续 Task 引入新的 package、feature 或 release gate 时继续追加，不能恢复不存在的
`windows_rdp_host/windows-native-rdp` feature。

```bash
rtk cargo fmt --all --check
rtk cargo clippy --locked -p windows_rdp_host --all-targets -- -D warnings
rtk cargo test --locked -p windows_rdp_host
rtk cargo test --locked -p windows-rdp-probe --test contract
rtk cargo test --locked -p remote_desktop_view
rtk cargo test --locked -p remote_desktop_view --features windows-native-rdp
rtk cargo test --locked -p remote_desktop
rtk cargo test --locked -p one-core
rtk cargo check --locked -p main
rtk cargo check --locked -p main --features windows-native-rdp
rtk cargo clippy --locked -p remote_desktop_view --all-targets -- -D warnings
rtk cargo check --locked --workspace --all-targets
rtk node script/test-release-packaging.mjs
rtk git diff --check
```

Windows x64/x86 的当前 host ABI gate 必须在 Windows-hosted MSVC/ATL 环境执行：

```powershell
./script/build-windows-rdp-probe.ps1 `
  -Target x86_64-pc-windows-msvc,i686-pc-windows-msvc
```

脚本对每个 target 执行：

```powershell
cargo build --locked -p windows-rdp-probe --target $RustTarget
cargo test --locked -p windows_rdp_host --target $RustTarget
```

该 gate 不运行 `windows-rdp-probe` executable，因此 probe/ActiveX 部分仍是
compile/link-only；它会实际运行 `windows_rdp_host` 的非 ActiveX native host tests。
macOS 本地测试不能替代 Windows compiler/linker/native-test 结果；GitHub-hosted runner
的成功也不能替代有交互桌面的 ActiveX runtime smoke。

还必须执行：

- Windows x64/x86 desktop smoke。
- RDP server matrix。
- DPI/monitor matrix。
- overlay/dialog matrix。
- credentials/certificate/Gateway security matrix。
- long-run/leak matrix。

---

## 14. 官方技术依据

- [MsRdpClient12 class](https://learn.microsoft.com/en-us/windows/win32/termserv/msrdpclient12)：列出 `MsRdpClient12` 实现的 `IMsRdpClient10` 至旧版本接口、non-scriptable interfaces、事件、属性、DLL 和 CLSID。
- [IMsRdpClient10 interface](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclient10)：最高主客户端接口和 display/session methods。
- [IMsRdpClient9 interface](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclient9)：`UpdateSessionDisplaySettings`、`SyncSessionDisplaySettings` 等 display contract。
- [IMsRdpClientAdvancedSettings interface](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclientadvancedsettings-interface)：`NotifyTSPublicKey` 属性明确标记为 unsupported；事件页面不能把不受支持的配置入口提升为 GA pinning contract。
- [IMsRdpClientAdvancedSettings5::RedirectClipboard](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclientadvancedsettings5-redirectclipboard)：公开 clipboard redirection 总开关。
- [IMsRdpExtendedSettings::Property](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpextendedsettings-property)：包括 `ManualClipboardSyncEnabled`、Restricted Logon、Redirected Authentication 和 child-session 等扩展属性；均须 capability-gate。
- [IMsRdpClipboard](https://learn.microsoft.com/en-us/windows/win32/api/mstscax/nn-mstscax-imsrdpclipboard)：手动 clipboard 同步接口；不能据此宣称文本与文件/目录 clipboard 可独立授权。
- [IMsRdpClientAdvancedSettings6::ConnectToAdministerServer](https://learn.microsoft.com/en-us/windows/win32/termserv/imsrdpclientadvancedsettings6-connecttoadministerserver)：admin session 配置入口，不等同 arbitrary session-id attach。
- [Remote Desktop ActiveX control classes](https://learn.microsoft.com/en-us/windows/win32/termserv/remote-desktop-activex-control-classes)：系统 Remote Desktop ActiveX CoClass 清单。
- [ATL Composite Control Global Functions](https://learn.microsoft.com/en-us/cpp/atl/reference/composite-control-global-functions?view=msvc-170)：`AtlAxWinInit`、`AtlAxCreateControlEx`、`AtlAxGetControl` 和 ActiveX hosting functions。

实现时以目标架构系统注册的 Remote Desktop ActiveX type library 及其由 MSVC `#import` 生成的 declarations 为编译期事实；Windows SDK 不提供本计划可依赖的 `mstscax.h`。以目标机器 runtime capability probe 为运行期事实。文档页面、编译机器 type library 和目标机器系统控件版本可能不同；任何属性都必须经 `QueryInterface`/HRESULT 验证后再向 UI 宣称可用。

---

## 15. 当前仓库接入点

- `crates/remote_desktop_view/src/view.rs`：现有远程桌面 view/runtime 生命周期。
- `crates/remote_desktop_view/src/view/render.rs`：canvas render、TabContent 和关闭入口。
- `crates/remote_desktop_view/src/view/output.rs`：bounds/scale。
- `crates/remote_desktop_view/src/view/resize.rs`：resize/DPI。
- `crates/remote_desktop_view/src/view/input.rs`：键鼠/text input。
- `crates/core/src/tab_container.rs`：tab activate/deactivate/try_close/force-close 顺序。
- `crates/webview/src/lib.rs`：GPUI 中 child native view 的 bounds/show/hide/focus 先例。
- `crates/ai_chat_view/src/html_code_block.rs`：`build_as_child` 先例。
- `main/src/home/home_tabs.rs`：打开 RDP tab。
- `crates/core/src/storage/models.rs`：连接模型和已有 proxy/audio/read-only 字段。
- `crates/remote_desktop/src/config.rs`：当前 RDP runtime options。
- `crates/remote_desktop/src/helper_protocol.rs`：当前尺寸、DPI、音频、shared-folder、input、clipboard contract。
- `.github/workflows/ci.yml`：现有 Windows PR target。
- `.github/workflows/release.yml`：现有 Windows x64/x86 release targets。
- `.github/workflows/build-windows-msi.yml`：Windows MSI architecture matrix。
- `script/install-window.ps1`：Windows native toolchain 安装。
- `installer/windows/navop.wxs`：MSI files。
- `script/test-release-packaging.mjs`：release packaging contract。

---

## 16. 计划完成定义

本计划只有在以下条件全部满足时才算完成：

- [ ] Task 0-24 全部完成并记录证据。
- [ ] Gate A-D 全部通过。
- [ ] Windows x64/x86 支持声明与实际一致。
- [ ] native unavailable 时 canvas fallback 可靠。
- [ ] overlay、focus、DPI、COM lifecycle、force-close、event unadvise 无已知 blocker。
- [ ] 凭据、证书、Gateway、clipboard 和 redirection 完成安全审查。
- [ ] 安装、升级、卸载、updater 和 release artifact 验证通过。
- [ ] 长稳和泄漏验证通过。
- [ ] 用户文档、开发文档、排障和回滚方案发布。
- [ ] Phase E 能力已分别建立后续计划或被产品明确取消，不能以“以后再说”悬空。
