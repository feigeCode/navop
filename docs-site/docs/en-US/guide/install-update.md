# Install and update

Navop is a desktop workspace built natively with Rust and GPUI. Installers cover macOS, Windows, and Linux. This page covers package selection, download verification, first launch, portable use, in-app updates, and how to keep or remove data when uninstalling.

The same source also builds two narrower editions: **Navop Shell** (SSH, terminals, and remote desktop) and **Navop Data** (databases and data browsing). They use the `navop-shell-` / `navop-data-` prefixes and fewer platform targets. Unless you specifically need only that subset, install Navop. The steps below apply to the editions too; replace the app and executable names.

## Packages

Package naming:

```text
navop-<version>-<label>.<ext>
```

`<label>` combines platform and architecture:

| Label | Meaning |
| --- | --- |
| `macos-arm64` | macOS on Apple Silicon |
| `macos-x64` | macOS on Intel |
| `windows-x64` | 64-bit Windows |
| `win32` | 32-bit Windows |
| `linux-x64` | 64-bit Linux |
| `linux-arm64` | Linux on ARM64 |

| Platform | Packages |
| --- | --- |
| macOS Apple Silicon | `navop-<version>-macos-arm64.dmg`, `navop-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-<version>-macos-x64.dmg`, `navop-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-<version>-windows-x64.msi`, `navop-<version>-windows-x64.exe`, `navop-<version>-windows-x64.zip`, `navop-<version>-windows-x64-portable.zip` |
| Windows 32-bit | `navop-<version>-win32.*` |
| Linux x64 | `navop-<version>-linux-x64.tar.gz`, `navop-<version>-linux-x64-portable.tar.gz`, `navop_<version>_amd64.deb`, `navop-<version>-1.x86_64.rpm`, `navop_<version>_amd64.AppImage`, `navop-<version>-linux-x64-gpu-stack.tar.gz` |
| Linux ARM64 | `navop-<version>-linux-arm64.tar.gz`, `navop-<version>-linux-arm64-portable.tar.gz`, `navop_<version>_arm64.deb`, `navop-<version>-1.aarch64.rpm`, `navop_<version>_arm64.AppImage`, `navop-<version>-linux-arm64-gpu-stack.tar.gz` |

Scoop and Flatpak are also available, described below. The current version number is shown on the [Download Center](https://navop.dev/en-US/extensions) or the Releases page; it is not hard-coded here.

## Download and verify

Preferred sources:

- Download Center: [https://navop.dev/en-US/extensions](https://navop.dev/en-US/extensions)
- GitHub Releases: [https://github.com/feigeCode/navop/releases](https://github.com/feigeCode/navop/releases)
- CNB: [https://cnb.cool/navop-dev/navop/-/releases](https://cnb.cool/navop-dev/navop/-/releases)
- Cloudflare R2 mirror: `https://navop.pdyyds.cn/releases/<tag>/<file>`

Every release includes `sha256sums.txt`. Verify before installing:

```bash
# Linux
sha256sum -c sha256sums.txt

# macOS
shasum -a 256 -c sha256sums.txt
```

On Windows, compute `certutil -hashfile <file> SHA256` and compare it with the matching line in `sha256sums.txt`. Only verify the files you actually downloaded; do not mix checksum files from different versions.

## First launch

Choose the interface language, theme, and default start page. SSH, SFTP, network databases, and remote desktop need LAN or internet access; grant local-network, firewall, and keychain permissions according to the actual connection scope. The Notes directory, external editors, and custom fonts need the corresponding filesystem permissions.

Start with a test connection that contains no production credentials, and confirm DNS, proxy, firewall, and certificate chain behavior. Install connection importers, remote desktop providers, database extensions, or ACP agents from the extension marketplace when needed.

## macOS

1. Download the `.dmg` (recommended) or `.tar.gz`.
2. Open the `.dmg` and drag `Navop.app` into `/Applications`.
3. If macOS says the app is damaged or cannot be opened after a network download, remove the quarantine attribute:

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

For a `.tar.gz`, extract it, move the `.app` to `/Applications`, and run the quarantine command if needed.

The first time you connect to a LAN device, macOS asks for Local Network permission. If you denied it earlier, allow Navop again under **System Settings > Privacy & Security > Local Network**.

## Windows

- **Installer:** run the `.msi` or `navop-<version>-windows-x64.exe`. The executable embeds and launches the same MSI flow. Both install per-user by default, create Start Menu and desktop shortcuts, use the normal Windows user data directory, and can remember the master key for automatic unlock; the default location does not require administrator rights.
- **Portable ZIP:** extract `navop-<version>-windows-x64.zip` and run `navop.exe`. It creates no shortcuts or file associations but still uses the normal Windows user data directory.
- **Portable-mode ZIP:** extract `...-portable.zip`; data goes into `data` next to the executable. Use this on a USB drive or in a controlled directory. Do not place the marker file next to a regular ZIP unless you intend to switch to portable mode.

If SmartScreen reports an unknown publisher, confirm the file came from an official source above and passed `sha256sums.txt` before continuing. 32-bit Windows uses the `win32` target.

## Linux

### Portable tar.gz

```bash
tar -xzf navop-<version>-linux-x64.tar.gz
cd navop-<version>-linux-x64
./navop
```

`-portable.tar.gz` uses `<install dir>/data` as its data directory and writes nothing to your home directory.

### deb / rpm / AppImage

```bash
# Linux x64
sudo apt install ./navop_<version>_amd64.deb
sudo dnf install ./navop-<version>-1.x86_64.rpm
chmod +x navop_<version>_amd64.AppImage
./navop_<version>_amd64.AppImage

# Linux ARM64
sudo apt install ./navop_<version>_arm64.deb
sudo dnf install ./navop-<version>-1.aarch64.rpm
chmod +x navop_<version>_arm64.AppImage
./navop_<version>_arm64.AppImage
```

These three package formats ship for both the Linux x64 and ARM64 targets; the architecture suffix differs: x64 uses `amd64` (deb / AppImage) and `x86_64` (rpm), ARM64 uses `arm64` (deb / AppImage) and `aarch64` (rpm).

### GPU stack package

If startup fails with `Failed to create surface: Failed to create surface for any enabled backend`, the system is missing usable graphics backend libraries. GPU dependency packages exist for Linux x64 and arm64: `navop-<version>-linux-x64-gpu-stack.tar.gz` and `navop-<version>-linux-arm64-gpu-stack.tar.gz`. Extract the package and run its `install.sh`:

```bash
mkdir navop-gpu-stack
tar -xzf navop-<version>-linux-x64-gpu-stack.tar.gz -C navop-gpu-stack
sudo navop-gpu-stack/install.sh --dry-run   # inspect the dependencies first
sudo navop-gpu-stack/install.sh
```

The script is incremental: SONAMEs the system can already resolve are skipped and host versions win; when usable EGL and DRI drivers exist, the whole Mesa renderer is skipped. `install.sh` supports `--dry-run`, `--force`, and `--uninstall`; it records installed files in `/usr/lib/navop-gpu-stack/installed.tsv` and uninstalls only what it wrote. The package targets glibc 2.28 as its baseline.

## Scoop

Scoop distributes the portable form, with `data` managed by Scoop:

```powershell
scoop bucket add extras
scoop install navop
```

Upgrade with `scoop update navop`. Scoop and MSI/EXE installs use different data directories; migrate `data` when switching. See the portable section for details.

## Flatpak

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
flatpak run dev.navop.Navop
```

The package is `dev.navop.Navop`; see [https://flatpark.org/apps/dev.navop.Navop/](https://flatpark.org/apps/dev.navop.Navop/). Flatpak runs in a sandbox, so SSH, database, SFTP, or remote desktop integration may need extra permissions.

## Portable mode

Place a marker file named `navop.portable` next to the executable (contents may be empty), and the app writes configuration, state, and cache to `<install dir>/data`. You can also use environment variables or command-line flags:

```bash
NAVOP_PORTABLE=1 ./navop
NAVOP_DATA_DIR=/path/to/data ./navop
./navop --portable
./navop --data-dir /path/to/data
```

`NAVOP_PORTABLE` accepts `1`, `true`, `yes`, and `on`. Data directory precedence: `--data-dir` > `--portable` > `NAVOP_DATA_DIR` > `NAVOP_PORTABLE` or marker file > normal user directory.

After first launch the portable layout looks like this:

```text
<install dir>/
├── navop
├── navop.portable
└── data/
    ├── config/
    ├── state/
    └── cache/
```

The directory must be writable; a read-only location causes startup to fail. Portable mode does not persist the master key by default, so it is requested on every launch. You can explicitly save an encrypted copy under `data/state/key_storage`, at the cost of weaker protection.

Portable builds register no file associations and support neither in-app updates nor automatic update checks. To upgrade, quit the app completely, extract the new version into a new directory, copy the old `data` directory over, confirm connections, extensions, and Notes, then remove the old directory. Back up `data` first and make sure two instances never write to the same data directory.

## Data directory and keychain

| Content | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Config | `~/.config/navop` | `~/.config/navop` | `%APPDATA%\navop` |
| Data | `~/Library/Application Support/navop` | `~/.local/share/navop` | `%APPDATA%\navop` |
| Cache | `~/Library/Caches/navop` | `~/.cache/navop` | `%APPDATA%\navop` |

Credentials live in the system keychain or credential store under the service name `com.navop.desktop`; the bundle id is `com.onetcli.app`. Uninstalling usually leaves keychain entries and data directories behind; clean them up separately using the steps below.

If Navop Shell or Navop Data is also installed, each uses its own `navop-shell` / `navop-data` directory and keychain entry and never shares connection ciphertext with Navop. Do not merge or copy these directories into one another.

## File associations and command line

Navop registers `.db`, `.duckdb`, `.md`, and `.cast` file associations. Double-clicking these files opens them in Navop. Portable mode registers no system associations; use an installed build when you need stable associations.

You can pass files directly on the command line:

```bash
navop ./data.db
navop ./notes.md
```

On Windows use `navop.exe`. If the system still points associations at an older version, reinstall from the package or reselect Navop under **Default apps**.

## In-app updates and rollback

Installed builds, regular ZIPs, and other conventional installs can check for updates automatically or manually in Settings. Navop reads `https://navop.pdyyds.cn/updates/latest.json`. If a narrower edition is installed alongside it, that edition reads its own manifest and is unaffected.

Portable builds (including the Windows `-portable.zip`, Linux `-portable.tar.gz`, and Scoop install) have no in-app updates and never check automatically; upgrade them manually as described in the portable section.

To roll back, download the older installer and install it over the current version. Connections and settings usually stay in the data directory, but if the newer version migrated local data structures, the older build may not read them; back up the data directory before rolling back.

## Uninstall and keep data

Uninstalling does not delete connections, settings, or history. To remove everything, quit the app first, then delete the directories.

### macOS

Move `Navop.app` to the Trash, then remove as needed:

```text
~/Library/Application Support/navop
~/Library/Caches/navop
~/.config/navop
```

### Windows

Uninstall Navop under **Settings > Apps**. The installer removes program files but keeps `%APPDATA%\navop`; for a portable build, delete the extracted directory. Remove `%APPDATA%\navop` separately for a full cleanup.

### Linux

For a portable build, delete the extracted directory. For deb/rpm installs, use the package manager:

```bash
sudo apt remove navop
sudo dnf remove navop
```

Delete the AppImage file directly. Then remove `~/.local/share/navop`, `~/.config/navop`, and `~/.cache/navop` as needed.

## Post-install checklist

- [ ] The app name and icon are Navop.
- [ ] The version in **About** matches the download page.
- [ ] The data directory is `navop`, with no other directory mixed in.
- [ ] File associations include `.db`, `.duckdb`, `.md`, and `.cast`.
- [ ] The Public MCP discovery name is `navop`.
- [ ] Update checks read Navop's manifest.
- [ ] You can create a test connection and **Test connection** succeeds.
