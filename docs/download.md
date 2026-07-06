---
title: 下载
description: 从 GitHub Releases 下载 OnetCli，获取纯 Rust 原生桌面版一体化运维工作台
---

# 下载

OnetCli 当前下载入口统一托管在 GitHub Releases。推荐先下载桌面版，启动后创建第一个数据库连接或 SSH 主机，快速体验一体化工作台。

[前往 GitHub Releases](https://github.com/feigeCode/onetcli/releases)

## 支持平台

| 平台 | 架构 | 渲染 |
| --- | --- | --- |
| macOS | Apple Silicon、Intel | Metal |
| Linux | x86_64、ARM64 | Vulkan |
| Windows | x86_64 | 原生桌面窗口 |

> 实际可下载包以 GitHub Releases 中当前发布的文件为准。

## Linux 包格式

Linux x86_64 发布包包含通用归档和发行版安装包：

- `onetcli-x86_64-unknown-linux-gnu.tar.gz`：应用内自动更新和通用手动安装使用。
- `onetcli_<version>_amd64.deb`：Ubuntu、Debian、Linux Mint 等 Debian 系发行版使用。
- `onetcli-<version>-1.x86_64.rpm`：Fedora、RHEL、CentOS、openSUSE 等 RPM 系发行版使用。
- `onetcli_<version>_amd64.AppImage`：不确定发行版或偏好便携运行时使用。

Linux ARM64 当前提供 `onetcli-aarch64-unknown-linux-gnu.tar.gz`。

## 下载后可以先体验什么

1. 打开 OnetCli，创建一个数据库连接。
2. 添加 SSH 主机，进入远程终端。
3. 打开 SFTP 文件管理，查看远程目录或传输文件。
4. 尝试 Redis Key 浏览或 MongoDB 文档浏览。
5. 在 SQL 或终端场景中使用 AI 辅助解释和生成下一步操作。

## macOS 安装提示

如果 macOS 在首次打开 DMG 安装后的应用时提示无法验证开发者，可以参考仓库 README 中的处理方式：

```bash
sudo xattr -rd com.apple.quarantine /Applications/OnetCli.app
```

## 从源码运行

如果你希望从源码运行，需要准备 Rust 环境和平台相关依赖。

```bash
cargo run -p main
```

macOS / Linux 的依赖初始化可以参考：

```bash
./script/bootstrap
```

Windows PowerShell 可以参考：

```powershell
.\script\install-window.ps1
```

## 相关入口

- [快速开始](/guide)
- [功能总览](/features)
- [更新日志](/changelog)
- [GitHub 仓库](https://github.com/feigeCode/onetcli)
