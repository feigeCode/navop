use anyhow::{Result, bail};

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionType, CredentialRepository, CredentialResolutionError, DbConnectionConfig, FtpParams,
    MongoDBParams, ProxyConfig, RedisParams, ReferencedCredentialFields, RemoteDesktopParams,
    RemoteFileParams, RemoteFileProtocol, SshAccountExpect, SshAgentForwardKey, SshAuthMethod,
    SshParams, StoredConnection, TelnetLoginStep, TelnetParams,
    resolve_credential_reference_strict,
};

impl CredentialRepository {
    /// Resolves credential references into a temporary in-memory connection.
    ///
    /// The returned connection must never be persisted: it may contain
    /// plaintext secrets loaded from the credential vault.
    pub fn resolve_connection(&self, connection: &StoredConnection) -> Result<StoredConnection> {
        let mut resolved = connection.clone();
        resolved.params = match connection.connection_type {
            ConnectionType::SshSftp => {
                serde_json::to_string(&self.resolve_ssh(connection.to_ssh_params()?)?)?
            }
            ConnectionType::Database => {
                serde_json::to_string(&self.resolve_database(connection.to_db_connection()?)?)?
            }
            ConnectionType::Redis => {
                serde_json::to_string(&self.resolve_redis(connection.to_redis_params()?)?)?
            }
            ConnectionType::MongoDB => {
                serde_json::to_string(&self.resolve_mongodb(connection.to_mongodb_params()?)?)?
            }
            ConnectionType::Telnet => {
                serde_json::to_string(&self.resolve_telnet(connection.to_telnet_params()?)?)?
            }
            ConnectionType::Rdp | ConnectionType::Vnc => serde_json::to_string(
                &self.resolve_remote_desktop(connection.to_remote_desktop_params()?)?,
            )?,
            _ => connection.params.clone(),
        };
        Ok(resolved)
    }

    pub fn resolve_ssh(&self, mut params: SshParams) -> Result<SshParams> {
        // 提权（su/sudo）应答属于终端行为，不是登录期望：钥匙串只提供缺省的
        // 登录提示匹配，连接级的提权配置优先保留。
        let connection_elevation = params.account_expect.elevation.clone();
        params.account_expect = SshAccountExpect {
            elevation: connection_elevation.clone(),
            ..SshAccountExpect::default()
        };
        let Some(reference) = params.credential_reference.as_ref() else {
            self.resolve_optional_proxy(params.proxy.as_mut())?;
            self.resolve_optional_jump(params.jump_server.as_mut())?;
            self.resolve_remote_file_ftp(params.remote_file.as_mut())?;
            return Ok(params);
        };
        let credential = self.resolve_reference_entry(reference)?;
        // 选了钥匙串后表单里的用户名、认证方式、「连接时输入」都被隐藏，
        // 保存下来的值不再参与：登录信息完全由钥匙串记录决定。
        let login = ssh_login_from_credential(reference, credential.as_ref())?;
        params.username = login.username;
        params.auth_method = login.auth_method;
        params.prompt_username = None;
        params.prompt_password = None;
        if let Some(credential) = credential.as_ref() {
            params.account_expect = credential.ssh_expect.clone();
            if !connection_elevation.is_empty() {
                params.account_expect.elevation = connection_elevation;
            }
            // 仅当凭据带私钥且标记「转发到 ssh-agent」时开启 ForwardAgent，
            // 让远端（如跳板机）可用这把本地私钥继续向更内层主机认证。
            //
            // 这与本连接实际选用的认证方式无关：即使这里选的是密码认证
            // （典型场景：密码登录跳板机，再用被转发的 agent 里的这把
            // 私钥登录更内层主机），私钥内容也要通过 agent_forward_key
            // 回填，否则下游只能从 auth_method 猜身份，密码场景下会漏填。
            if credential.forward_to_agent && credential.private_key().is_some() {
                params.forward_agent = Some(true);
                params.agent_forward_key = Some(SshAgentForwardKey {
                    private_key_path: credential
                        .private_key_path
                        .clone()
                        .filter(|value| !value.is_empty()),
                    private_key_content: credential
                        .private_key_content
                        .clone()
                        .filter(|value| !value.is_empty()),
                    passphrase: credential.passphrase.clone(),
                });
            }
        }
        self.resolve_optional_proxy(params.proxy.as_mut())?;
        self.resolve_optional_jump(params.jump_server.as_mut())?;
        self.resolve_remote_file_ftp(params.remote_file.as_mut())?;
        Ok(params)
    }

    /// 解析 SSH 连接记录上远程文件配置中的 FTP 凭据引用。
    ///
    /// 仅当协议为 FTP 且携带 `FtpParams.credential_reference` 时生效；
    /// 运行时副本会写入明文用户名/密码，但调用方不得将其持久化。
    pub fn resolve_remote_file_ftp(
        &self,
        remote_file: Option<&mut RemoteFileParams>,
    ) -> Result<()> {
        let Some(remote_file) = remote_file else {
            return Ok(());
        };
        if remote_file.protocol != RemoteFileProtocol::Ftp {
            return Ok(());
        }
        let Some(ftp) = remote_file.ftp.as_mut() else {
            bail!("remote_file protocol is FTP but ftp params are missing");
        };
        *ftp = self.resolve_ftp(ftp.clone())?;
        Ok(())
    }

    pub fn resolve_telnet(&self, mut params: TelnetParams) -> Result<TelnetParams> {
        let Some(reference) = params.credential_reference.as_ref() else {
            return Ok(params);
        };
        let credential = self.resolve_reference_entry(reference)?;
        let fields = resolve_credential_reference_strict(
            ReferencedCredentialFields::new(None, None, None, None),
            reference,
            credential.as_ref(),
        )?;
        let username = fields.username.as_deref();
        let password = fields.password.as_deref();

        if params.login_script.is_empty()
            && let Some(credential) = credential.as_ref()
        {
            params.login_script = telnet_login_script_from_credential(credential);
        }
        params.apply_login_credentials(username, password);
        Ok(params)
    }

    pub fn resolve_ftp(&self, mut params: FtpParams) -> Result<FtpParams> {
        let Some(reference) = params.credential_reference.as_ref() else {
            return Ok(params);
        };
        let credential = self.resolve_reference_entry(reference)?;
        let fields = resolve_credential_reference_strict(
            ReferencedCredentialFields::new(
                Some(params.username.clone()),
                Some(params.password.clone()),
                None,
                None,
            ),
            reference,
            credential.as_ref(),
        )?;
        params.username = fields.username.unwrap_or_default();
        params.password = fields.password.unwrap_or_default();
        Ok(params)
    }

    pub fn resolve_database(&self, mut params: DbConnectionConfig) -> Result<DbConnectionConfig> {
        if let Some(reference) = params.credential_reference.as_ref() {
            let credential = self.resolve_reference_entry(reference)?;
            let fields = resolve_credential_reference_strict(
                ReferencedCredentialFields::new(
                    Some(params.username.clone()),
                    Some(params.password.clone()),
                    None,
                    None,
                ),
                reference,
                credential.as_ref(),
            )?;
            params.username = fields.username.unwrap_or_default();
            params.password = fields.password.unwrap_or_default();
        }
        self.resolve_optional_proxy(params.proxy.as_mut())?;
        Ok(params)
    }

    pub fn resolve_redis(&self, mut params: RedisParams) -> Result<RedisParams> {
        if let Some(reference) = params.credential_reference.as_ref() {
            let credential = self.resolve_reference_entry(reference)?;
            let fields = resolve_credential_reference_strict(
                ReferencedCredentialFields::new(
                    params.username.clone(),
                    params.password.clone(),
                    None,
                    None,
                ),
                reference,
                credential.as_ref(),
            )?;
            params.username = fields.username;
            params.password = fields.password;
        }
        if let Some(sentinel) = params.sentinel.as_mut()
            && let Some(reference) = sentinel.credential_reference.as_ref()
        {
            let credential = self.resolve_reference_entry(reference)?;
            let fields = resolve_credential_reference_strict(
                ReferencedCredentialFields::new(
                    None,
                    sentinel.sentinel_password.clone(),
                    None,
                    None,
                ),
                reference,
                credential.as_ref(),
            )?;
            sentinel.sentinel_password = fields.password;
        }
        Ok(params)
    }

    pub fn resolve_mongodb(&self, mut params: MongoDBParams) -> Result<MongoDBParams> {
        if let Some(reference) = params.credential_reference.as_ref() {
            let credential = self.resolve_reference_entry(reference)?;
            let fields = resolve_credential_reference_strict(
                ReferencedCredentialFields::new(
                    params.username.clone(),
                    params.password.clone(),
                    None,
                    None,
                ),
                reference,
                credential.as_ref(),
            )?;
            params.username = fields.username;
            params.password = fields.password;
        }
        Ok(params)
    }

    pub fn resolve_remote_desktop(
        &self,
        mut params: RemoteDesktopParams,
    ) -> Result<RemoteDesktopParams> {
        if let Some(reference) = params.credential_reference.as_ref() {
            let credential = self.resolve_reference_entry(reference)?;
            let fields = resolve_credential_reference_strict(
                ReferencedCredentialFields::new(
                    params.username.clone(),
                    params.password.clone(),
                    None,
                    None,
                ),
                reference,
                credential.as_ref(),
            )?;
            params.username = fields.username;
            params.password = fields.password;
        }
        self.resolve_optional_proxy(params.proxy.as_mut())?;
        Ok(params)
    }

    fn resolve_optional_proxy(&self, proxy: Option<&mut ProxyConfig>) -> Result<()> {
        let Some(proxy) = proxy else {
            return Ok(());
        };
        let Some(reference) = proxy.credential_reference.as_ref() else {
            return Ok(());
        };
        let credential = self.resolve_reference_entry(reference)?;
        let fields = resolve_credential_reference_strict(
            ReferencedCredentialFields::new(
                proxy.username.clone(),
                proxy.password.clone(),
                None,
                None,
            ),
            reference,
            credential.as_ref(),
        )?;
        proxy.username = fields.username;
        proxy.password = fields.password;
        Ok(())
    }

    fn resolve_optional_jump(
        &self,
        jump: Option<&mut crate::storage::JumpServerConfig>,
    ) -> Result<()> {
        let Some(jump) = jump else {
            return Ok(());
        };
        let Some(reference) = jump.credential_reference.as_ref() else {
            return Ok(());
        };
        let credential = self.resolve_reference_entry(reference)?;
        // 选了钥匙串后表单里的用户名与认证方式都被隐藏，保存值不再参与。
        let login = ssh_login_from_credential(reference, credential.as_ref())?;
        jump.username = login.username;
        jump.auth_method = login.auth_method;
        Ok(())
    }

    fn resolve_reference_entry(
        &self,
        reference: &crate::storage::CredentialReference,
    ) -> Result<Option<crate::storage::CredentialEntry>> {
        if let Some(cloud_id) = reference.credential_cloud_id.as_deref() {
            self.get_by_cloud_id(cloud_id)
        } else {
            self.get(reference.credential_id)
        }
    }
}

/// 钥匙串记录解析出的 SSH 登录信息。
struct SshCredentialLogin {
    username: String,
    auth_method: SshAuthMethod,
}

/// 完全按钥匙串记录的内容得出 SSH 用户名与认证方式。
///
/// 选中钥匙串后，表单会隐藏用户名与认证方式，用户无法再改它们，所以连接里
/// 残留的保存值不能参与；`credential_reference` 里的各个布尔标志同样由表单
/// 根据当时隐藏的状态算出，也不作依据。规则只有两条：
///
/// - 用户名取记录里的用户名，为空就报错，绝不带着空用户名去连接；
/// - 记录里有什么凭据就用什么：只有密码用密码，只有私钥用私钥，两者都有时
///   交给 `SshAuthMethod::Chain`，由 SSH 层按服务器通告的方法决定试哪个
///   （服务器只接受其中一种就只用那一种，两种都接受则按下面的顺序：先密码）。
fn ssh_login_from_credential(
    reference: &crate::storage::CredentialReference,
    credential: Option<&crate::storage::CredentialEntry>,
) -> Result<SshCredentialLogin> {
    let credential_id = reference.credential_id;
    let credential = credential.ok_or_else(|| {
        anyhow::anyhow!(CredentialResolutionError::MissingCredential(credential_id))
    })?;

    let username = credential
        .username
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(CredentialResolutionError::EmptyField {
                credential_id,
                credential_name: credential.name.clone(),
                field: "username",
            })
        })?
        .to_string();

    let non_empty = |value: &Option<String>| value.clone().filter(|value| !value.is_empty());
    let passphrase = non_empty(&credential.passphrase);
    let password =
        non_empty(&credential.password).map(|password| SshAuthMethod::Password { password });
    // 内联私钥内容优先于文件路径。
    let private_key = non_empty(&credential.private_key_content)
        .map(|private_key| SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase: passphrase.clone(),
        })
        .or_else(|| {
            non_empty(&credential.private_key_path).map(|key_path| SshAuthMethod::PrivateKey {
                key_path,
                passphrase: passphrase.clone(),
            })
        });

    let auth_method = match (password, private_key) {
        (Some(password), Some(private_key)) => SshAuthMethod::Chain(vec![password, private_key]),
        (Some(single), None) | (None, Some(single)) => single,
        (None, None) => {
            return Err(anyhow::anyhow!(CredentialResolutionError::EmptyField {
                credential_id,
                credential_name: credential.name.clone(),
                field: "password or private key",
            }));
        }
    };

    Ok(SshCredentialLogin {
        username,
        auth_method,
    })
}

fn telnet_login_script_from_credential(
    credential: &crate::storage::CredentialEntry,
) -> Vec<TelnetLoginStep> {
    [
        &credential.ssh_expect.username,
        &credential.ssh_expect.password,
    ]
    .into_iter()
    .filter(|step| !step.expect.trim().is_empty())
    .map(|step| TelnetLoginStep {
        expect: step.expect.clone(),
        send: step.send.clone(),
    })
    .collect()
}

#[cfg(test)]
mod ssh_login_tests {
    use crate::storage::{
        CredentialEntry, CredentialReference, CredentialResolutionError, SshAuthMethod,
    };

    use super::ssh_login_from_credential;

    fn reference(flags_password: bool, flags_key: bool) -> CredentialReference {
        // 标志位故意随意设置：解析结果必须与它们无关。
        CredentialReference {
            credential_id: 7,
            credential_cloud_id: None,
            username: false,
            password: flags_password,
            private_key: flags_key,
            passphrase: false,
        }
    }

    fn entry(
        username: Option<&str>,
        password: Option<&str>,
        key_content: Option<&str>,
        key_path: Option<&str>,
    ) -> CredentialEntry {
        let mut entry = CredentialEntry::new("entry");
        entry.username = username.map(str::to_string);
        entry.password = password.map(str::to_string);
        entry.private_key_content = key_content.map(str::to_string);
        entry.private_key_path = key_path.map(str::to_string);
        entry.passphrase = Some("phrase".to_string());
        entry
    }

    fn resolve(
        reference: &CredentialReference,
        entry: &CredentialEntry,
    ) -> anyhow::Result<(String, SshAuthMethod)> {
        ssh_login_from_credential(reference, Some(entry))
            .map(|login| (login.username, login.auth_method))
    }

    #[test]
    fn username_comes_from_the_credential_regardless_of_flags() {
        for flags in [(false, false), (true, false), (false, true), (true, true)] {
            let (username, _) = resolve(
                &reference(flags.0, flags.1),
                &entry(Some("  vault-user "), Some("pw"), None, None),
            )
            .expect("resolve");
            assert_eq!("vault-user", username);
        }
    }

    #[test]
    fn missing_or_blank_username_is_an_error_not_an_empty_login() {
        for username in [None, Some(""), Some("   ")] {
            let error = resolve(
                &reference(false, false),
                &entry(username, Some("pw"), None, None),
            )
            .expect_err("an empty username must be rejected");
            assert!(matches!(
                error.downcast_ref::<CredentialResolutionError>(),
                Some(CredentialResolutionError::EmptyField {
                    field: "username",
                    credential_id: 7,
                    credential_name,
                }) if credential_name == "entry"
            ));
        }
    }

    #[test]
    fn password_only_uses_password_auth() {
        let (_, auth) = resolve(
            &reference(false, false),
            &entry(Some("u"), Some("vault-password"), None, None),
        )
        .expect("resolve");
        assert!(
            matches!(auth, SshAuthMethod::Password { ref password } if password == "vault-password")
        );
    }

    #[test]
    fn key_only_prefers_inline_content_and_carries_the_passphrase() {
        let (_, auth) = resolve(
            &reference(false, false),
            &entry(Some("u"), None, Some("inline-key"), Some("/path/key")),
        )
        .expect("resolve");
        assert!(matches!(
            auth,
            SshAuthMethod::PrivateKeyContent { ref private_key, ref passphrase }
                if private_key == "inline-key" && passphrase.as_deref() == Some("phrase")
        ));

        let (_, auth) = resolve(
            &reference(false, false),
            &entry(Some("u"), None, None, Some("/path/key")),
        )
        .expect("resolve");
        assert!(matches!(
            auth,
            SshAuthMethod::PrivateKey { ref key_path, ref passphrase }
                if key_path == "/path/key" && passphrase.as_deref() == Some("phrase")
        ));
    }

    #[test]
    fn password_and_key_become_a_chain_with_the_password_first() {
        for flags in [(false, false), (true, false), (false, true), (true, true)] {
            let (_, auth) = resolve(
                &reference(flags.0, flags.1),
                &entry(Some("u"), Some("vault-password"), Some("inline-key"), None),
            )
            .expect("resolve");
            match auth {
                SshAuthMethod::Chain(steps) => assert!(matches!(
                    steps.as_slice(),
                    [
                        SshAuthMethod::Password { password },
                        SshAuthMethod::PrivateKeyContent { private_key, .. }
                    ] if password == "vault-password" && private_key == "inline-key"
                )),
                other => panic!("期望认证链，实际：{other:?}"),
            }
        }
    }

    #[test]
    fn a_credential_without_any_secret_is_an_error() {
        let error = resolve(&reference(true, true), &entry(Some("u"), None, None, None))
            .expect_err("nothing to authenticate with");
        assert!(matches!(
            error.downcast_ref::<CredentialResolutionError>(),
            Some(CredentialResolutionError::EmptyField {
                field: "password or private key",
                credential_id: 7,
                credential_name,
            }) if credential_name == "entry"
        ));
    }

    #[test]
    fn user_facing_messages_name_the_keychain_entry_and_are_translated() {
        let error = resolve(
            &reference(false, false),
            &entry(Some(""), Some("pw"), None, None),
        )
        .expect_err("empty username");
        let message = error.to_string();
        assert!(
            message.contains("entry"),
            "应显示钥匙串记录名称而不是数字 ID，实际：{message}"
        );
        assert!(
            !message.contains("CredentialError."),
            "翻译键没有被解析，实际：{message}"
        );
        assert!(!message.contains('7'), "不应暴露内部 ID，实际：{message}");
    }

    #[test]
    fn a_missing_credential_keeps_the_dedicated_error_for_the_runtime_fallback() {
        let error = ssh_login_from_credential(&reference(false, false), None)
            .err()
            .expect("missing credential");
        assert!(matches!(
            error.downcast_ref::<CredentialResolutionError>(),
            Some(CredentialResolutionError::MissingCredential(7))
        ));
    }
}
