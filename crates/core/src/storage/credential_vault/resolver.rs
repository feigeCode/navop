use anyhow::{Result, bail};

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionType, CredentialRepository, DbConnectionConfig, FtpParams, MongoDBParams,
    ProxyConfig, RedisParams, ReferencedCredentialFields, RemoteDesktopParams, RemoteFileParams,
    RemoteFileProtocol, SshAccountExpect, SshAgentForwardKey, SshAuthMethod, SshParams,
    StoredConnection, TelnetLoginStep, TelnetParams, resolve_credential_reference_strict,
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
        params.account_expect = SshAccountExpect::default();
        let Some(reference) = params.credential_reference.as_ref() else {
            self.resolve_optional_proxy(params.proxy.as_mut())?;
            self.resolve_optional_jump(params.jump_server.as_mut())?;
            self.resolve_remote_file_ftp(params.remote_file.as_mut())?;
            return Ok(params);
        };
        let credential = self.resolve_reference_entry(reference)?;
        let manual = ssh_fields(&params);
        let fields = resolve_credential_reference_strict(manual, reference, credential.as_ref())?;
        params.username = fields.username.clone().unwrap_or_default();
        apply_ssh_auth(
            &mut params.auth_method,
            reference,
            fields,
            credential.as_ref(),
        )?;
        if let Some(credential) = credential.as_ref() {
            params.account_expect = credential.ssh_expect.clone();
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
        let fields = resolve_credential_reference_strict(
            ReferencedCredentialFields::new(
                Some(jump.username.clone()),
                password_from_auth(&jump.auth_method),
                private_key_from_auth(&jump.auth_method),
                passphrase_from_auth(&jump.auth_method),
            ),
            reference,
            credential.as_ref(),
        )?;
        jump.username = fields.username.clone().unwrap_or_default();
        apply_ssh_auth(
            &mut jump.auth_method,
            reference,
            fields,
            credential.as_ref(),
        )
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

fn ssh_fields(params: &SshParams) -> ReferencedCredentialFields {
    ReferencedCredentialFields::new(
        Some(params.username.clone()),
        password_from_auth(&params.auth_method),
        private_key_from_auth(&params.auth_method),
        passphrase_from_auth(&params.auth_method),
    )
}

fn password_from_auth(auth: &SshAuthMethod) -> Option<String> {
    match auth {
        SshAuthMethod::Password { password } => Some(password.clone()),
        SshAuthMethod::Chain(steps) => steps.iter().find_map(password_from_auth),
        _ => None,
    }
}

fn private_key_from_auth(auth: &SshAuthMethod) -> Option<String> {
    match auth {
        SshAuthMethod::PrivateKey { key_path, .. } => Some(key_path.clone()),
        SshAuthMethod::PrivateKeyContent { private_key, .. } => Some(private_key.clone()),
        SshAuthMethod::Chain(steps) => steps.iter().find_map(private_key_from_auth),
        _ => None,
    }
}

fn passphrase_from_auth(auth: &SshAuthMethod) -> Option<String> {
    match auth {
        SshAuthMethod::PrivateKey { passphrase, .. }
        | SshAuthMethod::PrivateKeyContent { passphrase, .. } => passphrase.clone(),
        SshAuthMethod::Chain(steps) => steps.iter().find_map(passphrase_from_auth),
        _ => None,
    }
}

/// 用解析后的字段构造私钥认证方式；凭据里有内联私钥内容时优先用它。
fn build_private_key_auth(
    private_key: Option<String>,
    passphrase: Option<String>,
    credential: Option<&crate::storage::CredentialEntry>,
) -> Result<SshAuthMethod> {
    let credential =
        credential.ok_or_else(|| anyhow::anyhow!("private-key credential is missing"))?;
    if let Some(private_key_content) = credential
        .private_key_content
        .clone()
        .filter(|value| !value.is_empty())
    {
        Ok(SshAuthMethod::PrivateKeyContent {
            private_key: private_key_content,
            passphrase,
        })
    } else {
        Ok(SshAuthMethod::PrivateKey {
            key_path: private_key.unwrap_or_default(),
            passphrase,
        })
    }
}

fn apply_ssh_auth(
    auth: &mut SshAuthMethod,
    reference: &crate::storage::CredentialReference,
    fields: ReferencedCredentialFields,
    credential: Option<&crate::storage::CredentialEntry>,
) -> Result<()> {
    if reference.password && reference.private_key {
        // 服务器要求「密码 + 密钥」两个因素时，凭据引用需要同时提供两份凭据。
        let password = fields.password.unwrap_or_default();
        let private_key =
            build_private_key_auth(fields.private_key, fields.passphrase, credential)?;
        *auth = SshAuthMethod::Chain(vec![SshAuthMethod::Password { password }, private_key]);
    } else if reference.password {
        *auth = SshAuthMethod::Password {
            password: fields.password.unwrap_or_default(),
        };
    } else if reference.private_key {
        *auth = build_private_key_auth(fields.private_key, fields.passphrase, credential)?;
    } else if reference.passphrase {
        match auth {
            SshAuthMethod::PrivateKey { passphrase, .. }
            | SshAuthMethod::PrivateKeyContent { passphrase, .. } => {
                *passphrase = fields.passphrase;
            }
            SshAuthMethod::Chain(steps) => {
                let key_factor = steps
                    .iter_mut()
                    .find(|step| step.contains_private_key())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "a passphrase reference requires private-key authentication"
                        )
                    })?;
                match key_factor {
                    SshAuthMethod::PrivateKey { passphrase, .. }
                    | SshAuthMethod::PrivateKeyContent { passphrase, .. } => {
                        *passphrase = fields.passphrase;
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "a passphrase reference requires private-key authentication"
                        ));
                    }
                }
            }
            _ => bail!("a passphrase reference requires private-key authentication"),
        }
    }
    Ok(())
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
