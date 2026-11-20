# 内置 yserver X11 转发设计

## 目标与边界

本期让 Windows 用户在不安装 VcXsrv 的情况下使用 X11 转发；macOS 继续引导 XQuartz。Linux 不纳入本期内置范围，仍使用本机 X server。远端 sshd、远端软件和现有 cookie 委托链路均不改变。

本期不实现 X server，不内嵌单窗口 tab，不把裸字节流塞入 universal-plugins。yserver 只作为原生 sidecar 子进程运行。`yserver-core` 不含窗口 shell、Vulkan 渲染和完整生命周期接口，不能靠直接链接 core 替代 sidecar；进程隔离也避免 yserver 崩溃或 GPU 异常带走 Navop。

## 解析优先级

设置提供 `Auto`、`External`、`Embedded` 三种持久化模式：

1. `Auto`：先探测本机 X server，失败后尝试内置组件；两者都不可用时返回带原因的停用状态。
2. `External`：只使用本机 X server。
3. `Embedded`：只使用内置组件，失败时明确提示可改用外部 server。

`crates/x11_component` 负责模式解析、清单、内嵌/下载安装、验签、版本目录、能力预检和进程监督；`crates/ssh` 只读取全局设置并取得解析结果。每个 `RusshClient` 保存自己的状态，避免并发会话相互覆盖。`X11Proxy::for_endpoint` 只替换本机 server 的来源，x11-req、回连通道、fake cookie、真实 cookie 改写和双向中继继续复用。

**本机 endpoint 与远端 DISPLAY 是两套语义。** 远端 DISPLAY 由 sshd 在 accepted x11-req 后分配；远端客户端通过 SSH 通道回连，不会把本机 TCP 端口号当作远端 display。本机使用 `127.0.0.1` 上随机 display 对应的 `6000 + display` 端口；发生冲突时在组件锁内轮换，不再宣称 yserver 支持任意 TCP 端口参数。

## 组件供应链与内嵌交付

组件版本为 `1.9.0-navop.1`，源码固定为 [feigeCode/yserver](https://github.com/feigeCode/yserver) 提交 `3b3c7f66fb9d9c80752af36b4948fad0b8760671`，发布物固定到 [navop-v1.9.0-1](https://github.com/feigeCode/yserver/releases/tag/navop-v1.9.0-1)。fork 基于上游 v1.9.0，只增加安全监听参数及 Windows 构建配置，不自动追踪 latest。

清单使用 schema 1，按 `windows-x86_64`、`windows-aarch64` 选择 artifact。artifact 包含版本、URL、SHA-256、Ed25519 签名、最低 Vulkan 版本、`format` 和 `executable_path`；Windows zip 完整保留 `yserver.exe`、`xauth.exe`、`xkb/` 与 licenses。`pinned_for_navop` 匹配当前 Navop 版本，artifact 自身版本独立解析。签名原始字节是完整 zip，不是摘要文本或解压后的单个可执行文件。同一组件版本不得替换不同摘要，更新组件必须提升版本并同步固定清单。

正式 Windows full 和 standalone-shell x86_64 构建前，`script/prepare-windows-x11-component.py` 从固定清单下载 zip、限制大小、校验 SHA-256 与 Ed25519 签名，再通过 `NAVOP_X11_BUNDLED_ARCHIVE` 交给 `crates/x11_component/build.rs` 编进 `navop.exe`。build script 不联网。这样 MSI、Burn、portable 和只替换 exe 的自更新都携带同一组件，不依赖旁置 zip。Win32 没有对应 yserver 产品，standalone-data 不嵌入；二者均不得误装 x64 包。当前 Navop 没有 Windows aarch64 产品 target，但组件安装器保留正确架构选择供后续产品接入。

运行时安装和“重新下载/修复”优先读取内嵌 zip，仍完整执行灰度、摘要、签名、安全 zip 解压、staging 与原子提交，不创建 HTTP client。内嵌包损坏或目标不符时 fail-closed，不回退网络。开发构建没有内嵌包时可从同一固定 Release 下载；mock HTTP 测试必须保持无内嵌包环境。

安装进入 `<appdata>/components/x11/.tmp-<random>/`，校验后原子 rename 到 `<version>/`。验签公钥固定在仓库且独立于应用更新公钥，私钥只在 fork 仓库的 GitHub Actions secret。任何校验失败都删除临时目录并 fail closed。版本目录保留最近两个非活跃版本；current、本次安装版本和任何实例仍在使用的版本都受保护。重新安装不会杀掉活跃 server，提交前在组件锁内重新检查活跃版本。

下载、验签、解压、目录替换与清理在 blocking 线程执行；开发下载的清单和 artifact 请求有明确超时。组件根目录的跨进程文件锁覆盖安装提交和 server 启动临界区；`instances/*` 记录各进程持有的 server，避免多实例并发覆盖 pidfile、误替换版本或误删记录。

远程清单同时承载灰度比例，缺省灰度为 0%；显式 Embedded 和手动二进制入口用于开发验收，不被 Auto 灰度拦截。`NAVOP_X11_YSERVER_PATH` 仍经过同一套 Vulkan、endpoint、进程和握手检查，但不能把上游原版的全网卡监听当作安全内置。

## 进程与健康检查

组件全局只维护一个 server，同一 Navop 进程内的所有 SSH 会话共享 endpoint，但各自保留 x11-req 的 cookie 委托。cookie 由组件生成并写入 server xauth 文件，同时交给 `X11Proxy::for_endpoint`。本期不跨 Navop 进程附加到已有 server：检测到另一实例仍持有它时明确返回停用原因，而不是猜用对方的 cookie。

启动后必须完成一次真实 X11 setup 握手：发送 MIT-MAGIC-COOKIE-1 ClientHello，读取完整 setup reply，并限制应答大小和总超时。仅等待进程或端口可连接不算健康。

进程退出由 supervisor 记录并重启，窗口为 10 分钟最多 3 次；超限后本次运行禁用内置，成功、失败和永久禁用都会留下供界面消费的崩溃通知。pidfile 必须同时校验 pid、进程启动时间和可执行文件身份，不能仅按 pid 杀进程。启动在组件锁内同时检查共享 pidfile 和 `instances/*`：另一 Navop 实例仍持有的活跃 server 不能当作孤儿清理；owner 已退出且 child 身份一致的进程才按孤儿回收。首次启动遇到 endpoint 冲突会在锁内轮换随机 identity，崩溃重启保持原 endpoint/cookie。Windows 子进程统一经过 `process-util` 的后台进程配置。

启动、崩溃、重启、Auto 回退和内置失败使用单调计数，并输出 `x11_component.metrics` structured 日志，供 dogfood 计算崩溃率和回退率；本期不虚构尚未接入的远端遥测后端。

## 上游 endpoint 限制与安全改造

上游公开 [setup 文档](https://github.com/joske/yserver/blob/master/docs/setup.md) 的 `yserver :7 -auth <path> -listen tcp` 默认监听 `0.0.0.0`，不能限定 loopback。[Windows 发布说明](https://github.com/joske/yserver/blob/master/tools/windows/README.txt)要求 trusted (`-Y`)，并声明 Vulkan 能力要求。该默认 TCP 形态不满足本设计的安全要求。

Navop fork 增加 `--listen IP`，只绑定明确给出的单个地址；保留默认行为以兼容上游其他用户。正式内置命令固定为 `--listen 127.0.0.1`，并继续使用 cookie。`CommandSpec::installed_tcp` 和 `manual_upstream_tcp` 分离：已验签的 fork 走安全参数，手动上游 TCP 仍 fail-closed，避免旧版静默忽略新参数。不能把全网卡监听伪装成随机回环端口，也不能靠改写远端 DISPLAY 规避问题。

## 用户可见状态

设置页展示模式、已安装组件版本和“重新下载/修复”。会话信息显示以下结构化状态之一：外部 server 及其名称、内置 yserver 及版本、已停用及具体原因。

下载失败、验签失败、Vulkan 不足、握手超时、平台/灰度不可用、远端拒绝 x11-req 都必须有对应文案。只有收到真实 CHANNEL_FAILURE 才显示远端拒绝诊断；本地发送失败、连接关闭和响应超时分别归类，成功响应恢复该会话此前的可用状态。远端拒绝时提示检查 `X11Forwarding yes`、`X11DisplayOffset`、xauth，并明确本应用使用 trusted (`-Y`)；不能拿不支持 SECURITY 扩展的 `ssh -X` 行为作为诊断依据。

X11 回连等待响应期间保留其他通道事件，每个 SSH 连接最多接受 32 条并发 X11 回连；本机连接、认证头读取和 setup 握手合计有 10 秒超时，已建立的正常双向中继不受这个总时长限制。

## 验证

- 单元：manifest、artifact、版本、路径、签名正/负/截断/换公钥、cookie、命令安全闸门、崩溃窗口与灰度。
- 集成：mock 清单和下载服务，覆盖断网、篡改、重复下载、握手失败、重启；内嵌路径额外覆盖离线安装、目标不匹配、损坏包和强制修复。
- 构建：准备脚本校验真实固定 zip，CI 编译内嵌产物并运行内嵌字节与固定清单一致性测试；下载 mock 测试不注入内嵌包。
- 转发：setup 探针执行真实协议往返，并验证可变长度正文完整读取。
- E2E：Windows 上真实 yserver + xterm/gtk3-demo 截图比对；该验收不能用 mock 替代。
- 手动矩阵：有无 Vulkan、RDP、多显示器、HiDPI、与 VcXsrv 共存、Windows arm64。

本环境无法完成 Windows 真实窗口和 GPU 矩阵时，交付报告必须明确标记未验收，不能把单元测试描述成端到端通过。
