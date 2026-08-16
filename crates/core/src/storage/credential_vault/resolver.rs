use anyhow::{Result, bail};

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionType, CredentialRepository, DbConnectionConfig, MongoDBParams, ProxyConfig,
    RedisParams, ReferencedCredentialFields, RemoteDesktopParams, SshAccountExpect, SshAuthMethod,
    SshParams, StoredConnection, resolve_credential_reference_strict,
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
            return Ok(params);
        };
        reject_conflicting_ssh_fields(reference)?;
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
        }
        self.resolve_optional_proxy(params.proxy.as_mut())?;
        self.resolve_optional_jump(params.jump_server.as_mut())?;
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
        _ => None,
    }
}

fn private_key_from_auth(auth: &SshAuthMethod) -> Option<String> {
    match auth {
        SshAuthMethod::PrivateKey { key_path, .. } => Some(key_path.clone()),
        SshAuthMethod::PrivateKeyContent { private_key, .. } => Some(private_key.clone()),
        _ => None,
    }
}

fn passphrase_from_auth(auth: &SshAuthMethod) -> Option<String> {
    match auth {
        SshAuthMethod::PrivateKey { passphrase, .. }
        | SshAuthMethod::PrivateKeyContent { passphrase, .. } => passphrase.clone(),
        _ => None,
    }
}

fn apply_ssh_auth(
    auth: &mut SshAuthMethod,
    reference: &crate::storage::CredentialReference,
    fields: ReferencedCredentialFields,
    credential: Option<&crate::storage::CredentialEntry>,
) -> Result<()> {
    if reference.password {
        *auth = SshAuthMethod::Password {
            password: fields.password.unwrap_or_default(),
        };
    } else if reference.private_key {
        let credential =
            credential.ok_or_else(|| anyhow::anyhow!("private-key credential is missing"))?;
        let passphrase = fields.passphrase;
        *auth = if let Some(private_key) = credential
            .private_key_content
            .clone()
            .filter(|value| !value.is_empty())
        {
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase,
            }
        } else {
            SshAuthMethod::PrivateKey {
                key_path: fields.private_key.unwrap_or_default(),
                passphrase,
            }
        };
    } else if reference.passphrase {
        match auth {
            SshAuthMethod::PrivateKey { passphrase, .. }
            | SshAuthMethod::PrivateKeyContent { passphrase, .. } => {
                *passphrase = fields.passphrase;
            }
            _ => bail!("a passphrase reference requires private-key authentication"),
        }
    }
    Ok(())
}

fn reject_conflicting_ssh_fields(reference: &crate::storage::CredentialReference) -> Result<()> {
    if reference.password && reference.private_key {
        bail!("a credential reference cannot select password and private key together");
    }
    Ok(())
}
