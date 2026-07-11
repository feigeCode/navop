# OnetCli 归档迁移提示设计

## 目标

在 OnetCli 最后一个版本中明确告知用户：项目仓库已经归档，后续不再提供功能更新、问题修复或新版本；后续开发已迁移至 Navop。应用内和仓库首页必须提供一致的 Navop 官网与 GitHub 入口。

## 用户入口

### 应用启动提示

主窗口创建完成后，每次启动都展示原生 GPUI Dialog。Dialog 不阻止用户继续使用当前版本，也不提供“不再提示”，因为归档状态永久有效。

内容包括：

- 状态：项目已归档。
- 标题：OnetCli 已停止维护。
- 说明：不再提供功能更新、问题修复或新版本发布。
- 迁移信息：后续开发已迁移至 Navop。
- Navop 官网：<https://navop.dev>。
- Navop GitHub：<https://github.com/feigeCode/navop>。

操作包括：

- 主操作“访问 Navop 官网”，使用系统浏览器打开官网。
- 次操作“查看 Navop GitHub”，使用系统浏览器打开新仓库。
- 关闭操作“继续使用 OnetCli”，关闭 Dialog 并保留当前应用会话。

Dialog 使用独立模块实现，由主窗口启动入口在 `Root` 创建完成后延迟打开。文案支持英文、简体中文和繁体中文。

### 自动更新

正常 GUI 启动不再调度 OnetCli 自动更新检查，避免归档公告与更新行为互相矛盾。已有更新模块和更新命令处理逻辑保留为历史能力，本次不做删除性重构。

### 仓库文档

`README.md` 与 `README_CN.md` 顶部增加醒目的归档公告，明确停止维护并优先展示 Navop 官网和 GitHub。原有功能、安装、截图与历史 Release 内容继续保留，作为归档资料；安装区需注明仅供历史版本使用，不再推荐新用户采用 OnetCli。

`CONTRIBUTING.md` 顶部注明 OnetCli 不再接受功能或缺陷修复贡献，并引导贡献者前往 Navop。历史设计文档、计划、Changelog 和源码标识不做机械替换。

## 边界与风险

- 不修改用户数据、设置或持久化结构。
- 不自动退出或禁用 OnetCli 的现有功能。
- 不在本次代码改动中执行 GitHub 仓库归档；该操作需由仓库所有者在 GitHub Settings 中完成。
- 启动 Dialog 可能与其他启动弹窗竞争，因此必须在根视图创建后通过窗口 defer 调度，并保持实现为单一启动公告。

## 验证

- 单元测试固定 Navop URL、归档文案 contract 和启动接线。
- 验证正常 GUI 启动不再调用 `schedule_update_check`。
- 运行 Rust 格式化、`main` 定向测试和 `cargo check -p main`。
- 检查 README 中英文公告和 CONTRIBUTING 引导链接一致。
