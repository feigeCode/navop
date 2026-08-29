# OpenSSH Sources

Use this when implementing or debugging OpenSSH-derived importers.

## Candidate Files

Declare both config and fallback discovery files:

```text
~/.ssh/config
%USERPROFILE%/.ssh/config
~/.ssh/known_hosts
%USERPROFILE%/.ssh/known_hosts
```

Many machines have `known_hosts` but no `config`. If only config is scanned, the UI can show OpenSSH as available from the manifest while preview returns zero records.

## OpenSSH Config Parsing

Import `Host` blocks conservatively:

- `Host`: aliases for display names and source ids.
- `HostName`: actual endpoint host; default to alias if absent.
- `User`: username; default to empty string if absent.
- `Port`: endpoint port; default to `22` if absent.
- `IdentityFile`: private key path; use the first supported value if multiple appear.

Parsing rules:

- Strip comments outside quoted values.
- Handle quoted directive values.
- Treat directive names case-insensitively.
- Ignore global directives before the first `Host` block unless the importer intentionally supports inheritance.
- Skip aliases with `*`, `?`, or `!` because they are patterns, not concrete hosts.

Do not over-import complex OpenSSH behavior until tests prove the mapping:

- `Include`
- `Match`
- `ProxyJump`
- `ProxyCommand`
- per-host inheritance across multiple blocks
- token expansion such as `%h`, `%p`, `%r`

If unsupported fields are common in the fixture, return warnings rather than silently pretending they were imported.

## known_hosts Parsing

`known_hosts` is host discovery only. It does not include usernames or auth settings.

Import:

- Plain host: `example.com ssh-ed25519 ...`
- Host with port: `[example.com]:2222 ssh-ed25519 ...`
- Comma-separated aliases: `example.com,192.0.2.10 ssh-ed25519 ...`

Skip:

- Hashed entries starting with `|`.
- Wildcards or negated patterns.
- Marker lines such as `@cert-authority` and `@revoked`.
- Malformed bracketed host/port values.

For imported entries:

- `username`: empty string.
- `port`: parsed port or `22`.
- `auth_method`: `AutoPublicKey`.
- `password_status`: `unsupported`.
- Deduplicate by `(host, port)`.

## Record Identity

Use deterministic ids:

```text
openssh-config:<host-alias-slug>
openssh-config:known-hosts-<host-port-slug>
```

Use `source_id` to preserve origin:

```text
~/.ssh/config:prod-db
~/.ssh/known_hosts:example.com:2222
```

## Fixtures

Include fixtures for:

- Simple `Host` block.
- Multiple aliases in one `Host` line.
- `HostName`, `User`, `Port`, and `IdentityFile`.
- Pattern aliases that must be skipped.
- `known_hosts` plain host.
- `known_hosts` bracketed port.
- `known_hosts` hashed entry skipped.
- Config absent but known_hosts present.
