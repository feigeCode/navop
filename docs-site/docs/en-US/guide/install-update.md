# Install and update

Navop now ships as three apps: the full **Navop**, remote operations app **Navop Shell**, and data toolkit **Navop Data**. All three use the same version number, ship with the same version, and can be installed side by side, but their installers, app names, data directories, and keychains are separate. This page helps you pick the right package, then covers installation, portable use, updates, and uninstall on each platform.

## Choose a product

| How you work | Install | Package prefix |
| --- | --- | --- |
| Databases, servers, terminals, remote desktops, and monitoring, or you need file associations | **Navop** (full edition) | `navop-` |
| Only SSH/SFTP, terminals, port forwarding, RDP/VNC, and serial/Telnet | **Navop Shell** | `navop-shell-` |
| Only SQL, Redis, MongoDB, ER diagrams, and data transfer | **Navop Data** | `navop-data-` |

If you are unsure, install the full Navop first. You can add Navop Shell or Navop Data alongside it later without uninstalling Navop.

## Packages at a glance

The current version is **v0.23.0**. All three products follow the same naming rule:

```text
navop-<version>-<label>.<ext>
navop-shell-<version>-<label>.<ext>
navop-data-<version>-<label>.<ext>
```

`<label>` combines platform and architecture:

| Label | Meaning | Products |
| --- | --- | --- |
| `macos-arm64` | macOS on Apple Silicon | Navop / Navop Shell / Navop Data |
| `macos-x64` | macOS on Intel | Navop / Navop Shell / Navop Data |
| `windows-x64` | 64-bit Windows | Navop / Navop Shell / Navop Data |
| `win32` | 32-bit Windows | Navop only |
| `linux-x64` | 64-bit Linux | Navop / Navop Shell / Navop Data |
| `linux-arm64` | Linux on ARM64 | Navop only |

### Navop (full edition)

| Platform | Packages |
| --- | --- |
| macOS Apple Silicon | `navop-<version>-macos-arm64.dmg`, `navop-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-<version>-macos-x64.dmg`, `navop-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-<version>-windows-x64.msi`, `navop-<version>-windows-x64.exe`, `navop-<version>-windows-x64.zip`, `navop-<version>-windows-x64-portable.zip` |
| Windows 32-bit | `navop-<version>-win32.*` |
| Linux x64 | `navop-<version>-linux-x64.tar.gz`, `navop-<version>-linux-x64-portable.tar.gz`, `navop_<version>_amd64.deb`, `navop-<version>-1.x86_64.rpm`, `navop_<version>_amd64.AppImage`, `navop-<version>-linux-x64-gpu-stack.tar.gz` |
| Linux ARM64 | `navop-<version>-linux-arm64.tar.gz`, `navop-<version>-linux-arm64-portable.tar.gz` |

### Navop Shell (remote operations)

| Platform | Packages |
| --- | --- |
| macOS Apple Silicon | `navop-shell-<version>-macos-arm64.dmg`, `navop-shell-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-shell-<version>-macos-x64.dmg`, `navop-shell-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-shell-<version>-windows-x64.msi`, `navop-shell-<version>-windows-x64.exe`, `navop-shell-<version>-windows-x64.zip`, `navop-shell-<version>-windows-x64-portable.zip` |
| Linux x64 | `navop-shell-<version>-linux-x64.tar.gz`, `navop-shell-<version>-linux-x64-portable.tar.gz` |

### Navop Data (data toolkit)

| Platform | Packages |
| --- | --- |
| macOS Apple Silicon | `navop-data-<version>-macos-arm64.dmg`, `navop-data-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-data-<version>-macos-x64.dmg`, `navop-data-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-data-<version>-windows-x64.msi`, `navop-data-<version>-windows-x64.exe`, `navop-data-<version>-windows-x64.zip`, `navop-data-<version>-windows-x64-portable.zip` |
| Linux x64 | `navop-data-<version>-linux-x64.tar.gz`, `navop-data-<version>-linux-x64-portable.tar.gz` |

### Platform differences

| Target or install method | Navop | Navop Shell | Navop Data |
| --- | --- | --- | --- |
| macOS ARM64 / x64 | Yes | Yes | Yes |
| Windows x64 | Yes | Yes | Yes |
| Windows 32-bit (`win32`) | Yes | — | — |
| Linux x64 | Yes | Yes | Yes |
| Linux ARM64 | Yes | — | — |
| `.deb` / `.rpm` / `.AppImage` | Linux x64 only | — | — |
| GPU dependency bundle (`-gpu-stack`) | Linux x64 only | — | — |
| Scoop | Yes | — | — |
| Flatpak | Yes | — | — |
| File associations (`.db`/`.duckdb`/`.md`/`.cast`) | Yes | — | — |

The focused apps have no Linux ARM64, Windows 32-bit, deb/rpm/AppImage, or GPU dependency bundle. Use the full Navop on those targets.

## Download and verify

Download from one of these sources:

- Download center: [https://navop.dev/en-US/extensions](https://navop.dev/en-US/extensions)
- GitHub Releases: [https://github.com/feigeCode/navop/releases](https://github.com/feigeCode/navop/releases)
- CNB: [https://cnb.cool/navop-dev/navop/-/releases](https://cnb.cool/navop-dev/navop/-/releases)
- Cloudflare R2 mirror: `https://navop.pdyyds.cn/releases/<tag>/<file>`

Every release includes `sha256sums.txt`. Verify the download before installing:

```bash
# Linux
sha256sum -c sha256sums.txt

# macOS
shasum -a 256 -c sha256sums.txt
```

On Windows, run `certutil -hashfile <file> SHA256` and compare the result with the matching line in `sha256sums.txt`. Verify only the files you actually downloaded, and do not mix checksum files from different versions.

## First launch

Choose the interface language, theme, and default start page. SSH, SFTP, network databases, and remote desktops need access to the LAN or the internet; grant local network, firewall, or keychain permissions according to the connections you actually use. Notes folders, external editors, and custom fonts need the corresponding file system permissions.

Create a test connection without production credentials first, and confirm DNS, proxy, firewall, and certificate chain behavior. Install connection importers, remote desktop providers, database extensions, or ACP agents from the extension marketplace when you need them.

## macOS

1. Download the `.dmg` (recommended) or `.tar.gz` for your product.
2. Open the `.dmg` and drag `Navop.app`, `Navop Shell.app`, or `Navop Data.app` into `/Applications`.
3. If macOS reports that the app is damaged or cannot be opened after a network download, remove the quarantine attribute:

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

If you installed Navop Shell or Navop Data, replace `/Applications/Navop.app` with the corresponding bundle path.

For a `.tar.gz`, extract it, move the `.app` into `/Applications`, and run the command above if needed. The three apps have different bundle names, so they can all live in `/Applications`.

The first time you connect to a LAN device, macOS asks for Local Network permission. If you denied it earlier, re-enable the app under System Settings > Privacy & Security > Local Network.

## Windows

- **Installer**: run the `.msi` or `.exe` and follow the wizard. The `.exe` embeds and launches the same MSI flow. Both install into the current user profile by default, create Start menu and desktop shortcuts, use the normal Windows user data directory, and can remember the master key for automatic unlock; no administrator rights are required for the default location.
- **Standalone ZIP**: extract `navop-<version>-windows-x64.zip` and run `navop.exe`, `navop-shell.exe`, or `navop-data.exe` from the folder. It creates no shortcuts or file associations but still uses the normal Windows user data directory.
- **Portable ZIP**: extract `...-portable.zip`; data goes into the `data` folder inside the extracted directory. Do not place a marker file next to a regular ZIP unless you really intend to switch to portable mode.

If SmartScreen warns about an unknown publisher, confirm the file came from one of the official sources above and passes `sha256sums.txt` before continuing. The full Navop also has a win32 target for 32-bit Windows.

## Linux

### Portable tar.gz

All three products provide a Linux x64 `tar.gz` and `-portable.tar.gz`:

```bash
tar -xzf navop-<version>-linux-x64.tar.gz
cd navop-<version>-linux-x64
./navop
```

Navop Shell and Navop Data use `navop-shell` and `navop-data` as their executables. The `-portable.tar.gz` build uses `<install dir>/data` as its data directory and never writes to your home directory.

### deb / rpm / AppImage (full edition only)

```bash
sudo apt install ./navop_<version>_amd64.deb
sudo dnf install ./navop-<version>-1.x86_64.rpm
chmod +x navop_<version>_amd64.AppImage
./navop_<version>_amd64.AppImage
```

These three formats ship only with the full Navop Linux x64 target; Navop Shell and Navop Data do not have them.

### GPU dependency bundle (full Linux x64 only)

If the app exits at startup with `Failed to create surface: Failed to create surface for any enabled backend`, the system is missing a usable graphics backend. Download `navop-<version>-linux-x64-gpu-stack.tar.gz` and run its installer:

```bash
mkdir navop-gpu-stack
tar -xzf navop-<version>-linux-x64-gpu-stack.tar.gz -C navop-gpu-stack
sudo navop-gpu-stack/install.sh --dry-run   # review what will be installed
sudo navop-gpu-stack/install.sh
```

The installer is incremental: libraries the system already resolves are skipped, and the host's own versions win; if the system already has usable EGL and DRI drivers, the entire Mesa renderer is skipped. `install.sh` supports `--dry-run`, `--force`, and `--uninstall`, and records installed files in `/usr/lib/navop-gpu-stack/installed.tsv`; uninstall removes only files the script actually wrote. The bundle targets glibc 2.28 and applies only to the full Navop Linux x64 build.

## Scoop (full edition only)

Scoop distributes only the full Navop, as a portable install with `data` managed by Scoop:

```powershell
scoop bucket add extras
scoop install navop
```

Upgrade with `scoop update navop`. Scoop uses a different data directory from the MSI/EXE installer, so migrate `data` when switching install methods. Navop Shell and Navop Data must be installed from an installer, ZIP, or portable package.

## Flatpak (full edition only)

Flatpak also covers only the full Navop:

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
flatpak run dev.navop.Navop
```

The package is `dev.navop.Navop`; see the [FlatPark page](https://flatpark.org/apps/dev.navop.Navop/). Flatpak runs in a sandbox, so SSH, database, SFTP, or remote desktop integration may need extra permissions. Navop Shell and Navop Data have no Flatpak package.

## Portable builds

All three products support portable mode, each with its own marker file:

| Product | Marker file | Executable |
| --- | --- | --- |
| Navop | `navop.portable` | `navop` / `navop.exe` |
| Navop Shell | `navop-shell.portable` | `navop-shell` / `navop-shell.exe` |
| Navop Data | `navop-data.portable` | `navop-data` / `navop-data.exe` |

Place the matching marker file next to the executable (it may be empty) and the app writes config, state, and cache to `<install dir>/data`. You can also use environment variables or command-line flags instead:

```bash
NAVOP_PORTABLE=1 ./navop
NAVOP_DATA_DIR=/path/to/data ./navop
./navop --portable
./navop --data-dir /path/to/data
```

`NAVOP_PORTABLE` accepts `1`, `true`, `yes`, or `on`. Data directory precedence is `--data-dir` > `--portable` > `NAVOP_DATA_DIR` > `NAVOP_PORTABLE` or marker file > normal user directory. The `NAVOP_*` variables are shared by all three products, but the marker file name must match the product.

After the first launch, the layout looks like this:

```text
<install dir>/
├── navop            # or navop-shell / navop-data
├── navop.portable   # or navop-shell.portable / navop-data.portable
└── data/
    ├── config/
    ├── state/
    └── cache/
```

The portable directory must be writable; a read-only location prevents startup. Portable mode does not persist the master key by default, so you must enter it on every launch. You can explicitly save an encrypted copy under `data/state/key_storage`, which weakens protection.

Portable builds do not register file associations and do not support in-app or automatic updates. To upgrade, quit the app, extract the new version into a new directory, copy the old `data` directory across, and confirm connections, extensions, and Notes before deleting the old directory. Back up `data` first, and never let two instances write to the same data.

## Data directories, keychain, and side-by-side installs

Each product has its own directory names and system identities:

| Product | Directory name | Keychain service | Bundle id |
| --- | --- | --- | --- |
| Navop | `navop` | `com.navop.desktop` | `com.onetcli.app` |
| Navop Shell | `navop-shell` | `com.navop.desktop.shell` | `com.navop.shell` |
| Navop Data | `navop-data` | `com.navop.desktop.data` | `com.navop.data` |

Locations by platform:

| Content | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Config | `~/.config/<dir>` | `~/.config/<dir>` | `%APPDATA%\<dir>` |
| Data | `~/Library/Application Support/<dir>` | `~/.local/share/<dir>` | `%APPDATA%\<dir>` |
| Cache | `~/Library/Caches/<dir>` | `~/.cache/<dir>` | `%APPDATA%\<dir>` |

For example, Navop Shell's Linux data directory is `~/.local/share/navop-shell`, and Navop Data's Windows config directory is `%APPDATA%\navop-data`. Because the directory names differ, the three apps never read or write the same connections and settings, and they can be installed and run at the same time.

Credentials live in the system keychain or credential store under the service names above. Uninstalling an app usually leaves keychain entries and data directories behind; clean them up separately as described below.

## File associations and command line

Only the full Navop registers `.db`, `.duckdb`, `.md`, and `.cast` file associations. Double-clicking those files opens Navop; Navop Shell and Navop Data register nothing and do not take over existing defaults. Portable mode also skips system associations; use an installed build if you need stable associations.

You can pass files on the command line:

```bash
navop ./data.db
navop ./notes.md
```

Use `navop.exe` on Windows. If the system still points associations at an old version, reinstall the full Navop from its installer, or reselect it under the system default apps settings.

## In-app updates and rollback

Each product checks its own update manifest:

| Product | Update manifest |
| --- | --- |
| Navop | `https://navop.pdyyds.cn/updates/latest.json` |
| Navop Shell | `https://navop.pdyyds.cn/updates/navop-shell/shell/latest.json` |
| Navop Data | `https://navop.pdyyds.cn/updates/navop-data/data/latest.json` |

An in-app update replaces only the current product; it never turns Navop Shell into the full Navop and never touches the other side-by-side installs. To switch products, download and install the other product separately.

Portable builds (including Windows `-portable.zip`, Linux `-portable.tar.gz`, and Scoop installs) have no in-app update and no automatic update check; upgrade them manually as described under Portable builds. Installed builds, regular ZIPs, and other non-portable installs can enable automatic checks or check manually in Settings.

To roll back, download the older installer and install it over the current version; connections and settings usually remain in the data directory. If the newer version migrated local data structures, however, the older build may not be able to read them, so back up the product's data directory before rolling back.

## Uninstall and keep data

Uninstalling an app does not delete connections, settings, or history automatically. Quit the app first, then remove that product's directories if you want a clean removal.

### macOS

Move `Navop.app`, `Navop Shell.app`, or `Navop Data.app` to the Trash. Then remove, as needed:

```text
~/Library/Application Support/<dir>
~/Library/Caches/<dir>
~/.config/<dir>
```

### Windows

Uninstall the product under Settings > Apps. The installer removes program files but keeps `%APPDATA%\<dir>`; for a portable build, just delete the extracted folder. To clean up completely, also delete `%APPDATA%\<dir>`.

### Linux

Delete the extracted folder for a portable build. For deb/rpm installs, remove the package:

```bash
sudo apt remove navop
sudo dnf remove navop
```

Delete the AppImage file directly. Then remove `~/.local/share/<dir>`, `~/.config/<dir>`, and `~/.cache/<dir>` if needed.

To reset one product without affecting the other two, delete only that product's directories. Never merge or copy directories between `navop`, `navop-shell`, and `navop-data`.

## Post-install checklist

- [ ] The app name and icon match the intended product: Navop, Navop Shell, or Navop Data.
- [ ] The About page shows version v0.23.0.
- [ ] The data directory was created under the matching product name, not one of the other two.
- [ ] File associations exist only for the full Navop (`.db`/`.duckdb`/`.md`/`.cast`).
- [ ] Public MCP discovery uses the matching name: `navop`, `navop-shell`, or `navop-data`.
- [ ] With all three installed, each app starts independently and their connection lists stay separate.
- [ ] Update checks read the current product's own manifest.
