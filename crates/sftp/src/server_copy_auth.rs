use crate::TransferCancelled;
use crate::remote_exec::{
    RemoteCommandOutput, RemoteCommandTimeout, exec_remote_command_with_input_deadline,
};
use anyhow::{Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use sha2::{Digest, Sha256};
use ssh::{SshConnectConfig, SshSessionManager};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::Instant;
use zeroize::Zeroize;

const DIRECT_COPY_KEY_SETUP_TIMEOUT: Duration = Duration::from_secs(45);
const DIRECT_COPY_KEY_PREFIX: &str = "navop_direct_copy_";
const DIRECT_COPY_KEY_COMMENT: &str = "navop-direct-copy";
const AUTHORIZED_KEY_RESTRICTIONS: &str =
    "no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-pty";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectCopyIdentity {
    file_name: String,
}

impl DirectCopyIdentity {
    pub(crate) fn for_endpoints(
        source: &SshConnectConfig,
        target: &SshConnectConfig,
    ) -> DirectCopyIdentity {
        let mut hasher = Sha256::new();
        for component in [
            "navop-direct-copy-key-v1",
            &source.username,
            &source.host,
            &source.port.to_string(),
            &target.username,
            &target.host,
            &target.port.to_string(),
        ] {
            hasher.update(component.as_bytes());
            hasher.update([0]);
        }
        let digest = hasher.finalize();
        let mut suffix = String::with_capacity(32);
        for byte in &digest[..16] {
            write!(&mut suffix, "{byte:02x}").expect("writing to String cannot fail");
        }
        DirectCopyIdentity {
            file_name: format!("{DIRECT_COPY_KEY_PREFIX}{suffix}"),
        }
    }

    pub(crate) fn file_name(&self) -> &str {
        &self.file_name
    }
}

#[derive(Debug, PartialEq, Eq)]
struct DirectCopyPublicKey {
    key_type: String,
    blob: String,
}

impl DirectCopyPublicKey {
    fn parse(output: &str) -> Result<Self> {
        if output.is_empty() || output.chars().any(char::is_control) || output.lines().count() != 1
        {
            bail!("source server returned an invalid dedicated SSH public key");
        }
        let fields = output.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 2 {
            bail!("source server returned an invalid dedicated SSH public key");
        }
        let key_type = fields[0];
        if !matches!(key_type, "ssh-ed25519" | "ssh-rsa") {
            bail!("source server returned an unsupported dedicated SSH public key type");
        }
        let decoded = BASE64
            .decode(fields[1].as_bytes())
            .map_err(|_| anyhow!("source server returned invalid SSH public key data"))?;
        validate_wire_key_type(&decoded, key_type)?;
        Ok(Self {
            key_type: key_type.to_string(),
            blob: fields[1].to_string(),
        })
    }

    fn authorized_key_line(&self) -> String {
        format!(
            "{AUTHORIZED_KEY_RESTRICTIONS} {} {} {DIRECT_COPY_KEY_COMMENT}\n",
            self.key_type, self.blob
        )
    }
}

pub(crate) async fn configure_direct_copy_auth(
    source: &SshSessionManager,
    target: &SshSessionManager,
    identity: &DirectCopyIdentity,
    cancelled: Arc<AtomicBool>,
) -> Result<()> {
    ensure_not_cancelled(&cancelled)?;
    let source_command = build_source_key_command(identity)?;
    let source_output = exec_remote_command_with_input_deadline(
        source,
        &source_command,
        &[],
        cancelled.clone(),
        Instant::now() + DIRECT_COPY_KEY_SETUP_TIMEOUT,
    )
    .await
    .map_err(|error| setup_error("generate a dedicated SSH key on the source server", error))?;
    if source_output.exit_status != 0 {
        bail!(
            "Navop could not generate a dedicated SSH key on the source server (status {}): {}. \
The private key was not copied from the source server, and Navop relay was not started",
            source_output.exit_status,
            command_error(&source_output)
        );
    }
    let public_key = DirectCopyPublicKey::parse(source_output.stdout.trim_end()).map_err(|error| {
        anyhow!(
            "Navop generated a dedicated SSH key on the source server but could not validate its \
public key: {error}. The private key remains on the source server, and Navop relay was not started"
        )
    })?;

    ensure_not_cancelled(&cancelled)?;
    let target_command =
        build_target_authorized_keys_command(public_key.authorized_key_line().as_bytes().len())?;
    let mut public_key_payload = public_key.authorized_key_line().into_bytes();
    let target_result = exec_remote_command_with_input_deadline(
        target,
        &target_command,
        &public_key_payload,
        cancelled,
        Instant::now() + DIRECT_COPY_KEY_SETUP_TIMEOUT,
    )
    .await;
    public_key_payload.zeroize();
    let target_output = target_result.map_err(|error| {
        setup_error("install the source public key on the target server", error)
    })?;
    if target_output.exit_status != 0 {
        bail!(
            "Navop generated the dedicated key on the source server but could not install its \
public key on the target server (status {}): {}. The private key remains only on the source \
server, and Navop relay was not started",
            target_output.exit_status,
            command_error(&target_output)
        );
    }
    Ok(())
}

pub(crate) fn build_source_key_command(identity: &DirectCopyIdentity) -> Result<String> {
    validate_identity_file_name(identity.file_name())?;
    Ok(format!(
        "set -eu\n\
umask 077\n\
[ -n \"${{HOME:-}}\" ] || {{ echo 'Navop could not determine the source home directory' >&2; exit 72; }}\n\
navop_ssh_dir=\"$HOME/.ssh\"\n\
navop_key=\"$navop_ssh_dir/{file_name}\"\n\
navop_lock=\"$navop_key.lock\"\n\
navop_tmp_dir=\n\
navop_tmp=\n\
navop_tmp_pub=\n\
navop_locked=0\n\
navop_cleanup() {{\n\
  navop_status=$?\n\
  trap - EXIT HUP INT TERM\n\
  [ -z \"$navop_tmp\" ] || rm -f \"$navop_tmp\"\n\
  [ -z \"$navop_tmp_pub\" ] || rm -f \"$navop_tmp_pub\"\n\
  [ -z \"$navop_tmp_dir\" ] || rm -rf \"$navop_tmp_dir\"\n\
  [ \"$navop_locked\" -eq 0 ] || rmdir \"$navop_lock\" 2>/dev/null || true\n\
  exit \"$navop_status\"\n\
}}\n\
trap navop_cleanup EXIT HUP INT TERM\n\
if [ -e \"$navop_ssh_dir\" ] || [ -L \"$navop_ssh_dir\" ]; then\n\
  [ -d \"$navop_ssh_dir\" ] && [ ! -L \"$navop_ssh_dir\" ] || {{ echo 'Source .ssh path is not a regular directory' >&2; exit 73; }}\n\
else\n\
  mkdir \"$navop_ssh_dir\"\n\
fi\n\
chmod 700 \"$navop_ssh_dir\"\n\
navop_attempt=0\n\
while ! mkdir \"$navop_lock\" 2>/dev/null; do\n\
  navop_attempt=$((navop_attempt + 1))\n\
  [ \"$navop_attempt\" -lt 16 ] || {{ echo 'Navop timed out waiting to configure the source SSH key' >&2; exit 75; }}\n\
  sleep 1\n\
done\n\
navop_locked=1\n\
if [ -e \"$navop_key\" ] || [ -L \"$navop_key\" ]; then\n\
  [ -f \"$navop_key\" ] && [ ! -L \"$navop_key\" ] || {{ echo 'Navop dedicated SSH key path is not a regular file' >&2; exit 73; }}\n\
else\n\
  navop_tmp_dir=$(mktemp -d \"$navop_ssh_dir/.navop-key.XXXXXX\")\n\
  navop_tmp=\"$navop_tmp_dir/key\"\n\
  navop_tmp_pub=\"$navop_tmp.pub\"\n\
  if ! ssh-keygen -q -t ed25519 -N '' -C '{comment}' -f \"$navop_tmp\"; then\n\
    rm -f \"$navop_tmp\" \"$navop_tmp_pub\"\n\
    ssh-keygen -q -t rsa -b 3072 -N '' -C '{comment}' -f \"$navop_tmp\"\n\
  fi\n\
  chmod 600 \"$navop_tmp\"\n\
  mv \"$navop_tmp\" \"$navop_key\"\n\
  navop_tmp=\n\
  if [ -f \"$navop_tmp_pub\" ]; then\n\
    mv \"$navop_tmp_pub\" \"$navop_key.pub\"\n\
  fi\n\
  navop_tmp_pub=\n\
  rmdir \"$navop_tmp_dir\"\n\
  navop_tmp_dir=\n\
fi\n\
chmod 600 \"$navop_key\"\n\
ssh-keygen -y -f \"$navop_key\"\n",
        file_name = identity.file_name(),
        comment = DIRECT_COPY_KEY_COMMENT,
    ))
}

pub(crate) fn build_target_authorized_keys_command(payload_length: usize) -> Result<String> {
    if payload_length == 0 || payload_length > 16 * 1024 {
        bail!("invalid direct copy public-key payload length");
    }
    Ok(format!(
        "set -eu\n\
umask 077\n\
[ -n \"${{HOME:-}}\" ] || {{ echo 'Navop could not determine the target home directory' >&2; exit 72; }}\n\
navop_ssh_dir=\"$HOME/.ssh\"\n\
navop_authorized_keys=\"$navop_ssh_dir/authorized_keys\"\n\
navop_lock=\"$navop_ssh_dir/.navop-authorized-keys.lock\"\n\
navop_payload=\n\
navop_tmp=\n\
navop_locked=0\n\
navop_cleanup() {{\n\
  navop_status=$?\n\
  trap - EXIT HUP INT TERM\n\
  [ -z \"$navop_payload\" ] || rm -f \"$navop_payload\"\n\
  [ -z \"$navop_tmp\" ] || rm -f \"$navop_tmp\"\n\
  [ \"$navop_locked\" -eq 0 ] || rmdir \"$navop_lock\" 2>/dev/null || true\n\
  exit \"$navop_status\"\n\
}}\n\
trap navop_cleanup EXIT HUP INT TERM\n\
if [ -e \"$navop_ssh_dir\" ] || [ -L \"$navop_ssh_dir\" ]; then\n\
  [ -d \"$navop_ssh_dir\" ] && [ ! -L \"$navop_ssh_dir\" ] || {{ echo 'Target .ssh path is not a regular directory' >&2; exit 73; }}\n\
else\n\
  mkdir \"$navop_ssh_dir\"\n\
fi\n\
chmod 700 \"$navop_ssh_dir\"\n\
navop_attempt=0\n\
while ! mkdir \"$navop_lock\" 2>/dev/null; do\n\
  navop_attempt=$((navop_attempt + 1))\n\
  [ \"$navop_attempt\" -lt 16 ] || {{ echo 'Navop timed out waiting to update authorized_keys' >&2; exit 75; }}\n\
  sleep 1\n\
done\n\
navop_locked=1\n\
if [ -e \"$navop_authorized_keys\" ] || [ -L \"$navop_authorized_keys\" ]; then\n\
  [ -f \"$navop_authorized_keys\" ] && [ ! -L \"$navop_authorized_keys\" ] || {{ echo 'Target authorized_keys is not a regular file' >&2; exit 73; }}\n\
  chmod 600 \"$navop_authorized_keys\"\n\
fi\n\
navop_payload=$(mktemp \"$navop_ssh_dir/.navop-public-key.XXXXXX\")\n\
dd bs=1 count={payload_length} of=\"$navop_payload\" 2>/dev/null\n\
[ \"$(wc -c < \"$navop_payload\")\" -eq {payload_length} ] || {{ echo 'Navop public-key payload was incomplete' >&2; exit 125; }}\n\
navop_key_type=$(awk 'NR == 1 {{ print $2 }}' \"$navop_payload\")\n\
navop_key_blob=$(awk 'NR == 1 {{ print $3 }}' \"$navop_payload\")\n\
[ -n \"$navop_key_type\" ] && [ -n \"$navop_key_blob\" ] || {{ echo 'Navop public-key payload was invalid' >&2; exit 65; }}\n\
if [ -f \"$navop_authorized_keys\" ] && awk -v key_type=\"$navop_key_type\" -v key_blob=\"$navop_key_blob\" '\n\
  $0 !~ /^[[:space:]]*#/ {{ for (field = 1; field < NF; field++) if ($field == key_type && $(field + 1) == key_blob) found = 1 }}\n\
  END {{ exit found ? 0 : 1 }}\n\
' \"$navop_authorized_keys\"; then\n\
  exit 0\n\
fi\n\
navop_tmp=$(mktemp \"$navop_ssh_dir/.navop-authorized-keys.XXXXXX\")\n\
if [ -f \"$navop_authorized_keys\" ]; then\n\
  awk '1' \"$navop_authorized_keys\" > \"$navop_tmp\"\n\
fi\n\
cat \"$navop_payload\" >> \"$navop_tmp\"\n\
chmod 600 \"$navop_tmp\"\n\
mv \"$navop_tmp\" \"$navop_authorized_keys\"\n\
navop_tmp=\n\
chmod 600 \"$navop_authorized_keys\"\n",
    ))
}

fn validate_identity_file_name(file_name: &str) -> Result<()> {
    if !file_name.starts_with(DIRECT_COPY_KEY_PREFIX)
        || !file_name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        bail!("invalid dedicated SSH key file name");
    }
    Ok(())
}

fn validate_wire_key_type(decoded: &[u8], expected: &str) -> Result<()> {
    let mut remaining = decoded;
    let wire_type = take_ssh_string(&mut remaining)?;
    if wire_type != expected.as_bytes() {
        bail!("source server returned mismatched SSH public key data");
    }
    match expected {
        "ssh-ed25519" => {
            if take_ssh_string(&mut remaining)?.len() != 32 {
                bail!("source server returned invalid SSH public key data");
            }
        }
        "ssh-rsa" => {
            if take_ssh_string(&mut remaining)?.is_empty()
                || take_ssh_string(&mut remaining)?.is_empty()
            {
                bail!("source server returned invalid SSH public key data");
            }
        }
        _ => bail!("source server returned unsupported SSH public key data"),
    }
    if !remaining.is_empty() {
        bail!("source server returned invalid SSH public key data");
    }
    Ok(())
}

fn take_ssh_string<'a>(remaining: &mut &'a [u8]) -> Result<&'a [u8]> {
    let Some(length_bytes) = remaining.get(..4) else {
        bail!("source server returned invalid SSH public key data");
    };
    let length = u32::from_be_bytes(
        length_bytes
            .try_into()
            .expect("four-byte slice has the required length"),
    ) as usize;
    let Some(end) = 4usize.checked_add(length) else {
        bail!("source server returned invalid SSH public key data");
    };
    let Some(value) = remaining.get(4..end) else {
        bail!("source server returned invalid SSH public key data");
    };
    *remaining = &remaining[end..];
    Ok(value)
}

fn setup_error(action: &'static str, error: anyhow::Error) -> anyhow::Error {
    if error.is::<TransferCancelled>() {
        return error;
    }
    if error.is::<RemoteCommandTimeout>() {
        anyhow!(
            "Navop timed out while trying to {action}: {error}. The dedicated private key was not \
sent to Navop or the target server, and Navop relay was not started"
        )
    } else {
        anyhow!(
            "Navop could not {action}: {error}. The dedicated private key was not sent to Navop or \
the target server, and Navop relay was not started"
        )
    }
}

fn command_error(output: &RemoteCommandOutput) -> String {
    let message = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    if message.is_empty() {
        "remote command returned no error output".to_string()
    } else {
        message.to_string()
    }
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AUTHORIZED_KEY_RESTRICTIONS, DirectCopyIdentity, DirectCopyPublicKey,
        build_source_key_command, build_target_authorized_keys_command,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use ssh::{HostKeyVerifier, SshAuth, SshConnectConfig};
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::process::{Command, Stdio};

    fn config(username: &str, host: &str, port: u16) -> SshConnectConfig {
        SshConnectConfig {
            host: host.to_string(),
            port,
            username: username.to_string(),
            auth: SshAuth::Agent,
            timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            jump_server: None,
            proxy: None,
            keyboard_interactive_responder: None,
            host_key_verifier: HostKeyVerifier::default(),
            x11_forwarding: false,
            allow_legacy_algorithms: false,
        }
    }

    fn public_key(key_type: &str) -> String {
        let mut wire = Vec::new();
        push_ssh_string(&mut wire, key_type.as_bytes());
        match key_type {
            "ssh-ed25519" => push_ssh_string(&mut wire, &[7; 32]),
            "ssh-rsa" => {
                push_ssh_string(&mut wire, &[1, 0, 1]);
                push_ssh_string(&mut wire, &[0, 0x80, 1, 2, 3, 4]);
            }
            _ => push_ssh_string(&mut wire, b"test-key-material"),
        }
        format!("{key_type} {}", BASE64.encode(wire))
    }

    fn push_ssh_string(wire: &mut Vec<u8>, value: &[u8]) {
        wire.extend_from_slice(&(value.len() as u32).to_be_bytes());
        wire.extend_from_slice(value);
    }

    #[test]
    fn identity_name_is_stable_endpoint_specific_and_shell_safe() {
        let source = config("root", "source.example", 22);
        let target = config("deploy", "target.example", 2222);
        let same = DirectCopyIdentity::for_endpoints(&source, &target);
        let changed =
            DirectCopyIdentity::for_endpoints(&source, &config("deploy", "target.example", 22));

        assert_eq!(same, DirectCopyIdentity::for_endpoints(&source, &target));
        assert_ne!(same, changed);
        assert!(same.file_name().starts_with("navop_direct_copy_"));
        assert!(
            same.file_name()
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        );
        assert!(!same.file_name().contains("source.example"));
        assert!(!same.file_name().contains("target.example"));
    }

    #[test]
    fn source_key_command_generates_without_overwriting_or_printing_private_key() {
        let identity = DirectCopyIdentity::for_endpoints(
            &config("root", "source", 22),
            &config("root", "target", 22),
        );
        let command = build_source_key_command(&identity).expect("source key command");

        assert!(command.contains("umask 077"));
        assert!(command.contains("[ ! -L \"$navop_ssh_dir\" ]"));
        assert!(command.contains("mkdir \"$navop_ssh_dir\""));
        assert!(command.contains("chmod 700 \"$navop_ssh_dir\""));
        assert!(command.contains("[ -f \"$navop_key\" ] && [ ! -L \"$navop_key\" ]"));
        assert!(command.contains("ssh-keygen -q -t ed25519 -N ''"));
        assert!(command.contains("ssh-keygen -q -t rsa -b 3072 -N ''"));
        assert!(command.contains("ssh-keygen -y -f \"$navop_key\""));
        assert!(!command.contains("cat \"$navop_key\""));
        assert!(!command.contains("PRIVATE KEY"));
    }

    #[cfg(unix)]
    #[test]
    fn source_key_generation_is_idempotent_and_keeps_private_key_on_source() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let home = directory.path().join("home");
        let bin = directory.path().join("bin");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&bin).expect("create bin");
        let generated_count = directory.path().join("generated-count");
        let fake_ssh_keygen = bin.join("ssh-keygen");
        std::fs::write(
            &fake_ssh_keygen,
            r#"#!/bin/sh
set -eu
if [ "${1-}" = "-y" ]; then
  printf '%s\n' "$NAVOP_FAKE_PUBLIC_KEY"
  exit 0
fi
key_path=
previous=
for argument in "$@"; do
  if [ "$previous" = "-f" ]; then
    key_path=$argument
  fi
  previous=$argument
done
[ -n "$key_path" ]
printf 'private-key-generated-on-source\n' > "$key_path"
printf '%s generated\n' "$NAVOP_FAKE_PUBLIC_KEY" > "$key_path.pub"
printf 'generated\n' >> "$NAVOP_GENERATED_COUNT"
"#,
        )
        .expect("fake ssh-keygen");
        let mut permissions = std::fs::metadata(&fake_ssh_keygen)
            .expect("fake ssh-keygen metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fake_ssh_keygen, permissions)
            .expect("make fake ssh-keygen executable");

        let identity = DirectCopyIdentity::for_endpoints(
            &config("root", "source", 22),
            &config("root", "target", 22),
        );
        let command = build_source_key_command(&identity).expect("source key command");
        let fake_public_key = public_key("ssh-ed25519");
        let inherited_path = std::env::var_os("PATH").unwrap_or_default();

        for _ in 0..2 {
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(&command)
                .env("HOME", &home)
                .env(
                    "PATH",
                    format!("{}:{}", bin.display(), inherited_path.to_string_lossy()),
                )
                .env("NAVOP_FAKE_PUBLIC_KEY", &fake_public_key)
                .env("NAVOP_GENERATED_COUNT", &generated_count)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("execute source key command");
            assert!(
                output.status.success(),
                "source key setup failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                format!("{fake_public_key}\n"),
                String::from_utf8(output.stdout).expect("public key output")
            );
        }

        let private_key = home.join(".ssh").join(identity.file_name());
        assert_eq!(
            "private-key-generated-on-source\n",
            std::fs::read_to_string(&private_key).expect("private key")
        );
        assert_eq!(
            "generated\n",
            std::fs::read_to_string(&generated_count).expect("generation count")
        );
        assert_eq!(
            0o600,
            std::fs::metadata(private_key)
                .expect("private key metadata")
                .permissions()
                .mode()
                & 0o777
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_key_generation_rejects_private_key_symlink() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let home = directory.path();
        let ssh_dir = home.join(".ssh");
        std::fs::create_dir(&ssh_dir).expect("create .ssh");
        let outside = home.join("outside");
        std::fs::write(&outside, "unchanged\n").expect("outside file");
        let identity = DirectCopyIdentity::for_endpoints(
            &config("root", "source", 22),
            &config("root", "target", 22),
        );
        std::os::unix::fs::symlink(&outside, ssh_dir.join(identity.file_name()))
            .expect("private key symlink");
        let command = build_source_key_command(&identity).expect("source key command");
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .env("HOME", home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("execute source key command");

        assert!(!output.status.success());
        assert_eq!(
            "unchanged\n",
            std::fs::read_to_string(outside).expect("outside file")
        );
    }

    #[test]
    fn public_key_parser_accepts_generated_types_and_rejects_untrusted_text() {
        for key_type in ["ssh-ed25519", "ssh-rsa"] {
            let key = public_key(key_type);
            let parsed = DirectCopyPublicKey::parse(&key).expect("valid public key");
            assert_eq!(key_type, parsed.key_type);
            assert!(
                parsed
                    .authorized_key_line()
                    .starts_with(AUTHORIZED_KEY_RESTRICTIONS)
            );
        }
        for invalid in [
            "",
            "ssh-ed25519 not-base64",
            "ssh-dss AAAA",
            "ssh-ed25519 AAAA\nssh-rsa AAAA",
            "ssh-ed25519 AAAA comment",
        ] {
            assert!(
                DirectCopyPublicKey::parse(invalid).is_err(),
                "{invalid:?} should be rejected"
            );
        }
    }

    #[test]
    fn public_key_parser_rejects_mismatched_wire_type() {
        let rsa_blob = public_key("ssh-rsa")
            .split_once(' ')
            .expect("key fields")
            .1
            .to_string();
        assert!(DirectCopyPublicKey::parse(&format!("ssh-ed25519 {rsa_blob}")).is_err());
    }

    #[test]
    fn public_key_parser_rejects_trailing_wire_data() {
        let key = public_key("ssh-ed25519");
        let (key_type, blob) = key.split_once(' ').expect("key fields");
        let mut decoded = BASE64.decode(blob).expect("key blob");
        decoded.extend_from_slice(b"trailing");

        assert!(
            DirectCopyPublicKey::parse(&format!("{key_type} {}", BASE64.encode(decoded))).is_err()
        );
    }

    #[test]
    fn target_install_command_is_atomic_locked_and_symlink_safe() {
        let command = build_target_authorized_keys_command(128).expect("install command");

        assert!(command.contains("chmod 700 \"$navop_ssh_dir\""));
        assert!(command.contains(".navop-authorized-keys.lock"));
        assert!(command.contains("mktemp \"$navop_ssh_dir/.navop-authorized-keys."));
        assert!(command.contains("[ ! -L \"$navop_authorized_keys\" ]"));
        assert!(command.contains("chmod 600 \"$navop_authorized_keys\""));
        assert!(command.contains("mv \"$navop_tmp\" \"$navop_authorized_keys\""));
        assert!(command.contains("$field == key_type && $(field + 1) == key_blob"));
        assert!(!command.contains("> \"$navop_authorized_keys\""));
    }

    #[cfg(unix)]
    #[test]
    fn target_install_is_idempotent_and_preserves_existing_keys() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let home = directory.path();
        let ssh_dir = home.join(".ssh");
        std::fs::create_dir(&ssh_dir).expect("create .ssh");
        let authorized_keys = ssh_dir.join("authorized_keys");
        let new_key = DirectCopyPublicKey::parse(&public_key("ssh-ed25519"))
            .expect("new key")
            .authorized_key_line();
        let existing = format!(
            "{} existing-comment\n# {}\n",
            public_key("ssh-rsa"),
            new_key.trim_end()
        );
        std::fs::write(&authorized_keys, &existing).expect("existing authorized_keys");
        let command = build_target_authorized_keys_command(new_key.len()).expect("install command");

        for _ in 0..2 {
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg(&command)
                .env("HOME", home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("run install command");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(new_key.as_bytes())
                .expect("write public key");
            let output = child.wait_with_output().expect("wait for install");
            assert!(
                output.status.success(),
                "install failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let installed = std::fs::read_to_string(&authorized_keys).expect("authorized_keys");
        assert!(installed.contains(&existing));
        assert_eq!(
            1,
            installed
                .lines()
                .filter(|line| *line == new_key.trim_end())
                .count()
        );
        assert_eq!(
            0o700,
            std::fs::metadata(&ssh_dir)
                .expect(".ssh metadata")
                .permissions()
                .mode()
                & 0o777
        );
        assert_eq!(
            0o600,
            std::fs::metadata(&authorized_keys)
                .expect("authorized_keys metadata")
                .permissions()
                .mode()
                & 0o777
        );
    }

    #[cfg(unix)]
    #[test]
    fn target_install_rejects_authorized_keys_symlink() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let home = directory.path();
        let ssh_dir = home.join(".ssh");
        std::fs::create_dir(&ssh_dir).expect("create .ssh");
        let outside = home.join("outside");
        std::fs::write(&outside, "unchanged\n").expect("outside file");
        std::os::unix::fs::symlink(&outside, ssh_dir.join("authorized_keys"))
            .expect("authorized_keys symlink");
        let new_key = DirectCopyPublicKey::parse(&public_key("ssh-ed25519"))
            .expect("new key")
            .authorized_key_line();
        let command = build_target_authorized_keys_command(new_key.len()).expect("install command");
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run install command");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(new_key.as_bytes())
            .expect("write public key");
        let output = child.wait_with_output().expect("wait for install");

        assert!(!output.status.success());
        assert_eq!(
            "unchanged\n",
            std::fs::read_to_string(outside).expect("outside file")
        );
    }
}
