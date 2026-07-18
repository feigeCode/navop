# Database connections

Navop includes MySQL, PostgreSQL, SQLite, DuckDB, SQL Server, Oracle, and ClickHouse support. Extensions can add Dameng, Kingbase, GBase 8s, OceanBase, openGauss, IoTDB, and other drivers. Fields and advanced capabilities depend on the driver and server version.

## Connect to a network database

Choose the database type and enter a name, host, port, user, password, and default database. Test networking, authentication, and driver compatibility before saving. Diagnose failures in layers: DNS, port, firewall, account, database state, and version. Avoid changing many fields at once.

Some drivers expose a default schema, character set, or extra parameters. Use least-privilege accounts in production; a routine query account should not automatically have DROP, TRUNCATE, or global administrative rights.

## Open local database files

SQLite and DuckDB connections can select local files, and `.db` or `.duckdb` files can open through the system association. Confirm path permissions and locking. If another application is writing the file, analyze a copy rather than allowing multiple tools to write concurrently.

Network drives and cloud-synced folders can introduce locking or consistency problems. Back up important files before writes. Removing the Navop connection does not normally remove the file, but SQL writes still modify it.

## Configure proxies and SSH tunnels

Use SOCKS5 or HTTP CONNECT when direct access is unavailable, with proxy authentication if required. A proxy changes the route; it does not replace database TLS or authorization. Test the proxy address, credentials, destination, and policy separately.

An SSH tunnel can reuse a saved SSH/SFTP connection or define a jump host manually. Authentication supports passwords, private-key files, private-key contents, and SSH Agent. Configure the actual tunnel target when the database is not on the SSH host itself. Test SSH first, then the database.

## Configure SSL/TLS

Provide a CA, client certificate, client private key, and validation mode when the driver supports them. Production systems should validate server certificates. Hostname mismatches and incorrect system time can cause failures. Keep certificate private keys out of repositories and synced Notes.

After certificate rotation, retest the connection and confirm that an extension driver supports the required TLS options.

## Filter schemas and manage context

Schema filters can hide system objects, include selected schemas, exclude selected schemas, or show everything. They affect navigation only; they do not change account permissions or prevent direct SQL access. When an object appears missing, check the current database, schema, name case, permissions, and filter.

Close dependent editors, data tabs, and comparison jobs before changing connection parameters so active sessions do not continue with stale settings.
