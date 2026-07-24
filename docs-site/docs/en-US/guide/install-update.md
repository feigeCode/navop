# Install, update, and file associations

Navop provides desktop builds for macOS, Windows, and Linux. Match the package to the operating system and CPU architecture. Before upgrading, save SQL and Notes, finish manual transactions, and allow file transfers to complete.

## Download and install

Choose the latest stable release from [GitHub Releases](https://github.com/feigeCode/navop/releases). On macOS, select Apple Silicon or Intel and move the app into Applications. On Windows, run the matching installer. On Linux, use the package format documented on the release page and ensure that the desktop environment permits graphical applications.

If Gatekeeper blocks the first macOS launch, verify the official release source and allow the app in Privacy & Security. Treat Windows or Linux security warnings the same way: confirm provenance rather than disabling system-wide protections. Managed devices may require administrator approval.

## Choose an installation package

| Platform | Device/architecture | Recommended file | Typical use |
| --- | --- | --- | --- |
| macOS | Apple Silicon | Apple Silicon `.dmg` or `.tar.gz` | M-series Macs |
| macOS | Intel | Intel `.dmg` or `.tar.gz` | Intel Macs |
| Windows | x86_64 | `.msi` | Standard installation, Start menu, desktop shortcuts, and stable file associations |
| Windows | x86_64 | `.zip` | Portable use or a manually managed folder |
| Linux | x86_64 | `.deb`, `.rpm`, `.AppImage`, or `.tar.gz` | Select for the distribution and desktop environment |

Use `sha256sums.txt` from the same release to verify download integrity.

## Windows portable edition

### Extract and start

The official Windows `.zip` is the portable edition and contains:

```text
navop.exe
navop.portable
```

`navop.portable` is the marker that enables portable mode. Keep it in the same directory as `navop.exe`; do not run Navop directly inside the ZIP, and do not normally delete or rename the marker. Fully extract the archive to a regular directory that the current user can write to, for example:

```text
D:\Apps\NavopPortable\
├── navop.exe
└── navop.portable
```

Double-click `navop.exe`, or start it from PowerShell:

```powershell
.\navop.exe
```

On first launch, Navop creates a `data` directory next to the executable:

```text
D:\Apps\NavopPortable\
├── navop.exe
├── navop.portable
└── data\
    ├── config\
    ├── state\
    └── cache\
```

The portable directory must be writable. Do not put it under `Program Files`, in a read-only directory, or on read-only media. A USB drive or external disk must allow writes and remain connected reliably. Navop refuses to start when the portable directory is not writable.

### Data directory and master key

The portable edition stores configuration, application state, and cache under `data/config`, `data/state`, and `data/cache`. This makes the application and its data easy to move or back up together. However, **portable mode never persists the master key locally, so you must enter the master key on every launch**. Copying `data`, extracting the ZIP again, or reinstalling Navop cannot recover a forgotten master key.

Keep the master key separately; do not store it as plain text in the portable directory or on the same USB drive. The `data` directory may contain connection configuration, state, extensions, and caches. Do not publish it, commit it to Git, or place it in an untrusted cloud-synchronized folder.

### Update a portable installation

Portable mode does not support installing updates in the app, and automatic update checks are skipped. You can still check manually to learn that a release is available, but confirming the update opens GitHub Releases so that you can download a new Windows `.zip`.

Do not overwrite an old directory that is still in use. Use this upgrade procedure:

1. Save SQL, Notes, and remote files; commit or roll back manual transactions; and wait for SFTP and remote-editing tasks to finish.
2. Quit Navop completely.
3. Back up the old portable directory, or at least its complete `data` directory.
4. Download the new Windows `.zip` for the matching architecture and extract it into a new, empty directory.
5. Copy the entire old `data` directory into the new directory, next to the new `navop.exe`.
6. Confirm that the new directory still contains `navop.portable` next to `navop.exe`.
7. Start the new version, enter the original master key, and verify the version, connections, extensions, Notes, theme, and keyboard shortcuts.
8. Delete the old directory only after verification. Keep it temporarily if you may need to roll back.

For example:

```text
D:\Apps\
├── NavopPortable-old\
│   ├── navop.exe
│   ├── navop.portable
│   └── data\
└── NavopPortable-new\
    ├── navop.exe
    ├── navop.portable
    └── data\   ← copied from the old version
```

### Move, associate files, and protect the folder

Quit Navop and finish transactions, transfers, and remote-editing tasks before moving the portable folder. You can normally move the entire folder, but do not let two Navop instances or two computers write to the same `data` directory at the same time.

Portable mode does not automatically register Windows associations for `.db`, `.duckdb`, or `.md`. You can still open files from inside Navop or manually select `navop.exe` with Windows Open With. A manually configured Open With path may stop working after you move the portable directory. Choose the `.msi` edition when you need stable file associations, Start menu or desktop shortcuts, in-app updates, or a master key persisted by the operating system.

Losing removable media can expose encrypted data and related metadata. A moved copy still requires the correct master key. Do not delete the original folder or backup until the new copy has been verified.

### Advanced startup options

The official Windows ZIP already includes `navop.portable`, so normal use requires no additional options. For testing or custom deployment, portable mode and the data location can also be selected explicitly:

```powershell
# Enable portable mode temporarily; use data next to navop.exe
.\navop.exe --portable

# Select a data directory; this option also enables portable paths
.\navop.exe --data-dir "E:\NavopData"

# Enable portable mode with an environment variable
$env:NAVOP_PORTABLE = "1"
.\navop.exe

# Select a data directory with an environment variable
$env:NAVOP_DATA_DIR = "E:\NavopData"
.\navop.exe
```

`NAVOP_PORTABLE` accepts `1`, `true`, `yes`, or `on`. Data-location precedence is `--data-dir`, `--portable`, `NAVOP_DATA_DIR`, `NAVOP_PORTABLE`/`navop.portable`, and finally standard installed mode. The selected directory must be writable. Prefer an absolute path because a relative path is resolved from the process's current working directory.

## Complete first-launch setup

Choose a language, theme, and start page. Database, SSH, SFTP, and remote desktop features need network access; grant local-network, firewall, keychain, or file permissions only when they match the resources you intend to use. Notes folders, external editors, and custom fonts require their own filesystem access.

Create a non-production test connection before importing real credentials. Install an extension only when you need its database driver, remote desktop provider, connection importer, or ACP Agent.

## Update and roll back

For a standard installation, enable automatic update checks in Settings or check manually. Close active connections, commit or roll back manual transactions, and finish SFTP transfers before applying an update. After restart, verify important connections, extensions, and keyboard shortcuts. Follow the separate portable update procedure above for the Windows `.zip` edition.

If a new release is incompatible with a critical extension, back up the Navop data directory and reinstall a known stable package from Releases. Downgrading is not a substitute for backup: local configuration formats may evolve, so confirm compatibility before opening older versions.

## Open associated files

Navop can open `.db`, `.duckdb`, and `.md` through operating-system file associations. Database files create or open local SQLite/DuckDB connections; Markdown files open in Notes. If the association is missing, choose Navop with the system Open With action and optionally make it the default.

Do not open a production database file that another process is actively writing. Copy it to a safe location first. External Markdown keeps paths relative to its original folder, so moving it may break images and linked resources.

## Uninstall without losing data

Removing the application may leave local settings, encrypted connections, Notes, and extension caches in the user data directory. Preserve that directory for a reinstall. For a complete removal, export required material, stop sync, hand over team responsibilities, and then remove both the app and user data.

Reinstallation cannot recover a forgotten master key. Confirm master-key and team-key recovery arrangements before deleting local data.
