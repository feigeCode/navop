# Extension marketplace and local extensions

Extensions add database drivers, middleware, native resource workbenches, ACP Agents, connection importers, remote desktop providers, language packs, and related capabilities. Extension code runs locally, so review source, permissions, platform, and compatibility as carefully as other desktop software.

## Understand extension types

Database drivers add connection types. ACP Agent extensions connect external agents. Import extensions read other applications. Remote desktop providers enable RDP/VNC. Language extensions add UI locales. Native resource workbench extensions (such as Docker) let the host render collections, detail pages, and operations. A composite extension bundles several panels into one package, and its panels appear in the terminal, database, or note sidebar. An "updates available" filter in the marketplace shows only extensions with newer releases.

A product name in a compatibility list does not mean its driver is installed. Check the marketplace entry, current platform, and Navop version first.

## First-party extension repository

First-party extensions are built and published independently from the [navop-extensions](https://github.com/feigeCode/navop-extensions) repository; official marketplace entries come from that repository. The current catalog, grouped by kind:

- **Database drivers**: TDengine, DuckDB, Redis, MongoDB (4.2+ / 3.6 / 3.2–3.4 tiers), Dameng DM, KingbaseES, GBase 8s, OceanBase, openGauss, Apache IoTDB, Oscar, and a pure-Go Oracle driver that needs no Oracle Instant Client.
- **Middleware**: MQTT (subscribe, messages, and publish views, with SSH tunneling), RocketMQ (topics, consumer groups, and messages), and Elasticsearch 9 (cluster overview, indices, and document search).
- **Native resource workbenches**: Docker (engine overview, container and image management, logs, processes, filesystem changes, and exec terminals), where the host renders collections, detail pages, and operations natively; plus composite panels such as Server Logs (a bottom full-width panel that tails or greps remote log files per connection) and Server Containers (right-side tabs for processes, containers, images, volumes, networks, Compose, and Kubernetes).
- **Remote desktop providers**: RDP and VNC.
- **ACP Agents**: Codex, Claude Code, and OpenCode.
- **Connection importers**: SecureCRT, Xshell, WindTerm, OpenSSH config, Navicat, DBeaver, JetBrains DataGrip, MongoDB Compass, Redis Desktop, and TablePlus.
- **External editors**: Zed, Notepad++, and Notepad--.
- **Notes rendering and export**: Mermaid diagrams, LaTeX math rendering, and HTML / PDF / Word export.
- **Language packs**: a Tree-sitter based syntax-highlighting bundle for common languages.

Platform support, minimum versions, and method contracts are defined by each marketplace entry and the repository docs; drivers targeting EOL servers (such as legacy MongoDB) intentionally omit newer features such as SRV and TLS.

## Install, update, and reload

Review publisher, description, permissions, and release notes before installation. Reload extensions or restart when requested. Navop refreshes capabilities by extension kind and keeps language parsers lazy, so an unrelated extension change does not compile every language WASM parser. Save work before updating a driver or provider because connection behavior may change.

On startup, Navop checks the marketplace in the background and compares it with installed versions. When updates are found, a notification links straight to the marketplace; each batch is announced only once. The "updates available" filter in the marketplace page shows only extensions that can be updated.

For failures, disable and re-enable or reload the extension, then inspect logs and versions. Do not update the driver currently handling a production task.

## Dock extension panels

Manifests can declare where an extension panel docks: left, right, or bottom sidebar, defaulting to the right. A manual drag takes precedence, so a panel stays where you put it. The database bottom panel spans the full content width and its height is draggable. The "+" at the sidebar opens the extension marketplace filtered to composite extensions.

## Plugin AI tools

Plugins built on the nsh runtime can declare `contributes.tools` and expose tools to the built-in Agent and external MCP clients. Tools are off by default: enable them per extension, disable individual tools, and set timeouts, result limits, and risk confirmation. A plugin tab can also show an AI sidebar on the right, collapsed by default and resizable.

## Use local and offline packages

Development and restricted-network environments can install a local extension or offline package. Verify origin and integrity because local packages may not receive marketplace distribution checks. Test with non-production resources and a separate workspace.

Keep manifests, binaries, and resources together. Prepare dependencies for an offline machine, and retest compatibility after upgrading Navop.

## Review permissions and limits

An extension may access network, files, connection metadata, or rendering resources according to its manifest. Grant only what is required. Unknown publishers should not receive production credentials. Importers may need additional filesystem permission; missing passwords after denial are expected.

WASM and other controlled runtimes have resource and timeout limits. When an extension task fails, inspect its input, permissions, logs, and compatible versions instead of removing every limit.

## Uninstall and report issues

Close dependent connections, ACP sessions, and remote desktops before uninstalling. Removal runs in the background and reports its current state; do not start another install, reload, or removal until it finishes. Removal usually deletes the capability, not the remote data; saved connections may work again after reinstalling a compatible extension.

Reports should include Navop version, extension and version, platform, steps, and redacted logs. Never attach passwords, private keys, API keys, master keys, or complete import files.
