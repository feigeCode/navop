/// 转发链路各环节统一使用的错误类型。
pub type X11Result<T> = Result<T, X11Error>;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum X11Error {
    #[error("找不到本机 DISPLAY（未安装或未启动 X server）")]
    DisplayNotFound,
    #[error("DISPLAY 格式无法解析：{0}")]
    DisplayMalformed(String),
    #[error("display 编号 {0} 无法映射到合法 TCP 端口")]
    DisplayPortOverflow(u16),
    #[error("读取 Xauthority 文件失败：{0}")]
    AuthorityUnreadable(String),
    #[error("Xauthority 记录被截断")]
    AuthorityTruncated,
    #[error("Xauthority 中没有适用于本 DISPLAY 的 MIT-MAGIC-COOKIE-1 记录")]
    AuthorityNoMatch,
    #[error("cookie 数据非法：{0}")]
    CookieMalformed(String),
    #[error("未知的 X11 认证协议名：{0}")]
    UnknownAuthName(String),
    #[error("setup 报文字节序标记非法：{0:#04x}")]
    BadByteOrderMark(u8),
    #[error("setup 报文长度不足")]
    SetupTruncated,
    #[error("setup 报文认证区超过 {0} 字节上限")]
    SetupOversized(usize),
    #[error("远端出示的 cookie 未通过校验")]
    CookieRejected,
}
