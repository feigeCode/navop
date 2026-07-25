# Extension marketplace and local extensions

Extensions add database drivers, ACP Agents, connection importers, remote desktop providers, language packs, and related capabilities. Extension code runs locally, so review source, permissions, platform, and compatibility as carefully as other desktop software.

## Understand extension types

Database drivers add connection types. ACP Agent extensions connect external agents. Import extensions read other applications. Remote desktop providers enable RDP/VNC. Language extensions add UI locales.

A product name in a compatibility list does not mean its driver is installed. Check the marketplace entry, current platform, and Navop version first.

## Install, update, and reload

Review publisher, description, permissions, and release notes before installation. Reload extensions or restart when requested. Navop refreshes capabilities by extension kind and keeps language parsers lazy, so an unrelated extension change does not compile every language WASM parser. Save work before updating a driver or provider because connection behavior may change.

For failures, disable and re-enable or reload the extension, then inspect logs and versions. Do not update the driver currently handling a production task.

## Use local and offline packages

Development and restricted-network environments can install a local extension or offline package. Verify origin and integrity because local packages may not receive marketplace distribution checks. Test with non-production resources and a separate workspace.

Keep manifests, binaries, and resources together. Prepare dependencies for an offline machine, and retest compatibility after upgrading Navop.

## Review permissions and limits

An extension may access network, files, connection metadata, or rendering resources according to its manifest. Grant only what is required. Unknown publishers should not receive production credentials. Importers may need additional filesystem permission; missing passwords after denial are expected.

WASM and other controlled runtimes have resource and timeout limits. When an extension task fails, inspect its input, permissions, logs, and compatible versions instead of removing every limit.

## Uninstall and report issues

Close dependent connections, ACP sessions, and remote desktops before uninstalling. Removal runs in the background and reports its current state; do not start another install, reload, or removal until it finishes. Removal usually deletes the capability, not the remote data; saved connections may work again after reinstalling a compatible extension.

Reports should include Navop version, extension and version, platform, steps, and redacted logs. Never attach passwords, private keys, API keys, master keys, or complete import files.
