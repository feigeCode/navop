# Install, update, and file associations

Navop provides desktop builds for macOS, Windows, and Linux. Match the package to the operating system and CPU architecture. Before upgrading, save SQL and Notes, finish manual transactions, and allow file transfers to complete.

## Download and install

Choose the latest stable release from the [official Download Center](https://navop.dev/en-US/extensions). On macOS, select Apple Silicon or Intel and move the app into Applications. On Windows, run the matching installer. On Linux, use the package format documented on the release page and ensure that the desktop environment permits graphical applications.

If Gatekeeper blocks the first macOS launch, verify the official release source and allow the app in Privacy & Security. If the app is still quarantined after the source is confirmed, run `sudo xattr -rd com.apple.quarantine /Applications/Navop.app` and reopen it. Treat Windows or Linux security warnings the same way: confirm provenance rather than disabling system-wide protections. Managed devices may require administrator approval.

## Choose an installation package

| Platform | Device/architecture | Recommended file | Typical use |
| --- | --- | --- | --- |
| macOS | Apple Silicon | `navop-<version>-macos-arm64.dmg` or `navop-<version>-macos-arm64.tar.gz` | M-series Macs |
| macOS | Intel | `navop-<version>-macos-x64.dmg` or `navop-<version>-macos-x64.tar.gz` | Intel Macs |
| Windows | x86_64 | `navop-<version>-windows-x64.msi` | MSI installer with Start menu, desktop shortcuts, and stable file associations |
| Windows | x86_64 | `navop-<version>-windows-x64.exe` | EXE installer wrapping the same standard per-user MSI installation |
| Windows | x86_64 | `navop-<version>-windows-x64.zip` | No-install use with data kept in the normal Windows user directories |
| Windows | x86_64 | `navop-<version>-windows-x64-portable.zip` | Keep the application and data together in a movable folder |
| Linux | x86_64 | `navop-<version>-linux-x64.tar.gz`, `navop_<version>_amd64.deb`, `navop-<version>-1.x86_64.rpm`, `navop_<version>_amd64.AppImage` | Select for the distribution and desktop environment |
| Linux | x86_64 | `navop-<version>-linux-x64-portable.tar.gz` | Keep the application and data together in a movable folder; see "Linux portable archive" below |
| Linux | ARM64 | `navop-<version>-linux-arm64.tar.gz` | ARM64 devices |
| Linux | ARM64 | `navop-<version>-linux-arm64-portable.tar.gz` | Keep the application and data together in a movable folder; see "Linux portable archive" below |
| Linux | x86_64 / ARM64 | `navop-<version>-linux-x64-gpu-stack.tar.gz`, `navop-<version>-linux-arm64-gpu-stack.tar.gz` | Only when the system lacks a usable Mesa/EGL renderer; combines with any Linux package above |

Use `sha256sums.txt` from the same release to verify download integrity.

## Install via Scoop (Windows)

Navop is available in the official Scoop `extras` bucket. Scoop installs the portable edition and persists the `data` directory for you:

```powershell
scoop bucket add extras
scoop install navop
```

Upgrade later with `scoop update navop`. A Scoop install uses its own data directory, separate from the MSI/EXE installers; when switching between them, migrate the `data` directory as described in the portable-mode notes below.

## Windows installers

Choose either `navop-<version>-windows-x64.msi` or `navop-<version>-windows-x64.exe` for a normal Windows installation. On 32-bit Windows, download a package whose name explicitly contains `win32`, such as `navop-<version>-win32.exe`. The EXE installer embeds and launches the same MSI installation, so both install for the current user by default, create Start menu and desktop shortcuts, register supported file associations, use the normal Windows user data directories, and support remembered master-key unlock. The default per-user location does not require administrator privileges.

## Windows no-install ZIP

The standard `navop-<version>-windows-x64.zip` (or `navop-<version>-win32.zip` for 32-bit Windows) contains only the ordinary `navop.exe`. Extract it before running. It does not install shortcuts or file associations, but it still uses the normal Windows user data directories and supports remembered master-key unlock. Do not place `navop.portable` beside the executable unless you intentionally want the portable behavior described below.

### Upgrading from the Windows ZIP in v0.10.1 or earlier

> [!IMPORTANT]
> The standard Windows ZIP in v0.10.1 and earlier already contained `navop.portable`, so users of those archives are currently running in portable mode. Upgrade by downloading the new `navop-<version>-windows-x64-portable.zip` (the 32-bit package contains `win32` in its name), backing up and preserving the complete existing `data` directory, and keeping `navop.portable` beside `navop.exe`.

Do not simply delete `navop.portable` from the old directory when changing editions. Removing the marker only makes Navop use the normal Windows user data directories; it does not copy or migrate the existing portable data.

If you extract the new standard `navop-<version>-windows-x64.zip` or `navop-<version>-win32.zip` to a new directory, or switch to the MSI/EXE installer, Navop uses the normal Windows user data directories. Existing connections, settings, and extensions may then appear missing, but the original portable data has not been deleted. The installer does not migrate that directory automatically. Keep the complete old portable directory and the master key until the migrated setup has been verified.

## Windows portable edition

### Extract and start

The official Windows `-portable.zip` is the separate portable edition and contains:

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

The portable edition stores configuration, application state, and cache under `data/config`, `data/state`, and `data/cache`. This makes the application and its data easy to move or back up together. **By default, portable mode does not persist the master key, so you must enter it on every launch.** In Settings, you may explicitly choose to store an encrypted, automatically recoverable copy at `data/state/key_storage`.

This option has a significant security risk: the file is encrypted with a key embedded in the application, not with device-bound protection. Anyone who obtains both the application and the complete `data` directory may be able to recover the master key. Enable it only if you understand and accept this risk. A forgotten master key still cannot be recovered merely by downloading or reinstalling Navop; automatic recovery is possible only when the option was enabled and a matching complete application and `data` copy was preserved.

Keep the master key separately; do not store it as plain text in the portable directory or on the same USB drive. The `data` directory may contain connection configuration, state, extensions, and caches. Do not publish it, commit it to Git, or place it in an untrusted cloud-synchronized folder.

### Update a portable installation

Portable mode does not support installing updates in the app, and automatic update checks are skipped. You can still check manually to learn that a release is available, but confirming the update opens GitHub Releases so that you can download a new Windows `-portable.zip`.

Do not overwrite an old directory that is still in use. Use this upgrade procedure:

1. Save SQL, Notes, and remote files; commit or roll back manual transactions; and wait for SFTP and remote-editing tasks to finish.
2. Quit Navop completely.
3. Back up the old portable directory, or at least its complete `data` directory.
4. Download the new Windows `-portable.zip` for the matching architecture and extract it into a new, empty directory.
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

Portable mode does not automatically register Windows associations for `.db`, `.duckdb`, or `.md`. You can still open files from inside Navop or manually select `navop.exe` with Windows Open With. A manually configured Open With path may stop working after you move the portable directory. Choose the `.msi` or EXE installer when you need stable file associations, Start menu or desktop shortcuts, in-app updates, or a master key persisted by the operating system.

Losing removable media can expose encrypted data and related metadata. A moved copy still requires the correct master key. Do not delete the original folder or backup until the new copy has been verified.

### Advanced startup options

The official Windows `-portable.zip` already includes `navop.portable`, so normal use requires no additional options. For testing or custom deployment, portable mode and the data location can also be selected explicitly:

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

## Linux portable archive

`navop-<version>-linux-x64-portable.tar.gz` (and `navop-<version>-linux-arm64-portable.tar.gz` on ARM64) **ships the very same binary** as the regular package for that architecture. The only difference is the extra `navop.portable` marker file inside the archive. When Navop finds that marker next to the executable, it relocates its data directories from the system user directories to a `data` folder beside the program, which makes the installation install-free and movable as a whole:

```bash
mkdir navop-portable
tar -xzf navop-<version>-linux-x64-portable.tar.gz -C navop-portable
cd navop-portable
./navop
```

```text
navop-portable/
├── navop
├── navop.portable
└── data/          <- created on first launch
    ├── config/
    ├── state/
    └── cache/
```

Keep the following in mind:

- **The portable archive carries no graphics dependencies.** Exactly like the regular package it still relies on the host for its graphics stack; install the "Linux graphics dependency package" below when that stack is missing, and the two combine.
- Portable mode registers no `.db`, `.duckdb`, or `.md` file associations, and it supports neither in-app installation nor automatic update checks. Use `.deb`, `.rpm`, or the AppImage when you need those.
- The portable directory must be writable. Placing it on read-only media makes Navop fail to start.
- Portable mode does not remember the master key by default, so it is requested on every launch. You can opt in to remembering it; the key stays inside the portable directory.
- Move or back up the whole directory as a unit, but quit Navop completely first and never let two instances write to the same `data`.

### Updating a portable copy

The portable archive has no in-app updater. To upgrade, extract the new `-portable.tar.gz` into a fresh directory and copy the old `data` over:

```bash
mkdir navop-portable-new
tar -xzf navop-<version>-linux-x64-portable.tar.gz -C navop-portable-new
cp -a navop-portable/data navop-portable-new/data
```

Delete the old directory only after confirming that the new one starts and that your connections and extensions are intact.

### Advanced launch options

The official `-portable.tar.gz` already contains `navop.portable`, so everyday use needs no extra arguments. For debugging or custom deployments you can also enable portable mode or point at a data directory explicitly:

```bash
# Enable portable mode for this run; the data directory sits beside navop
./navop --portable

# Use a specific data directory; the flag also enables portable paths
./navop --data-dir /data/navop

# Enable portable mode through the environment
NAVOP_PORTABLE=1 ./navop

# Use a specific data directory through the environment
NAVOP_DATA_DIR=/data/navop ./navop
```

`NAVOP_PORTABLE` accepts `1`, `true`, `yes`, or `on`. The data directory is chosen in this order: `--data-dir`, `--portable`, `NAVOP_DATA_DIR`, then `NAVOP_PORTABLE`/`navop.portable`, and finally the regular installed layout. The chosen directory must be writable, and a relative path is resolved against the working directory Navop was started from.

## Linux graphics dependency package

The Linux `navop-<version>-linux-x64.tar.gz`, `.deb`, `.rpm`, and `.AppImage` packages contain Navop only. Desktops normally already provide the graphics stack Navop needs, and nothing extra is required. Minimal containers, stripped-down server installs, WSL, and trimmed distributions can be missing the Mesa software renderer or the EGL client libraries, which shows up as an immediate exit with this in the log:

```text
Failed to create surface: Failed to create surface for any enabled backend: {}
```

In that case download the **graphics dependency package** from the same release page (`navop-<version>-linux-x64-gpu-stack.tar.gz` or `navop-<version>-linux-arm64-gpu-stack.tar.gz`), extract it, and run the installer it carries:

```bash
mkdir navop-gpu-stack
tar -xzf navop-<version>-linux-x64-gpu-stack.tar.gz -C navop-gpu-stack
sudo navop-gpu-stack/install.sh
```

The installer is additive: it only supplies libraries the host cannot already resolve. It therefore combines with every Linux package form and needs no environment variables.

- A file whose SONAME the host already resolves is skipped, so the system copy always wins. When the host already exposes a usable EGL plus a DRI driver, the entire Mesa renderer is skipped.
- `./install.sh --dry-run` prints the plan first, and `./install.sh --force` overwrites existing files of the same name.
- Files land in the directories the dynamic loader already searches (`/usr/lib64` and friends). Debian-style distributions additionally get `/etc/ld.so.conf.d/navop-gpu-stack.conf` and a refreshed loader cache.
- `sudo ./install.sh --uninstall` removes only what this installer actually wrote, recorded in `/usr/lib/navop-gpu-stack/installed.tsv`; it never touches the host's own libraries, and any file modified since installation is kept with a warning. Because uninstall needs the extracted directory, keep it if you want that option later.
- Every bundled `.so` is built to the same glibc 2.28 baseline as the Linux Navop binary.

The dependency package is architecture specific: download the one matching your Navop package. The installer refuses to run when the architectures differ. After installing, start Navop again; no other configuration is required.

## Linux Flatpak

Navop is also available from [FlatPark](https://flatpark.org/apps/dev.navop.Navop/) as a developer-endorsed community Flatpak package. Add the FlatPark remote and install Navop for the current user:

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
```

The Flatpak package runs in a sandbox, so some integrations may require additional permissions. See the [FlatPark package page](https://flatpark.org/apps/dev.navop.Navop/) for details and troubleshooting guidance.

## Complete first-launch setup

Choose a language, theme, and start page. Database, SSH, SFTP, and remote desktop features need network access; grant local-network, firewall, keychain, or file permissions only when they match the resources you intend to use. Notes folders, external editors, and custom fonts require their own filesystem access.

Create a non-production test connection before importing real credentials. Install an extension only when you need its database driver, remote desktop provider, connection importer, or ACP Agent.

## Update and roll back

For the MSI, EXE installer, and standard ZIP edition, enable automatic update checks in Settings or check manually. Close active connections, commit or roll back manual transactions, and finish SFTP transfers before applying an update. After restart, verify important connections, extensions, and keyboard shortcuts. The Windows `-portable.zip` and Linux `-portable.tar.gz` editions have no in-app update; follow their separate portable update procedures above.

If a new release is incompatible with a critical extension, back up the Navop data directory and reinstall a known stable package from Releases. Downgrading is not a substitute for backup: local configuration formats may evolve, so confirm compatibility before opening older versions.

## Open associated files

Navop can open `.db`, `.duckdb`, and `.md` through operating-system file associations. Database files create or open local SQLite/DuckDB connections; Markdown files open in Notes. If the association is missing, choose Navop with the system Open With action and optionally make it the default.

Do not open a production database file that another process is actively writing. Copy it to a safe location first. External Markdown keeps paths relative to its original folder, so moving it may break images and linked resources.

## Uninstall without losing data

Removing the application may leave local settings, encrypted connections, Notes, and extension caches in the user data directory. Preserve that directory for a reinstall. For a complete removal, export required material, stop sync, hand over team responsibilities, and then remove both the app and user data.

Reinstallation alone cannot recover a forgotten master key. If portable master-key persistence was enabled, automatic recovery also requires the matching application and complete `data` directory, including `data/state/key_storage`. Confirm master-key and team-key recovery arrangements before deleting local data.
