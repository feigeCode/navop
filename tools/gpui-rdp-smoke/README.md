# Minimal GPUI Windows Native RDP Smoke Client

`gpui-rdp-smoke` 是一个独立的 Windows 真机烟测程序。它只保留：

- 真实 `gpui` / `gpui_platform` 窗口与事件循环；
- 从 GPUI 窗口取得真实 Win32 `HWND`；
- `windows_rdp_host` 提供的 ATL ActiveX RDP 子窗口；
- 用户名、域、密码和最小连接参数；
- 原始事件、语义事件、同步错误、HRESULT、Win32 code、创建阶段、登录错误与断开原因日志。

它不依赖完整 Navop 主程序、`remote_desktop_view`、`gpui-component`、数据库、SSH、Tokio 或扩展系统。这个程序用于快速隔离“GPUI 原生窗口 + RDP ActiveX host + 凭据 + connect”链路，不代表完整 Navop 的全部远程桌面 UI 功能。

## Windows 构建环境

请在 **Developer PowerShell for VS 2022** 中运行，并确保 Visual Studio Installer 已安装：

- MSVC C++ build tools；
- C++ ATL/MFC；
- Windows SDK；
- Windows 系统 RDP ActiveX type library。

`windows_rdp_host` 的原生构建脚本要求 Windows + MSVC，并会使用 `VCToolsInstallDir` 下对应架构的 `atls.lib`。当前支持 x86_64 和 x86 MSVC target，不支持从 macOS 交叉构建该原生 host。

## 运行

密码只通过环境变量传入，不要把密码写入命令行：

```powershell
$env:NAVOP_RDP_PASSWORD = "secret"

cargo run `
  -p gpui-rdp-smoke `
  -- `
  --host 10.0.0.5 `
  --username Administrator
```

带域、自定义端口和桌面尺寸：

```powershell
$env:NAVOP_RDP_PASSWORD = "secret"

cargo run `
  -p gpui-rdp-smoke `
  -- `
  --host rdp.example.internal `
  --port 3389 `
  --username alice `
  --domain EXAMPLE `
  --width 1600 `
  --height 900 `
  --timeout-seconds 90
```

运行结束后清理密码环境变量：

```powershell
Remove-Item Env:NAVOP_RDP_PASSWORD
```

如果目标服务器允许无密码或系统以其他方式完成认证，可以不设置 `NAVOP_RDP_PASSWORD`；启动日志只输出环境变量是否存在，不输出密码内容。

## 如何判断结果

同步 `connect` 调用成功只表示 ActiveX 已接受连接请求，**不表示登录已经完成**。

只有控制台出现下面这一行，才能确认此次烟测至少完成了 RDP 登录：

```text
RESULT: LOGIN_COMPLETE
```

程序不会在登录成功后自动退出，以便直接观察和操作远端桌面。关闭 GPUI 窗口时，程序会在同一个 UI owner thread 上隐藏、断开并销毁原生 RDP host。

常见终止标记：

```text
RESULT: LOGON_ERROR
RESULT: FATAL_ERROR
RESULT: DISCONNECTED_BEFORE_LOGIN
RESULT: TIMEOUT
```

如果失败，请复制从以下内容开始、直到 `close:` 结束的**完整控制台输出**：

- `config:`
- `probe:`
- `create:`
- `bounds:`
- `credentials:`
- `connect:`
- 所有 `raw event:` / `event:` / `diagnostic:`
- 所有 `ERROR` / `ERROR_DEBUG` / `ERROR_FIELDS`
- `close:`

尤其不要只复制最后一行；`ERROR_FIELDS` 会包含用于定位原生创建失败的 `native_stage`、`win32_code`、`hresult_code` 和分类信息。
