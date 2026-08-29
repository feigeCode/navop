# Database Importers

Use this for applications that import database connections.

## Database Mapping Checklist

For each source profile, identify:

- Database engine type.
- Host or file path.
- Port.
- Username.
- Default database/schema.
- Connection display name.
- Stable source id.
- Password location and encryption behavior.
- SSH tunnel, SSL, proxy, or advanced params.

Keep database records conservative. Unsupported advanced options should become warnings or `extra_params`, not invented top-level fields.

## Engine Type Inference

Applications may store database type in different places:

- Explicit field: `type`, `driver`, `DatabaseType`, `serviceprovider`.
- Driver class: Java/JDBC driver name.
- URL prefix: `jdbc:mysql://`, `postgres://`, `mongodb://`.
- Parent path or group key: `MySQL`, `PostgreSQL`, `Oracle`.
- Filename or workspace metadata.

Do not trust labels such as `Default` as engine types. If the explicit field is generic, infer from stable surrounding context and test that variant.

## Navicat

Known paths:

```text
~/Library/Application Support/PremiumSoft CyberTech/Navicat*/Profiles/*.plist
~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist
%APPDATA%/PremiumSoft CyberTech/Navicat CC/Common/conn.plist
```

Navicat Premium Lite details:

- Lite shared config lives at `Navicat CC/Common/conn.plist`.
- `conn.plist` can be XML or binary plist.
- Use `plist::Value::from_reader(Cursor::new(bytes))` for both XML and binary support.
- Lite fields may be lowercase: `host`, `port`, `username`, `defaultdatabase`.
- `serviceprovider = Default` is not the database type.
- Infer database type from key/path segments such as `MySQL`, `PostgreSQL`, `Oracle`, or SQL Server containers.
- Skip nested parameter dictionaries such as `ssh_param`, `http_param`, `ssl_param`, and `compatibility_param`.

Fixture requirements:

- Classic plist profile with explicit type.
- Lite `conn.plist` with lowercase fields.
- Lite record where type is inferred from the path.
- Parameter dictionaries that must not produce records.

## DBeaver

Use DBeaver as the reference for workspace-style database importers:

- Multiple files may need to be read from a workspace.
- Driver or connection type may be stored separately from credentials.
- Host code should not special-case DBeaver. If a host capability is missing, add it generically.

Common DBeaver lessons:

- Path and permission expansion must work before parser code matters.
- UI preview work must not block the GPUI thread.
- Packaging visibility and preview parsing are separate phases.

## SQLite and File Databases

For local file databases:

- Preserve the file path.
- Use a stable display name from the profile if present, otherwise filename.
- Do not pretend there is a network host.
- Use warnings if the source app references a missing local file.

## Secrets

Database passwords often have product-specific storage:

- Plaintext in config: include only when `include_passwords` is true.
- OS keychain: use host secret APIs when available.
- Product-encrypted blob: unsupported unless the project has an approved decryptor and tests.
- Cloud sync vault: unsupported unless the local file contains usable secrets and user opted in.

Always test both `include_passwords = false` and `true` when password import is supported.
