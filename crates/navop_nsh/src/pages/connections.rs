use std::{
    cell::RefCell,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use nsh_base::{input, switch, theme};
use nsh_core::prelude::*;
use one_core::storage::traits::Repository as _;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, DbConnectionConfig, FtpParams,
    JumpServerConfig, MongoDBParams, MongoDriverVariant, MqttParams, PortForwardingKind,
    PortForwardingParams, ProxyConfig, ProxyType, RedisParams, RemoteDesktopParams,
    RemoteDesktopProtocol, SerialFlowControl, SerialParams, SerialParity, SftpAccount,
    SshAuthMethod, SshParams, StoredConnection, TelnetLoginStep, TelnetParams,
};
use otp::OtpSecret;

use crate::{services::NavopNshServices, ui};

#[derive(Clone)]
struct DatabaseDraft {
    name: String,
    database_type: DatabaseType,
    host: String,
    port: String,
    username: String,
    password: String,
    database: String,
    workspace_id: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SshAuthMode {
    Password,
    PrivateKey,
    Agent,
}

#[derive(Clone)]
struct SshDraft {
    name: String,
    host: String,
    port: String,
    username: String,
    auth_mode: SshAuthMode,
    password: String,
    private_key_path: String,
    passphrase: String,
    sftp_account_enabled: bool,
    sftp_username: String,
    sftp_password: String,
    sftp_default_directory: String,
    keyboard_interactive: bool,
    otp_secret: String,
    default_directory: String,
    init_script: String,
    jump_enabled: bool,
    jump_host: String,
    jump_port: String,
    jump_username: String,
    jump_password: String,
    proxy_enabled: bool,
    proxy_type: ProxyType,
    proxy_host: String,
    proxy_port: String,
    proxy_username: String,
    proxy_password: String,
    forward_agent: bool,
    x11_forwarding: bool,
    base: Option<SshParams>,
    workspace_id: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ServiceKind {
    Redis,
    Mongo,
    Ftp,
    Telnet,
    Mqtt,
    Serial,
    PortForwarding,
    Rdp,
    Vnc,
}

#[derive(Clone)]
struct ServiceDraft {
    kind: ServiceKind,
    name: String,
    host: String,
    port: String,
    username: String,
    password: String,
    database: String,
    domain: String,
    use_tls: bool,
    clean_session: bool,
    telnet_login_script: Vec<TelnetLoginStep>,
    remote_desktop_base: Option<RemoteDesktopParams>,
    serial_data_bits: String,
    serial_stop_bits: String,
    serial_parity: SerialParity,
    serial_flow_control: SerialFlowControl,
    forward_ssh_id: Option<i64>,
    forward_kind: PortForwardingKind,
    workspace_id: Option<i64>,
}

#[derive(Clone, PartialEq)]
struct ConnectionRow {
    id: Option<i64>,
    name: String,
    connection_type: ConnectionType,
    kind: String,
    detail: String,
    workspace: String,
}

pub(crate) fn connections() -> impl IntoElement {
    let repository = NavopNshServices::current().map(|services| services.connections.clone());
    let workspace_choices = NavopNshServices::current()
        .and_then(|services| services.workspaces.clone())
        .and_then(|repository| repository.list().ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|workspace| Some((workspace.id?, workspace.name)))
        .collect::<Vec<_>>();
    let ssh_choices = NavopNshServices::current()
        .map(|services| services.resolved_connections.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
        .filter_map(|connection| Some((connection.id?, connection.name)))
        .collect::<Vec<_>>();
    let items = signal(load_connections(repository.as_ref()));
    let status = signal(String::new());
    let show_form = signal(false);
    let show_ssh_form = signal(false);
    let show_service_form = signal(false);
    let editing_database_id = signal(None::<i64>);
    let editing_ssh_id = signal(None::<i64>);
    let editing_service_id = signal(None::<i64>);
    let name = signal(String::new());
    let database_type = signal(DatabaseType::PostgreSQL);
    let host = signal(String::from("localhost"));
    let port = signal(String::from("5432"));
    let username = signal(String::new());
    let password = signal(String::new());
    let database = signal(String::new());
    let selected_workspace = signal(None::<i64>);
    let ssh_name = signal(String::new());
    let ssh_host = signal(String::new());
    let ssh_port = signal(String::from("22"));
    let ssh_username = signal(String::new());
    let ssh_auth_mode = signal(SshAuthMode::Password);
    let ssh_password = signal(String::new());
    let ssh_private_key_path = signal(String::new());
    let ssh_passphrase = signal(String::new());
    let ssh_sftp_account_enabled = signal(false);
    let ssh_sftp_username = signal(String::new());
    let ssh_sftp_password = signal(String::new());
    let ssh_sftp_default_directory = signal(String::new());
    let ssh_keyboard_interactive = signal(true);
    let ssh_otp_secret = signal(String::new());
    let ssh_default_directory = signal(String::new());
    let ssh_init_script = signal(String::new());
    let ssh_jump_enabled = signal(false);
    let ssh_jump_host = signal(String::new());
    let ssh_jump_port = signal(String::from("22"));
    let ssh_jump_username = signal(String::new());
    let ssh_jump_password = signal(String::new());
    let ssh_proxy_enabled = signal(false);
    let ssh_proxy_type = signal(ProxyType::Socks5);
    let ssh_proxy_host = signal(String::new());
    let ssh_proxy_port = signal(String::from("1080"));
    let ssh_proxy_username = signal(String::new());
    let ssh_proxy_password = signal(String::new());
    let ssh_forward_agent = signal(false);
    let ssh_x11_forwarding = signal(false);
    let ssh_advanced_base = Rc::new(RefCell::new(None::<SshParams>));
    let service_kind = signal(ServiceKind::Redis);
    let service_name = signal(String::new());
    let service_host = signal(String::new());
    let service_port = signal(String::from("6379"));
    let service_username = signal(String::new());
    let service_password = signal(String::new());
    let service_database = signal(String::from("0"));
    let service_domain = signal(String::new());
    let service_use_tls = signal(false);
    let service_clean_session = signal(true);
    let service_telnet_login_script = signal(Vec::<TelnetLoginStep>::new());
    let service_serial_data_bits = signal(String::from("8"));
    let service_serial_stop_bits = signal(String::from("1"));
    let service_serial_parity = signal(SerialParity::None);
    let service_serial_flow_control = signal(SerialFlowControl::None);
    let service_forward_ssh_id = signal(None::<i64>);
    let service_forward_kind = signal(PortForwardingKind::Local);
    let remote_desktop_base = Rc::new(RefCell::new(None::<RemoteDesktopParams>));

    let create = {
        let repository = repository.clone();
        move || {
            let Some(repository) = repository.as_ref() else {
                status.set("Connection storage is unavailable".to_owned());
                return;
            };
            let draft = DatabaseDraft {
                name: name.get_untracked(),
                database_type: database_type.get_untracked(),
                host: host.get_untracked(),
                port: port.get_untracked(),
                username: username.get_untracked(),
                password: password.get_untracked(),
                database: database.get_untracked(),
                workspace_id: selected_workspace.get_untracked(),
            };
            match build_database_connection(draft) {
                Ok(connection) => {
                    let result = persist_connection(
                        repository,
                        connection,
                        editing_database_id.get_untracked(),
                    );
                    match result {
                        Ok(id) => {
                            items.set(load_connections(Some(repository)));
                            status.set(format!(
                                "{} connection #{id}",
                                if editing_database_id.get_untracked().is_some() {
                                    "Updated"
                                } else {
                                    "Created"
                                }
                            ));
                            show_form.set(false);
                            editing_database_id.set(None);
                            name.set(String::new());
                            host.set(String::from("localhost"));
                            port.set(String::from("5432"));
                            username.set(String::new());
                            password.set(String::new());
                            database.set(String::new());
                        }
                        Err(error) => status.set(format!("Failed to create connection: {error}")),
                    }
                }
                Err(error) => status.set(error),
            }
        }
    };
    let reload = {
        let repository = repository.clone();
        move || {
            items.set(load_connections(repository.as_ref()));
            status.set(String::new());
        }
    };
    let create_ssh = {
        let repository = repository.clone();
        let ssh_advanced_base = ssh_advanced_base.clone();
        move || {
            let Some(repository) = repository.as_ref() else {
                status.set("Connection storage is unavailable".to_owned());
                return;
            };
            let draft = SshDraft {
                name: ssh_name.get_untracked(),
                host: ssh_host.get_untracked(),
                port: ssh_port.get_untracked(),
                username: ssh_username.get_untracked(),
                auth_mode: ssh_auth_mode.get_untracked(),
                password: ssh_password.get_untracked(),
                private_key_path: ssh_private_key_path.get_untracked(),
                passphrase: ssh_passphrase.get_untracked(),
                sftp_account_enabled: ssh_sftp_account_enabled.get_untracked(),
                sftp_username: ssh_sftp_username.get_untracked(),
                sftp_password: ssh_sftp_password.get_untracked(),
                sftp_default_directory: ssh_sftp_default_directory.get_untracked(),
                keyboard_interactive: ssh_keyboard_interactive.get_untracked(),
                otp_secret: ssh_otp_secret.get_untracked(),
                default_directory: ssh_default_directory.get_untracked(),
                init_script: ssh_init_script.get_untracked(),
                jump_enabled: ssh_jump_enabled.get_untracked(),
                jump_host: ssh_jump_host.get_untracked(),
                jump_port: ssh_jump_port.get_untracked(),
                jump_username: ssh_jump_username.get_untracked(),
                jump_password: ssh_jump_password.get_untracked(),
                proxy_enabled: ssh_proxy_enabled.get_untracked(),
                proxy_type: ssh_proxy_type.get_untracked(),
                proxy_host: ssh_proxy_host.get_untracked(),
                proxy_port: ssh_proxy_port.get_untracked(),
                proxy_username: ssh_proxy_username.get_untracked(),
                proxy_password: ssh_proxy_password.get_untracked(),
                forward_agent: ssh_forward_agent.get_untracked(),
                x11_forwarding: ssh_x11_forwarding.get_untracked(),
                base: ssh_advanced_base.borrow().clone(),
                workspace_id: selected_workspace.get_untracked(),
            };
            match build_ssh_connection(draft) {
                Ok(connection) => {
                    let result =
                        persist_connection(repository, connection, editing_ssh_id.get_untracked());
                    match result {
                        Ok(id) => {
                            items.set(load_connections(Some(repository)));
                            status.set(format!(
                                "{} SSH connection #{id}",
                                if editing_ssh_id.get_untracked().is_some() {
                                    "Updated"
                                } else {
                                    "Created"
                                }
                            ));
                            show_ssh_form.set(false);
                            editing_ssh_id.set(None);
                            ssh_name.set(String::new());
                            ssh_host.set(String::new());
                            ssh_port.set(String::from("22"));
                            ssh_username.set(String::new());
                            ssh_password.set(String::new());
                            ssh_private_key_path.set(String::new());
                            ssh_passphrase.set(String::new());
                        }
                        Err(error) => status.set(format!("Failed to create connection: {error}")),
                    }
                }
                Err(error) => status.set(error),
            }
        }
    };
    let edit_connection = {
        let repository = repository.clone();
        let remote_desktop_base = remote_desktop_base.clone();
        let ssh_advanced_base = ssh_advanced_base.clone();
        move |id: i64| {
            let Some(repository) = repository.as_ref() else {
                status.set("Connection storage is unavailable".to_owned());
                return;
            };
            let Some(connection) = repository.get(id).ok().flatten() else {
                status.set(format!("Connection #{id} was not found"));
                return;
            };
            match connection.connection_type {
                ConnectionType::Database => {
                    let Ok(config) = connection.to_db_connection() else {
                        status.set("Database connection parameters are invalid".to_owned());
                        return;
                    };
                    editing_database_id.set(Some(id));
                    name.set(connection.name);
                    database_type.set(config.database_type);
                    host.set(config.host);
                    port.set(config.port.to_string());
                    username.set(config.username);
                    password.set(config.password);
                    database.set(config.database.unwrap_or_default());
                    selected_workspace.set(connection.workspace_id);
                    show_form.set(true);
                    show_ssh_form.set(false);
                }
                ConnectionType::SshSftp => {
                    let Ok(params) = connection.to_ssh_params() else {
                        status.set("SSH connection parameters are invalid".to_owned());
                        return;
                    };
                    *ssh_advanced_base.borrow_mut() = Some(params.clone());
                    let (mode, password, key_path, passphrase) = match params.auth_method.clone() {
                        SshAuthMethod::Password { password } => (
                            SshAuthMode::Password,
                            password,
                            String::new(),
                            String::new(),
                        ),
                        SshAuthMethod::PrivateKey {
                            key_path,
                            passphrase,
                        } => (
                            SshAuthMode::PrivateKey,
                            String::new(),
                            key_path,
                            passphrase.unwrap_or_default(),
                        ),
                        SshAuthMethod::Agent => (
                            SshAuthMode::Agent,
                            String::new(),
                            String::new(),
                            String::new(),
                        ),
                        _ => {
                            status.set(
                                "This SSH authentication mode is not editable in the nsh form yet"
                                    .to_owned(),
                            );
                            return;
                        }
                    };
                    let jump = params
                        .jump_server
                        .as_ref()
                        .or(params.disabled_jump_server.as_ref());
                    ssh_jump_enabled.set(params.jump_server.is_some());
                    ssh_jump_host.set(jump.map(|jump| jump.host.clone()).unwrap_or_default());
                    ssh_jump_port.set(
                        jump.map(|jump| jump.port.to_string())
                            .unwrap_or_else(|| "22".to_owned()),
                    );
                    ssh_jump_username
                        .set(jump.map(|jump| jump.username.clone()).unwrap_or_default());
                    ssh_jump_password.set(match jump.map(|jump| &jump.auth_method) {
                        Some(SshAuthMethod::Password { password }) => password.clone(),
                        _ => String::new(),
                    });
                    ssh_proxy_enabled.set(params.proxy.is_some());
                    ssh_proxy_type.set(
                        params
                            .proxy
                            .as_ref()
                            .map(|proxy| proxy.proxy_type)
                            .unwrap_or(ProxyType::Socks5),
                    );
                    ssh_proxy_host.set(
                        params
                            .proxy
                            .as_ref()
                            .map(|proxy| proxy.host.clone())
                            .unwrap_or_default(),
                    );
                    ssh_proxy_port.set(
                        params
                            .proxy
                            .as_ref()
                            .map(|proxy| proxy.port.to_string())
                            .unwrap_or_else(|| "1080".to_owned()),
                    );
                    ssh_proxy_username.set(
                        params
                            .proxy
                            .as_ref()
                            .and_then(|proxy| proxy.username.clone())
                            .unwrap_or_default(),
                    );
                    ssh_proxy_password.set(
                        params
                            .proxy
                            .as_ref()
                            .and_then(|proxy| proxy.password.clone())
                            .unwrap_or_default(),
                    );
                    ssh_sftp_account_enabled.set(params.sftp_account.is_some());
                    ssh_sftp_username.set(
                        params
                            .sftp_account
                            .as_ref()
                            .map(|account| account.username.clone())
                            .unwrap_or_default(),
                    );
                    ssh_sftp_password.set(
                        params
                            .sftp_account
                            .as_ref()
                            .map(|account| account.password.clone())
                            .unwrap_or_default(),
                    );
                    ssh_sftp_default_directory
                        .set(params.sftp_default_directory.clone().unwrap_or_default());
                    ssh_keyboard_interactive.set(params.keyboard_interactive_enabled());
                    ssh_otp_secret.set(
                        params
                            .otp
                            .as_ref()
                            .map(|otp| otp.secret.clone())
                            .unwrap_or_default(),
                    );
                    ssh_default_directory.set(params.default_directory.clone().unwrap_or_default());
                    ssh_init_script.set(params.init_script.clone().unwrap_or_default());
                    ssh_forward_agent.set(params.forward_agent.unwrap_or(false));
                    ssh_x11_forwarding.set(params.x11_forwarding.unwrap_or(false));
                    editing_ssh_id.set(Some(id));
                    ssh_name.set(connection.name);
                    ssh_host.set(params.host);
                    ssh_port.set(params.port.to_string());
                    ssh_username.set(params.username);
                    ssh_auth_mode.set(mode);
                    ssh_password.set(password);
                    ssh_private_key_path.set(key_path);
                    ssh_passphrase.set(passphrase);
                    selected_workspace.set(connection.workspace_id);
                    show_ssh_form.set(true);
                    show_form.set(false);
                }
                ConnectionType::Redis => {
                    remote_desktop_base.borrow_mut().take();
                    let Ok(params) = connection.to_redis_params() else {
                        status.set("Redis connection parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(ServiceKind::Redis);
                    service_name.set(connection.name);
                    service_host.set(params.host);
                    service_port.set(params.port.to_string());
                    service_username.set(params.username.unwrap_or_default());
                    service_password.set(params.password.unwrap_or_default());
                    service_database.set(params.db_index.to_string());
                    service_use_tls.set(params.use_tls);
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                ConnectionType::MongoDB => {
                    remote_desktop_base.borrow_mut().take();
                    let Ok(params) = connection.to_mongodb_params() else {
                        status.set("MongoDB connection parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(ServiceKind::Mongo);
                    service_name.set(connection.name);
                    service_host.set(params.host);
                    service_port.set(params.port.unwrap_or(27017).to_string());
                    service_username.set(params.username.unwrap_or_default());
                    service_password.set(params.password.unwrap_or_default());
                    service_database.set(params.database.unwrap_or_default());
                    service_use_tls.set(params.use_tls);
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                ConnectionType::Ftp => {
                    remote_desktop_base.borrow_mut().take();
                    let Ok(params) = connection.to_ftp_params() else {
                        status.set("FTP connection parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(ServiceKind::Ftp);
                    service_name.set(connection.name);
                    service_host.set(params.host);
                    service_port.set(params.port.to_string());
                    service_username.set(params.username);
                    service_password.set(params.password);
                    service_database.set(String::new());
                    service_use_tls.set(params.use_tls);
                    service_telnet_login_script.set(Vec::new());
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                ConnectionType::Telnet => {
                    remote_desktop_base.borrow_mut().take();
                    let Ok(params) = connection.to_telnet_params() else {
                        status.set("Telnet connection parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(ServiceKind::Telnet);
                    service_name.set(connection.name);
                    service_host.set(params.host);
                    service_port.set(params.port.to_string());
                    service_username.set(String::new());
                    service_password.set(String::new());
                    service_database.set(String::new());
                    service_use_tls.set(false);
                    service_telnet_login_script.set(params.login_script);
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                ConnectionType::Mqtt => {
                    remote_desktop_base.borrow_mut().take();
                    let Ok(params) = connection.to_mqtt_params() else {
                        status.set("MQTT connection parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(ServiceKind::Mqtt);
                    service_name.set(connection.name);
                    service_host.set(params.host);
                    service_port.set(params.port.to_string());
                    service_username.set(params.username.unwrap_or_default());
                    service_password.set(params.password.unwrap_or_default());
                    service_database.set(params.client_id);
                    service_domain.set(String::new());
                    service_use_tls.set(params.use_tls);
                    service_clean_session.set(params.clean_session);
                    service_telnet_login_script.set(Vec::new());
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                ConnectionType::Serial => {
                    remote_desktop_base.borrow_mut().take();
                    let Ok(params) = connection.to_serial_params() else {
                        status.set("Serial connection parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(ServiceKind::Serial);
                    service_name.set(connection.name);
                    service_host.set(params.port_name);
                    service_port.set(params.baud_rate.to_string());
                    service_username.set(String::new());
                    service_password.set(String::new());
                    service_database.set(String::new());
                    service_domain.set(String::new());
                    service_use_tls.set(false);
                    service_telnet_login_script.set(Vec::new());
                    service_serial_data_bits.set(params.data_bits.to_string());
                    service_serial_stop_bits.set(params.stop_bits.to_string());
                    service_serial_parity.set(params.parity);
                    service_serial_flow_control.set(params.flow_control);
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                ConnectionType::PortForwarding => {
                    remote_desktop_base.borrow_mut().take();
                    let Ok(params) = connection.to_port_forwarding_params() else {
                        status.set("Port forwarding parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(ServiceKind::PortForwarding);
                    service_name.set(connection.name);
                    service_host.set(params.bind_host);
                    service_port.set(params.bind_port.to_string());
                    service_domain.set(params.target_host);
                    service_database.set(params.target_port.to_string());
                    service_username.set(String::new());
                    service_password.set(String::new());
                    service_use_tls.set(false);
                    service_forward_ssh_id.set(Some(params.ssh_connection_id));
                    service_forward_kind.set(params.kind);
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                ConnectionType::Rdp | ConnectionType::Vnc => {
                    let Ok(params) = connection.to_remote_desktop_params() else {
                        status.set("Remote desktop parameters are invalid".to_owned());
                        return;
                    };
                    editing_service_id.set(Some(id));
                    service_kind.set(if params.protocol == RemoteDesktopProtocol::Rdp {
                        ServiceKind::Rdp
                    } else {
                        ServiceKind::Vnc
                    });
                    service_name.set(connection.name);
                    service_host.set(params.host.clone());
                    service_port.set(params.port.to_string());
                    service_username.set(params.username.clone().unwrap_or_default());
                    service_password.set(params.password.clone().unwrap_or_default());
                    service_domain.set(params.domain.clone().unwrap_or_default());
                    service_database.set(String::new());
                    service_use_tls.set(false);
                    service_telnet_login_script.set(Vec::new());
                    *remote_desktop_base.borrow_mut() = Some(params);
                    selected_workspace.set(connection.workspace_id);
                    show_service_form.set(true);
                    show_form.set(false);
                    show_ssh_form.set(false);
                }
                _ => status.set(format!(
                    "Editing {} connections is not available in the nsh form yet",
                    connection.connection_type.label()
                )),
            }
        }
    };
    let create_service = {
        let repository = repository.clone();
        let remote_desktop_base = remote_desktop_base.clone();
        move || {
            let Some(repository) = repository.as_ref() else {
                status.set("Connection storage is unavailable".to_owned());
                return;
            };
            let draft = ServiceDraft {
                kind: service_kind.get_untracked(),
                name: service_name.get_untracked(),
                host: service_host.get_untracked(),
                port: service_port.get_untracked(),
                username: service_username.get_untracked(),
                password: service_password.get_untracked(),
                database: service_database.get_untracked(),
                domain: service_domain.get_untracked(),
                use_tls: service_use_tls.get_untracked(),
                clean_session: service_clean_session.get_untracked(),
                telnet_login_script: service_telnet_login_script.get_untracked(),
                remote_desktop_base: remote_desktop_base.borrow().clone(),
                serial_data_bits: service_serial_data_bits.get_untracked(),
                serial_stop_bits: service_serial_stop_bits.get_untracked(),
                serial_parity: service_serial_parity.get_untracked(),
                serial_flow_control: service_serial_flow_control.get_untracked(),
                forward_ssh_id: service_forward_ssh_id.get_untracked(),
                forward_kind: service_forward_kind.get_untracked(),
                workspace_id: selected_workspace.get_untracked(),
            };
            match build_service_connection(draft) {
                Ok(connection) => {
                    let result = persist_connection(
                        repository,
                        connection,
                        editing_service_id.get_untracked(),
                    );
                    match result {
                        Ok(id) => {
                            items.set(load_connections(Some(repository)));
                            status.set(format!(
                                "{} service connection #{id}",
                                if editing_service_id.get_untracked().is_some() {
                                    "Updated"
                                } else {
                                    "Created"
                                }
                            ));
                            show_service_form.set(false);
                            editing_service_id.set(None);
                            service_name.set(String::new());
                            service_host.set(String::new());
                            service_username.set(String::new());
                            service_password.set(String::new());
                        }
                        Err(error) => status.set(format!("Failed to create connection: {error}")),
                    }
                }
                Err(error) => status.set(error),
            }
        }
    };
    let total = move || items.with(Vec::len);

    view()
        .size_full()
        .child(ui::page_header(
            "Connections",
            format!("{} saved connection(s)", total()),
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .gap_3()
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(ui::primary_button("New database connection").on_press(
                                move || {
                                    editing_database_id.set(None);
                                    selected_workspace.set(None);
                                    show_form.set(true);
                                    show_ssh_form.set(false);
                                    show_service_form.set(false);
                                },
                            ))
                            .child(ui::primary_button("New SSH connection").on_press(move || {
                                editing_ssh_id.set(None);
                                selected_workspace.set(None);
                                ssh_advanced_base.borrow_mut().take();
                                ssh_sftp_account_enabled.set(false);
                                ssh_sftp_username.set(String::new());
                                ssh_sftp_password.set(String::new());
                                ssh_sftp_default_directory.set(String::new());
                                ssh_keyboard_interactive.set(true);
                                ssh_otp_secret.set(String::new());
                                ssh_default_directory.set(String::new());
                                ssh_init_script.set(String::new());
                                ssh_jump_enabled.set(false);
                                ssh_jump_host.set(String::new());
                                ssh_jump_port.set(String::from("22"));
                                ssh_jump_username.set(String::new());
                                ssh_jump_password.set(String::new());
                                ssh_proxy_enabled.set(false);
                                ssh_proxy_host.set(String::new());
                                ssh_proxy_port.set(String::from("1080"));
                                ssh_proxy_username.set(String::new());
                                ssh_proxy_password.set(String::new());
                                ssh_forward_agent.set(false);
                                ssh_x11_forwarding.set(false);
                                show_ssh_form.set(true);
                                show_form.set(false);
                                show_service_form.set(false);
                            }))
                            .child(ui::primary_button("New service connection").on_press(move || {
                                editing_service_id.set(None);
                                selected_workspace.set(None);
                                service_telnet_login_script.set(Vec::new());
                                service_clean_session.set(true);
                                service_serial_data_bits.set(String::from("8"));
                                service_serial_stop_bits.set(String::from("1"));
                                service_serial_parity.set(SerialParity::None);
                                service_serial_flow_control.set(SerialFlowControl::None);
                                service_forward_ssh_id.set(None);
                                service_forward_kind.set(PortForwardingKind::Local);
                                remote_desktop_base.borrow_mut().take();
                                show_service_form.set(true);
                                show_form.set(false);
                                show_ssh_form.set(false);
                            }))
                            .child(ui::secondary_button("Refresh").on_press(reload))
                            .child(
                                text(move || status.get())
                                    .flex_1()
                                    .text_sm()
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .when(show_form.get(), |page| {
                        page.child(
                            ui::card()
                                .gap_3()
                                .child(
                                    text(move || {
                                        if editing_database_id.get().is_some() {
                                            "Edit database connection"
                                        } else {
                                            "New database connection"
                                        }
                                    })
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                                )
                                .child(
                                    view().flex_row().flex_wrap().gap_1().children(
                                        [
                                            DatabaseType::PostgreSQL,
                                            DatabaseType::MySQL,
                                            DatabaseType::MSSQL,
                                            DatabaseType::Oracle,
                                            DatabaseType::SQLite,
                                            DatabaseType::DuckDB,
                                        ]
                                        .into_iter()
                                        .map({
                                            move |kind| database_type_button(kind, database_type)
                                        }),
                                    ),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_3()
                                        .child(form_input("Name", name, "Production PostgreSQL"))
                                        .child(form_input("Host / path", host, "db.example.com")),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_3()
                                        .child(form_input("Port", port, "5432"))
                                        .child(form_input("Username", username, "navop"))
                                        .child(secret_input("Password", password, "Optional")),
                                )
                                .child(form_input(
                                    "Database",
                                    database,
                                    "Optional default database",
                                ))
                                .child(workspace_picker(
                                    workspace_choices.clone(),
                                    selected_workspace,
                                ))
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_2()
                                        .child(ui::primary_button("Save").on_press(create))
                                        .child(
                                            ui::secondary_button("Cancel")
                                                .on_press(move || show_form.set(false)),
                                        ),
                                ),
                        )
                    })
                    .when(show_ssh_form.get(), |page| {
                        page.child(
                            ui::card()
                                .gap_3()
                                .child(
                                    text(move || {
                                        if editing_ssh_id.get().is_some() {
                                            "Edit SSH connection"
                                        } else {
                                            "New SSH connection"
                                        }
                                    })
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_3()
                                        .child(form_input("Name", ssh_name, "Production shell"))
                                        .child(form_input("Host", ssh_host, "shell.example.com")),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_3()
                                        .child(form_input("Port", ssh_port, "22"))
                                        .child(form_input("Username", ssh_username, "root")),
                                )
                                .child(
                                    view().flex_row().gap_1().children(
                                        [
                                            (SshAuthMode::Password, "Password"),
                                            (SshAuthMode::PrivateKey, "Private key"),
                                            (SshAuthMode::Agent, "SSH agent"),
                                        ]
                                        .into_iter()
                                        .map({
                                            move |(mode, label)| {
                                                ssh_auth_button(mode, label, ssh_auth_mode)
                                            }
                                        }),
                                    ),
                                )
                                .when(ssh_auth_mode.get() == SshAuthMode::Password, |form| {
                                    form.child(secret_input(
                                        "Password",
                                        ssh_password,
                                        "SSH password",
                                    ))
                                })
                                .when(ssh_auth_mode.get() == SshAuthMode::PrivateKey, |form| {
                                    form.child(
                                        view()
                                            .flex_row()
                                            .gap_3()
                                            .child(form_input(
                                                "Private key path",
                                                ssh_private_key_path,
                                                "~/.ssh/id_ed25519",
                                            ))
                                            .child(form_input(
                                                "Passphrase",
                                                ssh_passphrase,
                                                "Optional",
                                            )),
                                    )
                                })
                                .when(ssh_auth_mode.get() == SshAuthMode::Agent, |form| {
                                    form.child(
                                        text(
                                            "Uses the local SSH agent. Start or unlock the agent \
                                             before connecting.",
                                        )
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                    )
                                })
                                .child(
                                    view()
                                        .flex_row()
                                        .flex_wrap()
                                        .gap_3()
                                        .child(form_input(
                                            "Default directory",
                                            ssh_default_directory,
                                            "Optional, e.g. /srv/app",
                                        ))
                                        .child(form_input(
                                            "Init script",
                                            ssh_init_script,
                                            "Optional command run after login",
                                        )),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .flex_wrap()
                                        .gap_3()
                                        .child(switch_option(
                                            "Keyboard-interactive / OTP",
                                            ssh_keyboard_interactive,
                                        ))
                                        .child(switch_option(
                                            "Forward SSH agent",
                                            ssh_forward_agent,
                                        ))
                                        .child(switch_option("X11 forwarding", ssh_x11_forwarding)),
                                )
                                .child(secret_input(
                                    "TOTP secret",
                                    ssh_otp_secret,
                                    "Optional base32 secret or otpauth:// URI",
                                ))
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_3()
                                        .child(switch_option(
                                            "Separate SFTP account",
                                            ssh_sftp_account_enabled,
                                        ))
                                        .when(ssh_sftp_account_enabled.get(), |row| {
                                            row.child(form_input(
                                                "SFTP username",
                                                ssh_sftp_username,
                                                "Optional separate account",
                                            ))
                                            .child(secret_input(
                                                "SFTP password",
                                                ssh_sftp_password,
                                                "Optional",
                                            ))
                                        }),
                                )
                                .when(ssh_sftp_account_enabled.get(), |form| {
                                    form.child(form_input(
                                        "SFTP initial directory",
                                        ssh_sftp_default_directory,
                                        "Optional, e.g. /var/www",
                                    ))
                                })
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_3()
                                        .child(switch_option("SSH jump host", ssh_jump_enabled)),
                                )
                                .when(ssh_jump_enabled.get(), |form| {
                                    form.child(
                                        view()
                                            .flex_row()
                                            .flex_wrap()
                                            .gap_3()
                                            .child(form_input(
                                                "Jump host",
                                                ssh_jump_host,
                                                "jump.example.com",
                                            ))
                                            .child(form_input("Jump port", ssh_jump_port, "22"))
                                            .child(form_input(
                                                "Jump username",
                                                ssh_jump_username,
                                                "bastion",
                                            ))
                                            .child(secret_input(
                                                "Jump password",
                                                ssh_jump_password,
                                                "Required",
                                            )),
                                    )
                                })
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_3()
                                        .child(switch_option("SSH proxy", ssh_proxy_enabled))
                                        .when(ssh_proxy_enabled.get(), |row| {
                                            row.child(proxy_type_button(
                                                ProxyType::Socks5,
                                                "SOCKS5",
                                                ssh_proxy_type,
                                            ))
                                            .child(proxy_type_button(
                                                ProxyType::Http,
                                                "HTTP",
                                                ssh_proxy_type,
                                            ))
                                        }),
                                )
                                .when(ssh_proxy_enabled.get(), |form| {
                                    form.child(
                                        view()
                                            .flex_row()
                                            .flex_wrap()
                                            .gap_3()
                                            .child(form_input(
                                                "Proxy host",
                                                ssh_proxy_host,
                                                "127.0.0.1",
                                            ))
                                            .child(form_input(
                                                "Proxy port",
                                                ssh_proxy_port,
                                                "1080",
                                            ))
                                            .child(form_input(
                                                "Proxy username",
                                                ssh_proxy_username,
                                                "Optional",
                                            ))
                                            .child(secret_input(
                                                "Proxy password",
                                                ssh_proxy_password,
                                                "Optional",
                                            )),
                                    )
                                })
                                .child(workspace_picker(
                                    workspace_choices.clone(),
                                    selected_workspace,
                                ))
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_2()
                                        .child(ui::primary_button("Save").on_press(create_ssh))
                                        .child(
                                            ui::secondary_button("Cancel")
                                                .on_press(move || show_ssh_form.set(false)),
                                        ),
                                ),
                        )
                    })
                    .when(show_service_form.get(), |page| {
                        page.child(
                            ui::card()
                                .gap_3()
                                .child(
                                    text(move || {
                                        if editing_service_id.get().is_some() {
                                            "Edit service connection"
                                        } else {
                                            "New service connection"
                                        }
                                    })
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_1()
                                        .child(service_kind_button(
                                            ServiceKind::Redis,
                                            "Redis",
                                            "6379",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::Mongo,
                                            "MongoDB",
                                            "27017",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::Ftp,
                                            "FTP / FTPS",
                                            "21",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::Telnet,
                                            "Telnet",
                                            "23",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::Mqtt,
                                            "MQTT",
                                            "1883",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::Serial,
                                            "Serial",
                                            "115200",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::PortForwarding,
                                            "Tunnel",
                                            "8080",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::Rdp,
                                            "RDP",
                                            "3389",
                                            service_kind,
                                            service_port,
                                        ))
                                        .child(service_kind_button(
                                            ServiceKind::Vnc,
                                            "VNC",
                                            "5900",
                                            service_kind,
                                            service_port,
                                        )),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_3()
                                        .child(form_input(
                                            "Name",
                                            service_name,
                                            "Production service",
                                        ))
                                        .child(form_input_reactive(
                                            move || {
                                                if service_kind.get() == ServiceKind::Serial {
                                                    "Serial port"
                                                } else if service_kind.get()
                                                    == ServiceKind::PortForwarding
                                                {
                                                    "Bind host"
                                                } else {
                                                    "Host"
                                                }
                                            },
                                            service_host,
                                            "Value",
                                        )),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_3()
                                        .child(form_input_reactive(
                                            move || {
                                                if service_kind.get() == ServiceKind::Serial {
                                                    "Baud rate"
                                                } else if service_kind.get()
                                                    == ServiceKind::PortForwarding
                                                {
                                                    "Bind port"
                                                } else {
                                                    "Port"
                                                }
                                            },
                                            service_port,
                                            "Value",
                                        ))
                                        .when(
                                            !matches!(
                                                service_kind.get(),
                                                ServiceKind::Telnet
                                                    | ServiceKind::Serial
                                                    | ServiceKind::PortForwarding
                                            ),
                                            |row| {
                                                row.child(form_input(
                                                    "Username",
                                                    service_username,
                                                    "Optional",
                                                ))
                                                .child(secret_input(
                                                    "Password",
                                                    service_password,
                                                    "Optional",
                                                ))
                                            },
                                        ),
                                )
                                .when(service_kind.get() == ServiceKind::Serial, |form| {
                                    form.child(
                                        view()
                                            .flex_row()
                                            .flex_wrap()
                                            .gap_3()
                                            .child(form_input(
                                                "Data bits",
                                                service_serial_data_bits,
                                                "8",
                                            ))
                                            .child(form_input(
                                                "Stop bits",
                                                service_serial_stop_bits,
                                                "1",
                                            ))
                                            .child(
                                                view()
                                                    .flex_1()
                                                    .min_w(px(180.0))
                                                    .gap_1()
                                                    .child(
                                                        text("Parity")
                                                            .text_xs()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(move || {
                                                                theme().muted_foreground
                                                            }),
                                                    )
                                                    .child(
                                                        view()
                                                            .flex_row()
                                                            .gap_1()
                                                            .children(
                                                                SerialParity::all()
                                                                    .iter()
                                                                    .copied()
                                                                    .map(|parity| {
                                                                        serial_parity_button(
                                                                            parity,
                                                                            service_serial_parity,
                                                                        )
                                                                    }),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                view()
                                                    .flex_1()
                                                    .min_w(px(180.0))
                                                    .gap_1()
                                                    .child(
                                                        text("Flow control")
                                                            .text_xs()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(move || {
                                                                theme().muted_foreground
                                                            }),
                                                    )
                                                    .child(
                                                        view()
                                                            .flex_row()
                                                            .gap_1()
                                                            .children(
                                                                SerialFlowControl::all()
                                                                    .iter()
                                                                    .copied()
                                                                    .map(|flow| {
                                                                        serial_flow_button(
                                                                            flow,
                                                                            service_serial_flow_control,
                                                                        )
                                                                    }),
                                                            ),
                                                    ),
                                            ),
                                    )
                                })
                                .when(service_kind.get() == ServiceKind::Rdp, |form| {
                                    form.child(form_input(
                                        "Domain",
                                        service_domain,
                                        "Optional Windows domain",
                                    ))
                                })
                                .when(
                                    matches!(
                                        service_kind.get(),
                                        ServiceKind::Redis | ServiceKind::Mongo
                                    ),
                                    |form| {
                                        form.child(form_input(
                                            "Database",
                                            service_database,
                                            "Redis DB index or MongoDB database",
                                        ))
                                    },
                                )
                                .when(service_kind.get() == ServiceKind::Mqtt, |form| {
                                    form.child(form_input(
                                        "Client ID",
                                        service_database,
                                        "Optional; generated when empty",
                                    ))
                                })
                                .when(
                                    !matches!(
                                        service_kind.get(),
                                        ServiceKind::Telnet
                                            | ServiceKind::Serial
                                            | ServiceKind::PortForwarding
                                            | ServiceKind::Rdp
                                            | ServiceKind::Vnc
                                    ),
                                    |form| {
                                        form.child(
                                            switch()
                                                .checked(move || service_use_tls.get())
                                                .track_class(
                                                    "w-9 h-5 rounded-full bg-muted checked:bg-primary",
                                                )
                                                .thumb_class(
                                                    "size-4 rounded-full bg-background checked:translate-x-4",
                                                )
                                                .accessibility_label("Use TLS")
                                                .on_change(move |next: &bool| {
                                                    service_use_tls.set(*next)
                                                })
                                                .child(
                                                    text("Use TLS").text_xs().text_color(
                                                        move || theme().muted_foreground,
                                                    ),
                                                ),
                                        )
                                    },
                                )
                                .when(service_kind.get() == ServiceKind::Mqtt, |form| {
                                    form.child(
                                        switch()
                                            .checked(move || service_clean_session.get())
                                            .track_class(
                                                "w-9 h-5 rounded-full bg-muted checked:bg-primary",
                                            )
                                            .thumb_class(
                                                "size-4 rounded-full bg-background checked:translate-x-4",
                                            )
                                            .accessibility_label("Clean session")
                                            .on_change(move |next: &bool| {
                                                service_clean_session.set(*next)
                                            })
                                            .child(
                                                text("Clean session").text_xs().text_color(
                                                    move || theme().muted_foreground,
                                                ),
                                            ),
                                    )
                                })
                                .when(
                                    service_kind.get() == ServiceKind::PortForwarding,
                                    |form| {
                                        form.child(
                                            view()
                                                .gap_2()
                                                .child(
                                                    text("SSH host")
                                                        .text_xs()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(move || {
                                                            theme().muted_foreground
                                                        }),
                                                )
                                                .child(
                                                    view()
                                                        .flex_row()
                                                        .flex_wrap()
                                                        .gap_1()
                                                        .children(
                                                            ssh_choices.iter().cloned().map(
                                                                |(id, name)| {
                                                                    ssh_picker_button(
                                                                        id,
                                                                        name,
                                                                        service_forward_ssh_id,
                                                                    )
                                                                },
                                                            ),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            view()
                                                .flex_row()
                                                .gap_1()
                                                .child(forwarding_kind_button(
                                                    PortForwardingKind::Local,
                                                    service_forward_kind,
                                                ))
                                                .child(forwarding_kind_button(
                                                    PortForwardingKind::Remote,
                                                    service_forward_kind,
                                                ))
                                                .child(forwarding_kind_button(
                                                    PortForwardingKind::Dynamic,
                                                    service_forward_kind,
                                                )),
                                        )
                                        .when(
                                            service_forward_kind.get()
                                                != PortForwardingKind::Dynamic,
                                            |form| {
                                                form.child(
                                                    view()
                                                        .flex_row()
                                                        .gap_3()
                                                        .child(form_input(
                                                            "Target host",
                                                            service_domain,
                                                            "db.internal",
                                                        ))
                                                        .child(form_input(
                                                            "Target port",
                                                            service_database,
                                                            "5432",
                                                        )),
                                                )
                                            },
                                        )
                                    },
                                )
                                .child(workspace_picker(
                                    workspace_choices.clone(),
                                    selected_workspace,
                                ))
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_2()
                                        .child(ui::primary_button("Save").on_press(create_service))
                                        .child(ui::secondary_button("Cancel").on_press(
                                            move || show_service_form.set(false),
                                        )),
                                ),
                        )
                    })
                    .children(items.with(|items| items.clone()).into_iter().map({
                        let repository = repository.clone();
                        let edit_connection = edit_connection.clone();
                        move |connection| {
                            let repository = repository.clone();
                            let edit_connection = edit_connection.clone();
                            connection_row(connection, items, status, repository, edit_connection)
                        }
                    })),
            ),
        )
}

fn load_connections(repository: Option<&ConnectionRepository>) -> Vec<ConnectionRow> {
    repository
        .and_then(|repository| repository.list().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|connection| ConnectionRow {
            id: connection.id,
            name: connection.name,
            connection_type: connection.connection_type,
            kind: connection.connection_type.label().to_owned(),
            detail: connection
                .remark
                .filter(|remark| !remark.trim().is_empty())
                .unwrap_or_else(|| "No description".to_owned()),
            workspace: connection
                .workspace_id
                .map_or("Personal".to_owned(), |id| format!("Workspace #{id}")),
        })
        .collect()
}

fn build_database_connection(draft: DatabaseDraft) -> Result<StoredConnection, String> {
    let name = draft.name.trim().to_owned();
    if name.is_empty() {
        return Err("Connection name is required".to_owned());
    }
    let host = draft.host.trim().to_owned();
    if host.is_empty() {
        return Err("Host or database path is required".to_owned());
    }
    let port = if matches!(
        draft.database_type,
        DatabaseType::SQLite | DatabaseType::DuckDB
    ) {
        0
    } else {
        draft
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| "Port must be between 1 and 65535".to_owned())?
    };
    if !matches!(
        draft.database_type,
        DatabaseType::SQLite | DatabaseType::DuckDB
    ) && port == 0
    {
        return Err("Port must be between 1 and 65535".to_owned());
    }
    let config = DbConnectionConfig {
        id: format!(
            "nsh-pending-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ),
        database_type: draft.database_type,
        name: name.clone(),
        host,
        port,
        username: draft.username.trim().to_owned(),
        password: draft.password,
        database: (!draft.database.trim().is_empty()).then(|| draft.database.trim().to_owned()),
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        credential_reference: None,
        extra_params: Default::default(),
    };
    Ok(StoredConnection::new_database(
        name,
        config,
        draft.workspace_id,
    ))
}

fn build_ssh_connection(draft: SshDraft) -> Result<StoredConnection, String> {
    let name = draft.name.trim().to_owned();
    if name.is_empty() {
        return Err("Connection name is required".to_owned());
    }
    let host = draft.host.trim().to_owned();
    if host.is_empty() {
        return Err("SSH host is required".to_owned());
    }
    let port = draft
        .port
        .trim()
        .parse::<u16>()
        .map_err(|_| "Port must be between 1 and 65535".to_owned())?;
    if port == 0 {
        return Err("Port must be between 1 and 65535".to_owned());
    }
    let username = draft.username.trim().to_owned();
    if username.is_empty() {
        return Err("SSH username is required".to_owned());
    }
    let auth_method = match draft.auth_mode {
        SshAuthMode::Password => {
            if draft.password.is_empty() {
                return Err("SSH password is required".to_owned());
            }
            SshAuthMethod::Password {
                password: draft.password,
            }
        }
        SshAuthMode::PrivateKey => {
            if draft.private_key_path.trim().is_empty() {
                return Err("Private key path is required".to_owned());
            }
            SshAuthMethod::PrivateKey {
                key_path: draft.private_key_path.trim().to_owned(),
                passphrase: (!draft.passphrase.is_empty()).then_some(draft.passphrase),
            }
        }
        SshAuthMode::Agent => SshAuthMethod::Agent,
    };
    let mut params = draft.base.unwrap_or_else(|| SshParams {
        host: String::new(),
        port: 22,
        username: String::new(),
        auth_method: SshAuthMethod::Agent,
        sftp_account: None,
        sftp_default_directory: None,
        credential_reference: None,
        prompt_username: None,
        prompt_password: None,
        keyboard_interactive: None,
        otp: None,
        terminal_encoding: Default::default(),
        terminal_type: Default::default(),
        account_expect: Default::default(),
        connect_timeout: Some(30),
        keepalive_interval: Some(60),
        keepalive_max: Some(3),
        default_directory: None,
        init_script: None,
        disable_shell_integration: None,
        x11_forwarding: None,
        allow_legacy_algorithms: None,
        forward_agent: None,
        agent_forward_key: None,
        jump_server: None,
        disabled_jump_server: None,
        proxy: None,
        os_id: None,
        icon: None,
        icon_file_path: None,
        remote_file: None,
    });
    params.host = host;
    params.port = port;
    params.username = username;
    params.auth_method = auth_method;
    params.sftp_account = if draft.sftp_account_enabled {
        let sftp_username = draft.sftp_username.trim();
        if sftp_username.is_empty() {
            return Err("SFTP account username is required".to_owned());
        }
        Some(SftpAccount {
            username: sftp_username.to_owned(),
            password: draft.sftp_password,
        })
    } else {
        None
    };
    params.sftp_default_directory = optional_value(draft.sftp_default_directory);
    params.keyboard_interactive = Some(draft.keyboard_interactive);
    params.otp = if draft.otp_secret.trim().is_empty() {
        None
    } else {
        Some(
            OtpSecret::parse(draft.otp_secret.trim())
                .map_err(|error| format!("Invalid OTP secret: {error}"))?,
        )
    };
    params.default_directory = optional_value(draft.default_directory);
    params.init_script = optional_value(draft.init_script);
    params.forward_agent = Some(draft.forward_agent);
    params.x11_forwarding = Some(draft.x11_forwarding);
    if draft.jump_enabled {
        let jump_host = draft.jump_host.trim();
        if jump_host.is_empty() {
            return Err("Jump host is required".to_owned());
        }
        let jump_port = service_tcp_port(&draft.jump_port)
            .map_err(|_| "Jump host port must be between 1 and 65535".to_owned())?;
        let jump_username = draft.jump_username.trim();
        if jump_username.is_empty() {
            return Err("Jump host username is required".to_owned());
        }
        if draft.jump_password.is_empty() {
            return Err("Jump host password is required".to_owned());
        }
        let mut jump = params
            .jump_server
            .clone()
            .or_else(|| params.disabled_jump_server.clone())
            .unwrap_or(JumpServerConfig {
                host: String::new(),
                port: 22,
                username: String::new(),
                auth_method: SshAuthMethod::Password {
                    password: String::new(),
                },
                credential_reference: None,
                mode: Default::default(),
                bastion: None,
            });
        jump.host = jump_host.to_owned();
        jump.port = jump_port;
        jump.username = jump_username.to_owned();
        jump.auth_method = SshAuthMethod::Password {
            password: draft.jump_password,
        };
        params.jump_server = Some(jump);
        params.disabled_jump_server = None;
    } else if let Some(jump) = params.jump_server.take() {
        params.disabled_jump_server = Some(jump);
    }
    if draft.proxy_enabled {
        let proxy_host = draft.proxy_host.trim();
        if proxy_host.is_empty() {
            return Err("Proxy host is required".to_owned());
        }
        let proxy_port = service_tcp_port(&draft.proxy_port)
            .map_err(|_| "Proxy port must be between 1 and 65535".to_owned())?;
        let mut proxy = params.proxy.clone().unwrap_or(ProxyConfig {
            proxy_type: draft.proxy_type,
            host: String::new(),
            port: proxy_port,
            username: None,
            password: None,
            credential_reference: None,
        });
        proxy.proxy_type = draft.proxy_type;
        proxy.host = proxy_host.to_owned();
        proxy.port = proxy_port;
        proxy.username = optional_value(draft.proxy_username);
        proxy.password = optional_value(draft.proxy_password);
        params.proxy = Some(proxy);
    } else {
        params.proxy = None;
    }
    Ok(StoredConnection::new_ssh(name, params, draft.workspace_id))
}

fn build_service_connection(draft: ServiceDraft) -> Result<StoredConnection, String> {
    let name = draft.name.trim().to_owned();
    if name.is_empty() {
        return Err("Connection name is required".to_owned());
    }
    let host = draft.host.trim().to_owned();
    if host.is_empty() {
        return Err(if draft.kind == ServiceKind::Serial {
            "Serial port is required".to_owned()
        } else {
            "Host is required".to_owned()
        });
    }
    let username = draft.username.trim().to_owned();
    let password = draft.password;
    match draft.kind {
        ServiceKind::Redis => {
            let port = service_tcp_port(&draft.port)?;
            let db_index = if draft.database.trim().is_empty() {
                0
            } else {
                draft
                    .database
                    .trim()
                    .parse::<u8>()
                    .map_err(|_| "Redis database must be between 0 and 255".to_owned())?
            };
            Ok(StoredConnection::new_redis(
                name,
                RedisParams {
                    host,
                    port,
                    password: optional_value(password.clone()),
                    username: optional_value(username.clone()),
                    credential_reference: None,
                    db_index,
                    mode: Default::default(),
                    use_tls: draft.use_tls,
                    connect_timeout: Some(30),
                    sentinel: None,
                    cluster: None,
                    ssh_tunnel: None,
                },
                draft.workspace_id,
            ))
        }
        ServiceKind::Mongo => {
            let port = service_tcp_port(&draft.port)?;
            let database = optional_value(draft.database);
            Ok(StoredConnection::new_mongodb(
                name,
                MongoDBParams {
                    driver_variant: MongoDriverVariant::default(),
                    connection_string: String::new(),
                    host,
                    port: Some(port),
                    database,
                    username: optional_value(username.clone()),
                    password: optional_value(password.clone()),
                    credential_reference: None,
                    auth_source: None,
                    replica_set: None,
                    read_preference: None,
                    use_srv_record: false,
                    direct_connection: false,
                    use_tls: draft.use_tls,
                    connect_timeout_seconds: Some(30),
                    application_name: Some("Navop".to_owned()),
                    ssh_tunnel: None,
                },
                draft.workspace_id,
            ))
        }
        ServiceKind::Ftp => {
            let port = service_tcp_port(&draft.port)?;
            Ok(StoredConnection::new_ftp(
                name,
                FtpParams {
                    host,
                    port,
                    username,
                    password,
                    credential_reference: None,
                    prompt_username: None,
                    prompt_password: None,
                    passive_mode: true,
                    use_tls: draft.use_tls,
                    connect_timeout: Some(30),
                },
                draft.workspace_id,
            ))
        }
        ServiceKind::Telnet => {
            let port = service_tcp_port(&draft.port)?;
            Ok(StoredConnection::new_telnet(
                name,
                TelnetParams {
                    host,
                    port,
                    credential_reference: None,
                    prompt_username: None,
                    prompt_password: None,
                    backspace_code: Default::default(),
                    login_script: draft.telnet_login_script,
                },
                draft.workspace_id,
            ))
        }
        ServiceKind::Mqtt => {
            let port = service_tcp_port(&draft.port)?;
            Ok(StoredConnection::new_mqtt(
                name,
                MqttParams {
                    host,
                    port,
                    client_id: draft.database.trim().to_owned(),
                    username: optional_value(username),
                    password: optional_value(password),
                    credential_reference: None,
                    use_tls: draft.use_tls,
                    connect_timeout: Some(30),
                    keep_alive: Some(30),
                    mqtt_version: Default::default(),
                    clean_session: draft.clean_session,
                    ssh_tunnel: None,
                },
                draft.workspace_id,
            ))
        }
        ServiceKind::Serial => {
            let baud_rate = draft
                .port
                .trim()
                .parse::<u32>()
                .map_err(|_| "Baud rate must be a positive integer".to_owned())?;
            if baud_rate == 0 {
                return Err("Baud rate must be a positive integer".to_owned());
            }
            let data_bits = draft
                .serial_data_bits
                .trim()
                .parse::<u8>()
                .map_err(|_| "Data bits must be one of 5, 6, 7 or 8".to_owned())?;
            if !(5..=8).contains(&data_bits) {
                return Err("Data bits must be one of 5, 6, 7 or 8".to_owned());
            }
            let stop_bits = draft
                .serial_stop_bits
                .trim()
                .parse::<u8>()
                .map_err(|_| "Stop bits must be 1 or 2".to_owned())?;
            if !matches!(stop_bits, 1 | 2) {
                return Err("Stop bits must be 1 or 2".to_owned());
            }
            Ok(StoredConnection::new_serial(
                name,
                SerialParams {
                    port_name: host,
                    baud_rate,
                    data_bits,
                    stop_bits,
                    parity: draft.serial_parity,
                    flow_control: draft.serial_flow_control,
                },
                draft.workspace_id,
            ))
        }
        ServiceKind::PortForwarding => {
            let ssh_connection_id = draft
                .forward_ssh_id
                .ok_or_else(|| "Select an SSH host for the tunnel".to_owned())?;
            let bind_port = service_tcp_port(&draft.port)?;
            let (target_host, target_port) = if draft.forward_kind == PortForwardingKind::Dynamic {
                (String::new(), 0)
            } else {
                let target_host = draft.domain.trim().to_owned();
                if target_host.is_empty() {
                    return Err("Tunnel target host is required".to_owned());
                }
                let target_port = service_tcp_port(&draft.database)?;
                (target_host, target_port)
            };
            Ok(StoredConnection::new_port_forwarding(
                name,
                PortForwardingParams {
                    ssh_connection_id,
                    kind: draft.forward_kind,
                    bind_host: host,
                    bind_port,
                    target_host,
                    target_port,
                },
                draft.workspace_id,
            ))
        }
        ServiceKind::Rdp | ServiceKind::Vnc => {
            let port = service_tcp_port(&draft.port)?;
            let protocol = if draft.kind == ServiceKind::Rdp {
                RemoteDesktopProtocol::Rdp
            } else {
                RemoteDesktopProtocol::Vnc
            };
            let mut params = draft
                .remote_desktop_base
                .unwrap_or_else(|| RemoteDesktopParams {
                    protocol,
                    host: String::new(),
                    port: if protocol == RemoteDesktopProtocol::Rdp {
                        3389
                    } else {
                        5900
                    },
                    username: None,
                    password: None,
                    credential_reference: None,
                    domain: None,
                    read_only: false,
                    audio_playback: false,
                    proxy: None,
                    backend_preference: Default::default(),
                    rdp: None,
                });
            params.protocol = protocol;
            params.host = host;
            params.port = port;
            params.username = optional_value(username);
            params.password = optional_value(password);
            params.domain = optional_value(draft.domain);
            Ok(StoredConnection::new_remote_desktop(
                name,
                params,
                draft.workspace_id,
            ))
        }
    }
}

fn service_tcp_port(value: &str) -> Result<u16, String> {
    let port = value
        .trim()
        .parse::<u16>()
        .map_err(|_| "Port must be between 1 and 65535".to_owned())?;
    if port == 0 {
        return Err("Port must be between 1 and 65535".to_owned());
    }
    Ok(port)
}

fn optional_value(value: String) -> Option<String> {
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn persist_connection(
    repository: &ConnectionRepository,
    connection: StoredConnection,
    editing_id: Option<i64>,
) -> Result<i64, String> {
    if let Some(id) = editing_id {
        let existing = repository
            .get(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Connection #{id} was not found"))?;
        let connection = preserve_connection_identity(connection, &existing);
        repository
            .update(&connection)
            .map_err(|error| error.to_string())?;
        Ok(id)
    } else {
        let mut connection = connection;
        repository
            .insert(&mut connection)
            .map_err(|error| error.to_string())
    }
}

fn preserve_connection_identity(
    mut connection: StoredConnection,
    existing: &StoredConnection,
) -> StoredConnection {
    connection.id = existing.id;
    connection.credential_revision = existing.credential_revision;
    connection.workspace_id = existing.workspace_id;
    connection.remark = existing.remark.clone();
    connection.sync_enabled = existing.sync_enabled;
    connection.cloud_id = existing.cloud_id.clone();
    connection.last_synced_at = existing.last_synced_at;
    connection.last_used_at = existing.last_used_at;
    connection.sort_order = existing.sort_order;
    connection.created_at = existing.created_at;
    connection.updated_at = existing.updated_at;
    connection.team_id = existing.team_id.clone();
    connection.owner_id = existing.owner_id.clone();
    connection.preferred_open_mode = existing.preferred_open_mode;
    connection.ai_access_level = existing.ai_access_level;
    connection
}

fn connection_row(
    connection: ConnectionRow,
    items: Signal<Vec<ConnectionRow>>,
    status: Signal<String>,
    repository: Option<ConnectionRepository>,
    edit: impl Fn(i64) + Clone + 'static,
) -> impl IntoElement {
    let kind = connection.kind;
    let detail = connection.detail;
    let connection_id = connection.id;
    let connection_type = connection.connection_type;
    ui::card()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
            view()
                .size(px(38.0))
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(10.0))
                .bg(move || theme().muted)
                .child(
                    text(kind.chars().next().unwrap_or('?').to_string())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    text(connection.name)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!("{kind} · {detail}"))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            text(connection.workspace)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .when_some(connection_id, |row, id| {
            let mut row = row;
            if let Some(route) = connection_route(connection_type) {
                row = row.child(
                    ui::primary_button("Open")
                        .on_press(move || crate::route::open_connection(route, id)),
                );
            }
            if connection_type == ConnectionType::SshSftp {
                row = row.child(ui::secondary_button("Terminal").on_press(move || {
                    crate::route::open_connection(crate::route::Route::Terminal, id)
                }));
            }
            row.child(ui::secondary_button("Edit").on_press({
                let edit = edit.clone();
                move || edit(id)
            }))
            .child(ui::secondary_button("Delete").on_press(move || {
                let Some(repository) = repository.as_ref() else {
                    status.set("Connection storage is unavailable".to_owned());
                    return;
                };
                match repository.delete(id) {
                    Ok(()) => {
                        items.set(load_connections(Some(repository)));
                        status.set(format!("Deleted connection #{id}"));
                    }
                    Err(error) => status.set(format!("Delete failed: {error}")),
                }
            }))
        })
}

fn connection_route(connection_type: ConnectionType) -> Option<crate::route::Route> {
    use crate::route::Route;
    match connection_type {
        ConnectionType::Database => Some(Route::Database),
        ConnectionType::Redis => Some(Route::Redis),
        ConnectionType::MongoDB => Some(Route::Mongo),
        ConnectionType::SshSftp | ConnectionType::Ftp => Some(Route::Sftp),
        ConnectionType::Serial | ConnectionType::Telnet => Some(Route::Terminal),
        ConnectionType::PortForwarding => Some(Route::PortForwarding),
        ConnectionType::Rdp | ConnectionType::Vnc => Some(Route::RemoteDesktop),
        _ => None,
    }
}

fn database_type_button(kind: DatabaseType, selected: Signal<DatabaseType>) -> impl IntoElement {
    let label = kind.as_str().to_owned();
    let border_kind = kind.clone();
    let bg_kind = kind.clone();
    let state_kind = kind.clone();
    let press_kind = kind;
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == border_kind {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == bg_kind {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == state_kind,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(press_kind.clone()))
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

fn ssh_auth_button(
    mode: SshAuthMode,
    label: &'static str,
    selected: Signal<SshAuthMode>,
) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == mode {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == mode {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == mode,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(mode))
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

fn proxy_type_button(
    kind: ProxyType,
    label: &'static str,
    selected: Signal<ProxyType>,
) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == kind {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == kind {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == kind,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(kind))
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

fn ssh_picker_button(id: i64, label: String, selected: Signal<Option<i64>>) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == Some(id) {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == Some(id) {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == Some(id),
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(Some(id)))
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

fn forwarding_kind_button(
    kind: PortForwardingKind,
    selected: Signal<PortForwardingKind>,
) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == kind {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == kind {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == kind,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(kind))
        .child(
            text(kind.label())
                .text_xs()
                .text_color(move || theme().foreground),
        )
}

fn service_kind_button(
    kind: ServiceKind,
    label: &'static str,
    default_port: &'static str,
    selected: Signal<ServiceKind>,
    port: Signal<String>,
) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == kind {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == kind {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == kind,
            ..AccessibilityState::default()
        })
        .on_press(move || {
            selected.set(kind);
            port.set(default_port.to_owned());
        })
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

fn serial_parity_button(parity: SerialParity, selected: Signal<SerialParity>) -> impl IntoElement {
    view()
        .px_2()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == parity {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == parity {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == parity,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(parity))
        .child(
            text(parity.label())
                .text_size(px(10.0))
                .text_color(move || theme().foreground),
        )
}

fn serial_flow_button(
    flow: SerialFlowControl,
    selected: Signal<SerialFlowControl>,
) -> impl IntoElement {
    view()
        .px_2()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == flow {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == flow {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == flow,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(flow))
        .child(
            text(flow.label())
                .text_size(px(10.0))
                .text_color(move || theme().foreground),
        )
}

fn switch_option(label: &'static str, checked: Signal<bool>) -> impl IntoElement {
    switch()
        .checked(move || checked.get())
        .track_class("w-9 h-5 rounded-full bg-muted checked:bg-primary")
        .thumb_class("size-4 rounded-full bg-background checked:translate-x-4")
        .accessibility_label(label)
        .on_change(move |next: &bool| checked.set(*next))
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

fn form_input(
    label: impl Into<SharedString>,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoElement {
    let label = label.into();
    view()
        .flex_1()
        .min_w(px(180.0))
        .gap_1()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            input()
                .value(move || value.get())
                .on_change(move |next: &str| value.set(next.to_owned()))
                .placeholder(placeholder)
                .h(px(36.0))
                .px_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().background)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn form_input_reactive(
    label: impl Fn() -> &'static str + Clone + 'static,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoElement {
    let text_label = label.clone();
    view()
        .flex_1()
        .min_w(px(180.0))
        .gap_1()
        .child(
            text(move || text_label())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            input()
                .value(move || value.get())
                .on_change(move |next: &str| value.set(next.to_owned()))
                .placeholder(placeholder)
                .h(px(36.0))
                .px_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().background)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn secret_input(
    label: impl Into<SharedString>,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoElement {
    let label = label.into();
    view()
        .flex_1()
        .min_w(px(180.0))
        .gap_1()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            input()
                .masked(true)
                .value(move || value.get())
                .on_change(move |next: &str| value.set(next.to_owned()))
                .placeholder(placeholder)
                .h(px(36.0))
                .px_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().background)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn workspace_picker(
    choices: Vec<(i64, String)>,
    selected: Signal<Option<i64>>,
) -> impl IntoElement {
    view()
        .gap_1()
        .child(
            text("Workspace")
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .child(workspace_button(None, "Personal".to_owned(), selected))
                .children(
                    choices
                        .into_iter()
                        .map(move |(id, name)| workspace_button(Some(id), name, selected)),
                ),
        )
}

fn workspace_button(
    id: Option<i64>,
    label: String,
    selected: Signal<Option<i64>>,
) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == id {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == id {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == id,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(id))
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: DatabaseType) -> DatabaseDraft {
        DatabaseDraft {
            name: "Local".to_owned(),
            database_type: kind,
            host: "localhost".to_owned(),
            port: "5432".to_owned(),
            username: "navop".to_owned(),
            password: String::new(),
            database: "app".to_owned(),
            workspace_id: None,
        }
    }

    #[test]
    fn database_draft_builds_a_stored_connection() {
        let stored = build_database_connection(draft(DatabaseType::PostgreSQL)).unwrap();
        assert_eq!(
            stored.connection_type,
            one_core::storage::ConnectionType::Database
        );
        let config = stored.to_db_connection().unwrap();
        assert_eq!(config.database_type, DatabaseType::PostgreSQL);
        assert_eq!(config.port, 5432);
        assert_eq!(config.database.as_deref(), Some("app"));
    }

    #[test]
    fn file_databases_do_not_require_a_tcp_port() {
        let mut input = draft(DatabaseType::SQLite);
        input.port = "not-a-port".to_owned();
        assert!(build_database_connection(input).is_ok());
    }

    fn ssh_draft() -> SshDraft {
        SshDraft {
            name: "Bastion".to_owned(),
            host: "bastion.example.com".to_owned(),
            port: "22".to_owned(),
            username: "deploy".to_owned(),
            auth_mode: SshAuthMode::Password,
            password: "secret".to_owned(),
            private_key_path: String::new(),
            passphrase: String::new(),
            sftp_account_enabled: false,
            sftp_username: String::new(),
            sftp_password: String::new(),
            sftp_default_directory: String::new(),
            keyboard_interactive: true,
            otp_secret: String::new(),
            default_directory: String::new(),
            init_script: String::new(),
            jump_enabled: false,
            jump_host: String::new(),
            jump_port: "22".to_owned(),
            jump_username: String::new(),
            jump_password: String::new(),
            proxy_enabled: false,
            proxy_type: ProxyType::Socks5,
            proxy_host: String::new(),
            proxy_port: "1080".to_owned(),
            proxy_username: String::new(),
            proxy_password: String::new(),
            forward_agent: false,
            x11_forwarding: false,
            base: None,
            workspace_id: None,
        }
    }

    #[test]
    fn ssh_draft_builds_password_and_agent_connections() {
        let password = build_ssh_connection(ssh_draft()).unwrap();
        assert_eq!(
            password.connection_type,
            one_core::storage::ConnectionType::SshSftp
        );
        assert!(matches!(
            password.to_ssh_params().unwrap().auth_method,
            SshAuthMethod::Password { .. }
        ));

        let mut agent = ssh_draft();
        agent.auth_mode = SshAuthMode::Agent;
        agent.password.clear();
        assert!(matches!(
            build_ssh_connection(agent)
                .unwrap()
                .to_ssh_params()
                .unwrap()
                .auth_method,
            SshAuthMethod::Agent
        ));
    }

    #[test]
    fn ssh_password_and_private_key_modes_validate_required_secrets() {
        let mut password = ssh_draft();
        password.password.clear();
        assert!(build_ssh_connection(password).is_err());

        let mut private_key = ssh_draft();
        private_key.auth_mode = SshAuthMode::PrivateKey;
        private_key.password.clear();
        assert!(build_ssh_connection(private_key).is_err());
    }

    #[test]
    fn ssh_advanced_options_build_jump_proxy_sftp_and_otp() {
        let mut advanced = ssh_draft();
        advanced.jump_enabled = true;
        advanced.jump_host = "jump.example.com".to_owned();
        advanced.jump_port = "2200".to_owned();
        advanced.jump_username = "bastion".to_owned();
        advanced.jump_password = "jump-secret".to_owned();
        advanced.proxy_enabled = true;
        advanced.proxy_type = ProxyType::Http;
        advanced.proxy_host = "proxy.example.com".to_owned();
        advanced.proxy_port = "8080".to_owned();
        advanced.sftp_account_enabled = true;
        advanced.sftp_username = "files".to_owned();
        advanced.sftp_password = "sftp-secret".to_owned();
        advanced.sftp_default_directory = "/srv/files".to_owned();
        advanced.keyboard_interactive = true;
        advanced.otp_secret = "JBSWY3DPEHPK3PXP".to_owned();
        advanced.default_directory = "/srv/app".to_owned();
        advanced.init_script = "export ENV=prod".to_owned();
        advanced.forward_agent = true;
        advanced.x11_forwarding = true;

        let params = build_ssh_connection(advanced)
            .unwrap()
            .to_ssh_params()
            .unwrap();
        let jump = params.jump_server.expect("jump server");
        assert_eq!(jump.host, "jump.example.com");
        assert_eq!(jump.port, 2200);
        assert!(matches!(
            jump.auth_method,
            SshAuthMethod::Password { password } if password == "jump-secret"
        ));
        let proxy = params.proxy.expect("proxy");
        assert_eq!(proxy.proxy_type, ProxyType::Http);
        assert_eq!(proxy.host, "proxy.example.com");
        assert_eq!(proxy.port, 8080);
        assert_eq!(
            params
                .sftp_account
                .as_ref()
                .map(|account| account.username.as_str()),
            Some("files")
        );
        assert_eq!(params.sftp_default_directory.as_deref(), Some("/srv/files"));
        assert_eq!(
            params.otp.as_ref().map(|otp| otp.secret.as_str()),
            Some("JBSWY3DPEHPK3PXP")
        );
        assert_eq!(params.default_directory.as_deref(), Some("/srv/app"));
        assert_eq!(params.init_script.as_deref(), Some("export ENV=prod"));
        assert_eq!(params.forward_agent, Some(true));
        assert_eq!(params.x11_forwarding, Some(true));
    }

    #[test]
    fn updates_preserve_local_identity_and_sync_metadata() {
        let mut existing = build_database_connection(draft(DatabaseType::PostgreSQL)).unwrap();
        existing.id = Some(42);
        existing.credential_revision = Some(7);
        existing.workspace_id = Some(3);
        existing.team_id = Some("team-1".to_owned());
        existing.owner_id = Some("owner-1".to_owned());
        existing.sync_enabled = false;
        let mut replacement = build_database_connection(draft(DatabaseType::MySQL)).unwrap();
        replacement.name = "Renamed".to_owned();
        let merged = preserve_connection_identity(replacement, &existing);
        assert_eq!(merged.id, Some(42));
        assert_eq!(merged.credential_revision, Some(7));
        assert_eq!(merged.workspace_id, Some(3));
        assert_eq!(merged.team_id.as_deref(), Some("team-1"));
        assert_eq!(merged.owner_id.as_deref(), Some("owner-1"));
        assert!(!merged.sync_enabled);
        assert_eq!(merged.name, "Renamed");
        assert_eq!(
            merged.to_db_connection().unwrap().database_type,
            DatabaseType::MySQL
        );
    }

    fn service_draft(kind: ServiceKind) -> ServiceDraft {
        ServiceDraft {
            kind,
            name: "Cache".to_owned(),
            host: "cache.example.com".to_owned(),
            port: if kind == ServiceKind::Redis {
                "6379".to_owned()
            } else {
                "27017".to_owned()
            },
            username: "navop".to_owned(),
            password: "secret".to_owned(),
            database: if kind == ServiceKind::Redis {
                "2".to_owned()
            } else {
                "app".to_owned()
            },
            domain: String::new(),
            use_tls: true,
            clean_session: true,
            telnet_login_script: Vec::new(),
            remote_desktop_base: None,
            serial_data_bits: "8".to_owned(),
            serial_stop_bits: "1".to_owned(),
            serial_parity: SerialParity::None,
            serial_flow_control: SerialFlowControl::None,
            forward_ssh_id: None,
            forward_kind: PortForwardingKind::Local,
            workspace_id: Some(5),
        }
    }

    #[test]
    fn service_draft_builds_redis_mongodb_ftp_and_telnet_connections() {
        let redis = build_service_connection(service_draft(ServiceKind::Redis)).unwrap();
        assert_eq!(
            redis.connection_type,
            one_core::storage::ConnectionType::Redis
        );
        assert_eq!(redis.workspace_id, Some(5));
        let params = redis.to_redis_params().unwrap();
        assert_eq!(params.db_index, 2);
        assert!(params.use_tls);

        let mongo = build_service_connection(service_draft(ServiceKind::Mongo)).unwrap();
        assert_eq!(
            mongo.connection_type,
            one_core::storage::ConnectionType::MongoDB
        );
        let params = mongo.to_mongodb_params().unwrap();
        assert_eq!(params.database.as_deref(), Some("app"));
        assert!(params.use_tls);

        let ftp = build_service_connection(service_draft(ServiceKind::Ftp)).unwrap();
        assert_eq!(ftp.connection_type, one_core::storage::ConnectionType::Ftp);
        assert!(ftp.to_ftp_params().unwrap().use_tls);

        let telnet = build_service_connection(service_draft(ServiceKind::Telnet)).unwrap();
        assert_eq!(
            telnet.connection_type,
            one_core::storage::ConnectionType::Telnet
        );

        let rdp = build_service_connection(service_draft(ServiceKind::Rdp)).unwrap();
        assert_eq!(rdp.connection_type, one_core::storage::ConnectionType::Rdp);

        let vnc = build_service_connection(service_draft(ServiceKind::Vnc)).unwrap();
        assert_eq!(vnc.connection_type, one_core::storage::ConnectionType::Vnc);
    }

    #[test]
    fn mqtt_draft_preserves_client_identity_tls_and_session_cleanup() {
        let mut draft = service_draft(ServiceKind::Mqtt);
        draft.port = "8883".to_owned();
        draft.database = "navop-monitor".to_owned();
        draft.clean_session = false;

        let connection = build_service_connection(draft).unwrap();
        assert_eq!(
            connection.connection_type,
            one_core::storage::ConnectionType::Mqtt
        );
        let params = connection.to_mqtt_params().unwrap();
        assert_eq!(params.host, "cache.example.com");
        assert_eq!(params.port, 8883);
        assert_eq!(params.client_id, "navop-monitor");
        assert!(params.use_tls);
        assert!(!params.clean_session);
    }

    #[test]
    fn serial_draft_maps_line_settings_and_validates_them() {
        let mut draft = service_draft(ServiceKind::Serial);
        draft.host = "/dev/ttyUSB0".to_owned();
        draft.port = "921600".to_owned();
        draft.serial_data_bits = "7".to_owned();
        draft.serial_stop_bits = "2".to_owned();
        draft.serial_parity = SerialParity::Even;
        draft.serial_flow_control = SerialFlowControl::Hardware;

        let connection = build_service_connection(draft).unwrap();
        assert_eq!(
            connection.connection_type,
            one_core::storage::ConnectionType::Serial
        );
        let params = connection.to_serial_params().unwrap();
        assert_eq!(params.port_name, "/dev/ttyUSB0");
        assert_eq!(params.baud_rate, 921600);
        assert_eq!(params.data_bits, 7);
        assert_eq!(params.stop_bits, 2);
        assert_eq!(params.parity, SerialParity::Even);
        assert_eq!(params.flow_control, SerialFlowControl::Hardware);

        let mut invalid = service_draft(ServiceKind::Serial);
        invalid.host = String::new();
        assert!(build_service_connection(invalid).is_err());

        let mut invalid = service_draft(ServiceKind::Serial);
        invalid.port = "0".to_owned();
        assert!(build_service_connection(invalid).is_err());

        let mut invalid = service_draft(ServiceKind::Serial);
        invalid.serial_data_bits = "9".to_owned();
        assert!(build_service_connection(invalid).is_err());
    }

    #[test]
    fn port_forwarding_draft_requires_an_ssh_host_and_target_for_non_dynamic_modes() {
        let mut draft = service_draft(ServiceKind::PortForwarding);
        draft.host = "127.0.0.1".to_owned();
        draft.port = "8080".to_owned();
        draft.domain = "db.internal".to_owned();
        draft.database = "5432".to_owned();
        draft.forward_ssh_id = Some(7);
        draft.forward_kind = PortForwardingKind::Local;

        let connection = build_service_connection(draft).unwrap();
        let params = connection.to_port_forwarding_params().unwrap();
        assert_eq!(params.ssh_connection_id, 7);
        assert_eq!(params.bind_port, 8080);
        assert_eq!(params.target_host, "db.internal");
        assert_eq!(params.target_port, 5432);

        let mut invalid = service_draft(ServiceKind::PortForwarding);
        invalid.forward_ssh_id = None;
        assert!(build_service_connection(invalid).is_err());

        let mut dynamic = service_draft(ServiceKind::PortForwarding);
        dynamic.forward_ssh_id = Some(7);
        dynamic.forward_kind = PortForwardingKind::Dynamic;
        dynamic.domain.clear();
        dynamic.database.clear();
        let params = build_service_connection(dynamic)
            .unwrap()
            .to_port_forwarding_params()
            .unwrap();
        assert_eq!(params.kind, PortForwardingKind::Dynamic);
        assert_eq!(params.target_port, 0);
    }

    #[test]
    fn remote_desktop_edit_preserves_advanced_parameters() {
        let mut draft = service_draft(ServiceKind::Rdp);
        draft.port = "3389".to_owned();
        draft.remote_desktop_base = Some(RemoteDesktopParams {
            protocol: RemoteDesktopProtocol::Rdp,
            host: "old.example.com".to_owned(),
            port: 3389,
            username: None,
            password: None,
            credential_reference: None,
            domain: None,
            read_only: true,
            audio_playback: true,
            proxy: None,
            backend_preference: Default::default(),
            rdp: None,
        });
        let connection = build_service_connection(draft).unwrap();
        let params = connection.to_remote_desktop_params().unwrap();
        assert_eq!(params.host, "cache.example.com");
        assert!(params.read_only);
        assert!(params.audio_playback);
    }

    #[test]
    fn connection_rows_route_to_the_matching_workspace() {
        assert_eq!(
            connection_route(ConnectionType::Database),
            Some(crate::route::Route::Database)
        );
        assert_eq!(
            connection_route(ConnectionType::Redis),
            Some(crate::route::Route::Redis)
        );
        assert_eq!(
            connection_route(ConnectionType::MongoDB),
            Some(crate::route::Route::Mongo)
        );
        assert_eq!(
            connection_route(ConnectionType::SshSftp),
            Some(crate::route::Route::Sftp)
        );
        assert_eq!(
            connection_route(ConnectionType::Rdp),
            Some(crate::route::Route::RemoteDesktop)
        );
        assert_eq!(
            connection_route(ConnectionType::Vnc),
            Some(crate::route::Route::RemoteDesktop)
        );
        assert_eq!(
            connection_route(ConnectionType::Telnet),
            Some(crate::route::Route::Terminal)
        );
        assert_eq!(
            connection_route(ConnectionType::Serial),
            Some(crate::route::Route::Terminal)
        );
    }
}
