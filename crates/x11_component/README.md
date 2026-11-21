# x11_component

内置与本机 X11 服务的解析、签名安装和单实例进程管理。内置范围为 Windows x86_64/aarch64 和 Apple Silicon macOS；外置模式继续使用本机 X server，macOS 上为 XQuartz。

## 应用集成

- 启动时调用 `set_data_dir`，传入已经限定产品的数据目录；默认值为 `dirs::data_dir()/navop`，组件根目录只追加 `components/x11`。
- 调用 `set_navop_version(env!("CARGO_PKG_VERSION").parse()?)`，清单的 `pinned_for_navop` 校验应用版本，而不是 yserver 版本。
- 使用 `resolve_server(mode)` 获取共享代理；设置变化时调用 `set_configured_mode`。
- `Auto` 优先选择通过真实 X11 setup 握手的外置服务，失败后尝试内置；`External` 只使用外置服务，不会静默切换；`Embedded` 只使用内置服务。
- `NAVOP_X11_YSERVER_PATH` 仅供开发/验收注入手动二进制，仍执行相同的 xauth、握手和进程生命周期检查。上游原版会监听所有网卡，因此 TCP 手动入口会 fail-closed；macOS 手动入口使用 Unix socket。

## 内嵌构建

正式 Windows full/standalone-shell x86_64 和 macOS full/standalone-shell arm64 构建把完整签名 zip 编进主可执行文件。安装包、portable 和应用自更新都随主程序携带组件，不依赖旁置资源。

```bash
python3 script/prepare-x11-component.py \
  --target macos-aarch64 --output "$PWD/target/navop-x11-component.zip"
NAVOP_X11_BUNDLED_ARCHIVE="$PWD/target/navop-x11-component.zip" \
  cargo build --release -p navop
```

准备脚本只接受仓库内固定清单的版本和构建脚本中的固定 URL，在构建前执行 SHA-256、Ed25519 验签和包结构检查；`build.rs` 不联网。支持的目标为 `windows-x86_64`、`windows-aarch64`、`macos-aarch64`，路径必须是绝对路径。当前 Navop 没有 Windows aarch64 产品 target；Win32、Intel macOS、standalone-data 以及未显式配置内嵌包的开发构建使用空占位，不能误装其他架构的包。

运行时安装与“修复/重新安装”只读取内嵌 zip，完整走灰度（显式 Embedded 除外）、摘要、签名、安全解压、staging 和原子提交。crate 不包含 HTTP client、远程清单或下载重试逻辑；没有内嵌包、包损坏或与编译目标不符时直接失败，不存在网络回退。

## 发布与信任根

独立组件公钥固定在 `resources/windows/x11-component/public-key.txt`，也可用 `NAVOP_X11_PUBLIC_KEY_BASE64` 在编译期覆盖。它不得复用应用更新公钥。私钥只保存在签名仓库的 GitHub Actions secret，不进入 Navop 仓库。

构建脚本的固定地址指向 [feigeCode/yserver `navop-v1.9.0-1`](https://github.com/feigeCode/yserver/releases/tag/navop-v1.9.0-1)，不发现 tag、不追踪 latest。Windows 使用 `1.9.0-navop.1`，源码提交为 `3b3c7f66fb9d9c80752af36b4948fad0b8760671`；macOS 使用 `1.9.0-navop.2`，源码提交为 `465c3db5`。macOS zip 包含 19 个非系统动态库、XKB、KosmicKrisp/Vulkan loader、xauth 和许可证，无需用户安装 XQuartz 或 Homebrew。

运行时清单必须显式给出 `format`（`binary` 或 `zip`）和安全的相对 `executable_path`，不接受 URL 等下载元数据。签名原始字节是完整 zip。同一组件版本不得替换为不同摘要，更新时必须提升组件版本并同步固定清单、构建期 URL 和签名。

## 运行条件

- Windows 使用仅监听 `127.0.0.1` 的 TCP endpoint；macOS 使用随机 Unix socket，并传递 `--rootless -nolisten tcp`。
- macOS 安装时保留 ICD 中的 `@@YSERVER_ROOT@@` 占位符，启动前按已提交的 executable 目录重定位，不能把 staging 绝对路径写入最终安装目录。
- macOS 启动后最多等待 15 秒，每 100ms 重试真实 X11 setup，同时检查子进程是否提前退出；首次 Vulkan/Metal 初始化可能耗时数秒，不能在 socket 尚未出现时立即杀进程。Windows 保留原有单次 2 秒握手预算，避免 macOS 冷启动条件改变其他平台的失败时延。
- KosmicKrisp 的真实渲染环境要求 Apple Silicon 与 macOS 26+；不支持此条件的用户可选择 `External`，使用 XQuartz。Intel Mac 没有内置 artifact。

## 测试

```bash
cargo fmt -p x11_component
cargo test -p x11_component
cargo clippy -p x11_component --tests -- -D warnings
```

测试使用独立临时数据目录、临时签名密钥和内存 zip，覆盖 SHA256/签名错误、安全 zip、原子安装、离线安装/修复、重复/强制安装、rollout、重启窗口、永久禁用原因和指标；普通运行时测试不访问发布服务。CI 使用真实固定 zip 编译并校验内嵌产物，macOS arm64 还运行真实安装、启动和 X11 setup 握手。外置 XQuartz 探测测试在本机有 XQuartz 时单独运行，不依赖其存在作为所有测试的前提。
