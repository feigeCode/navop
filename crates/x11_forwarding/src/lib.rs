//! SSH X11 转发支持库。
//!
//! 基于 X.Org 公开的 X11 协议规范实现，覆盖转发所需的四个环节：
//!
//! 1. 本机环境探测（[`detect_local_server`]）：DISPLAY（macOS 下回退
//!    `launchctl getenv DISPLAY`）与 Xauthority 中的 MIT-MAGIC-COOKIE-1；
//! 2. 按 SSH 会话签发 fake cookie（[`X11Proxy::issue_request`]），真实
//!    cookie 不离开本机；
//! 3. sshd 回连通道的 setup 报文校验：fake cookie 匹配后，把认证数据
//!    替换为本机真实 cookie（[`X11ProxyHandle::run_channel`]）；
//! 4. 通过校验后桥接到本机 X server，未通过校验的字节一律不转发。

mod bridge;
mod cookie;
mod detect;
mod display;
mod error;
mod proxy;
mod setup;
mod xauthority;

pub use cookie::{ForwardRequest, MagicCookie};
pub use detect::detect_local_server;
pub use display::{DisplayAddress, ServerEndpoint};
pub use error::{X11Error, X11Result};
pub use proxy::{X11Proxy, X11ProxyHandle};
