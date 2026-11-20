# x11_component

内置与本机 X11 服务的解析、签名安装和单实例进程管理。本期内置范围仅为 Windows x86_64/aarch64；macOS 保持 XQuartz 外部服务。

## 应用集成

- 启动时调用 `set_data_dir`，传入已经限定产品的数据目录；默认值为 `dirs::data_dir()/navop`，组件根目录只追加 `components/x11`。
- 调用 `set_navop_version(env!("CARGO_PKG_VERSION").parse()?)`，清单的 `pinned_for_navop` 校验应用版本，而不是 yserver 版本。
- 使用 `resolve_server(mode)` 获取共享代理；设置变化时调用 `set_configured_mode`。
- `NAVOP_X11_YSERVER_PATH` 仅供开发/验收注入手动二进制，仍执行相同的 Vulkan、xauth、握手和进程生命周期检查。上游原版会监听所有网卡，因此 TCP 手动入口会 fail-closed。

## 内嵌构建

正式 Windows full 和 standalone-shell 的 x86_64 构建把完整签名 zip 编进 `navop.exe`。MSI、Burn、portable 和应用自更新都随 exe 携带组件，不依赖旁置资源。

```bash
python3 script/prepare-windows-x11-component.py \
  --target windows-x86_64 --output "$PWD/target/navop-x11-component.zip"
NAVOP_X11_BUNDLED_ARCHIVE="$PWD/target/navop-x11-component.zip" \
  cargo build --release -p navop
```

准备脚本只接受仓库内固定清单的固定版本和 URL，在构建前执行 SHA-256 与 Ed25519 验签；`build.rs` 不联网。支持的目标为 `windows-x86_64`、`windows-aarch64`，路径必须是绝对路径。当前 Navop 没有 Windows aarch64 产品 target；Win32、standalone-data 以及未显式配置内嵌包的开发构建使用空占位，不能误装 x64 包。

运行时安装与“重新下载/修复”优先复用内嵌 zip，完整走灰度（显式 Embedded 除外）、摘要、签名、安全解压、staging 和原子提交，无需创建 HTTP client。内嵌包损坏或与编译目标不符时直接失败，不回退网络。普通开发构建未配置内嵌包时，仍可从同一固定 Release 下载，便于运行 mock HTTP 测试。

## 发布与信任根

独立组件公钥固定在 `resources/windows/x11-component/public-key.txt`，也可用 `NAVOP_X11_PUBLIC_KEY_BASE64` 在编译期覆盖。它不得复用应用更新公钥。私钥只保存在签名仓库的 GitHub Actions secret，不进入 Navop 仓库。

`DEFAULT_MANIFEST_URL` 固定到 [feigeCode/yserver `navop-v1.9.0-1`](https://github.com/feigeCode/yserver/releases/tag/navop-v1.9.0-1)，不发现 tag、不追踪 latest。该 fork 基于 yserver v1.9.0，增加 `--listen 127.0.0.1`，由 GitHub Actions 构建并签名；源码提交为 `3b3c7f66fb9d9c80752af36b4948fad0b8760671`。

清单必须显式给出 `format`（`binary` 或 `zip`）和安全的相对 `executable_path`。当前 zip 完整保留 `yserver.exe`、`xauth.exe`、`xkb/` 和 licenses；同一组件版本不得替换为不同摘要，更新时必须提升组件版本并同步固定清单、URL 和签名。

## 测试

```bash
cargo fmt -p x11_component
cargo test -p x11_component
cargo clippy -p x11_component --tests -- -D warnings
```

测试使用独立临时数据目录、临时签名密钥和本地 mock HTTP，覆盖无网络、SHA256/签名错误、安全 zip、原子安装、内嵌离线安装/修复、重复/强制下载、rollout、重启窗口、永久禁用原因和指标；不访问真实发布服务，也不依赖开发者机器已有 yserver。CI 另用真实固定 zip 编译并校验内嵌产物，不能给下载 mock 测试全局注入内嵌包。macOS 内置路径始终返回 `UnsupportedPlatform`，External 探测仍可工作。
