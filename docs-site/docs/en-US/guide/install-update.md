# Install, update, and file associations

Navop provides desktop builds for macOS, Windows, and Linux. Match the package to the operating system and CPU architecture. Before upgrading, save SQL and Notes, finish manual transactions, and allow file transfers to complete.

## Download and install

Choose the latest stable release from [GitHub Releases](https://github.com/feigeCode/navop/releases). On macOS, select Apple Silicon or Intel and move the app into Applications. On Windows, run the matching installer. On Linux, use the package format documented on the release page and ensure that the desktop environment permits graphical applications.

If Gatekeeper blocks the first macOS launch, verify the official release source and allow the app in Privacy & Security. Treat Windows or Linux security warnings the same way: confirm provenance rather than disabling system-wide protections. Managed devices may require administrator approval.

## Complete first-launch setup

Choose a language, theme, and start page. Database, SSH, SFTP, and remote desktop features need network access; grant local-network, firewall, keychain, or file permissions only when they match the resources you intend to use. Notes folders, external editors, and custom fonts require their own filesystem access.

Create a non-production test connection before importing real credentials. Install an extension only when you need its database driver, remote desktop provider, connection importer, or ACP Agent.

## Update and roll back

Enable automatic update checks in Settings or check manually. Close active connections, commit or roll back manual transactions, and finish SFTP transfers before applying an update. After restart, verify important connections, extensions, and keyboard shortcuts.

If a new release is incompatible with a critical extension, back up the Navop data directory and reinstall a known stable package from Releases. Downgrading is not a substitute for backup: local configuration formats may evolve, so confirm compatibility before opening older versions.

## Open associated files

Navop can open `.db`, `.duckdb`, and `.md` through operating-system file associations. Database files create or open local SQLite/DuckDB connections; Markdown files open in Notes. If the association is missing, choose Navop with the system Open With action and optionally make it the default.

Do not open a production database file that another process is actively writing. Copy it to a safe location first. External Markdown keeps paths relative to its original folder, so moving it may break images and linked resources.

## Uninstall without losing data

Removing the application may leave local settings, encrypted connections, Notes, and extension caches in the user data directory. Preserve that directory for a reinstall. For a complete removal, export required material, stop sync, hand over team responsibilities, and then remove both the app and user data.

Reinstallation cannot recover a forgotten master key. Confirm master-key and team-key recovery arrangements before deleting local data.
