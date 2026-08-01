# Changelog

Navop user-facing release notes. Generate and review each bilingual version entry before creating the release tag.

<!-- NAVOP_RELEASES -->

## [v0.10.0] - 2026-07-30

### 中文

#### 更新内容

- 新增 SSH 远程/反向端口转发（`ssh -R`），支持固定远程端口和端口 `0` 自动分配，并贯通连接管理、启动与停止、命令复制、分享、个人同步、状态展示及中英文文档。
- AI Chat 统一执行模式现在会持久化保存，重新打开应用后仍会保留上次选择。

#### 修复与优化

- 完善远程端口转发的生命周期处理，避免自动分配端口时的启动竞态，并确保停止失败后不会残留错误的运行状态。
- 优化 RDP 远程光标移动，使鼠标反馈更加平滑稳定。
- 修复数据库表重命名失败时错误未正确显示的问题。
- 修复 PostgreSQL 主键修改未正确应用的问题。
- 扩大 Tab 重命名输入区域，长名称编辑时可以看到更多内容。

---

### English

#### What's New

- Added SSH remote/reverse port forwarding (`ssh -R`) with both fixed remote ports and automatic port allocation via port `0`, integrated across connection management, start/stop handling, command copying, sharing, personal sync, status display, and bilingual documentation.
- Unified execution mode in AI Chat is now persisted, preserving the selected mode after restarting the application.

#### Fixes and Improvements

- Improved remote port-forwarding lifecycle handling by preventing the startup race during automatic port allocation and ensuring failed cleanup does not leave a stale running state.
- Smoothed remote cursor movement in RDP sessions for more stable pointer feedback.
- Fixed database table rename failures not being surfaced correctly.
- Fixed PostgreSQL primary-key edits not being applied correctly.
- Widened the Tab rename input so longer names remain visible while editing.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.9.8...v0.10.0
