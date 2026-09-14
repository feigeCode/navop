use zeroize::Zeroize;

const TERMSRV_PREFIX: &str = "TERMSRV/";
#[cfg(target_os = "windows")]
const HANDOFF_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(target_os = "windows")]
const NAVOP_CREDENTIAL_MARKER: &str = "Navop temporary MSTSC credential";

pub(super) struct MstscCredentials {
    pub(super) target: String,
    pub(super) username: String,
    pub(super) password: String,
}

pub(super) struct MstscCredentialInput<'a> {
    pub(super) host: &'a str,
    pub(super) username: Option<&'a str>,
    pub(super) password: Option<&'a str>,
    pub(super) domain: Option<&'a str>,
}

impl Drop for MstscCredentials {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

/// 构造独立 MSTSC 进程要用的临时凭据。
///
/// **target 只能是 `TERMSRV/<主机>`，不能带端口。** MSTSC 在 NLA 阶段查询
/// Windows 凭据管理器时只按主机名查，`/v:<主机>:<端口>` 里的端口不参与 target：
/// 写成 `TERMSRV/<主机>:<端口>` 时无论端口是默认 3389 还是自定义端口都查不到，
/// MSTSC 于是弹出「输入你的凭据」并要求手输密码。
///
/// 该结论由本机 loopback + 假 RDP 服务端（只完成 X.224 协商并声明
/// `PROTOCOL_HYBRID`）的对照实验验证：端口 3389 与 13389 下，
/// `TERMSRV/host:port` 都会触发凭据输入框，`TERMSRV/host` 则直接进入 CredSSP。
/// 副作用是同一主机不同端口无法保存不同凭据，这是 MSTSC 自身的限制。
pub(super) fn mstsc_credentials(input: MstscCredentialInput<'_>) -> Option<MstscCredentials> {
    let username = input.username.filter(|value| !value.is_empty())?;
    let password = input.password.filter(|value| !value.is_empty())?;
    let username = if username.contains(|character| matches!(character, '\\' | '@')) {
        username.to_string()
    } else if let Some(domain) = input.domain.filter(|value| !value.is_empty()) {
        format!("{domain}\\{username}")
    } else {
        username.to_string()
    };
    Some(MstscCredentials {
        target: format!("{TERMSRV_PREFIX}{}", input.host),
        username,
        password: password.to_string(),
    })
}

#[cfg(target_os = "windows")]
#[path = "mstsc_credentials_windows.rs"]
mod windows_credential;

#[cfg(target_os = "windows")]
pub(super) use windows_credential::store_temporary;
