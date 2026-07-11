# Team Sync Security Hardening Design

## 目标

修复团队密钥、缓存、账号切换、团队退出和云端删除确认中的安全与一致性问题。产品尚未发布，因此团队加密只支持唯一的 V2 envelope，不保留旧格式兼容。浏览器团队管理 URL 携带 token 的问题不在本次范围内。

## 验收范围

1. 团队口令只用于解包随机团队数据密钥，不直接加密数据。
2. 所有团队 verification 必须是 `TEAMKEY2:` envelope；其他格式视为无效或未初始化。
3. owner/admin 可初始化或轮换团队口令与随机数据密钥。
4. 远端密钥版本变化后，本地旧密钥立即失效。
5. 团队缓存、角色和密钥按 Supabase 环境与云用户隔离。
6. 刷新后清理已退出团队的缓存、权限和运行时密钥。
7. 持久化缓存状态与运行时解锁状态使用不同类型。
8. 云端软删除必须确认恰好影响一条记录。

## 非目标

- 不修改团队管理 Web token 传递方式。
- 不兼容、迁移或自动升级旧团队 verification。
- 不改变个人主密钥和个人云配置的既有 verification 格式。
- 不删除本地连接；失去团队访问权后只禁止团队写操作。

## 团队密钥架构

团队初始化生成随机 256-bit 数据密钥。Argon2id 从团队口令派生 key-encryption key，再用 AES-256-GCM 包装随机数据密钥。`teams.key_verification` 保存 `TEAMKEY2:<base64-json>`，内容包括版本、KDF 参数、随机 salt、随机 nonce 和 wrapped data key。

生产参数为 Argon2id 64 MiB、3 iterations、parallelism 1；测试显式注入低成本参数。团队记录继续使用现有 AES-GCM blob 格式，但密钥输入改为随机数据密钥的 Base64 文本。解析只接受合法 V2 envelope，任何格式或密码学校验失败都返回统一的无效团队密钥错误，不回退旧格式。

## 初始化与轮换

初始化要求至少 12 个 Unicode 字符的口令。owner/admin 轮换时用旧口令解开当前 envelope，生成全新随机数据密钥和 envelope，重加密全部团队记录，再通过现有事务 RPC 原子更新 verification、key version 和记录。新旧口令相同也执行完整数据密钥轮换。任何乐观版本冲突都回滚。

## 缓存与状态

`team_key_cache` 使用 `PRIMARY KEY (cloud_environment, user_id, team_id)`，记录远端版本、本地 cached version、verification、由个人主密钥加密后的团队口令、验证时间和角色。旧无作用域缓存直接丢弃。所有 repository API 显式携带 `CloudAccountScope`。

状态模型为：

```text
TeamKeyCacheStatus: Missing | Cached | VersionMismatch | Invalid
TeamKeyLoadStatus:  Unlocked | Missing | VersionMismatch | Invalid
```

`Invalid` 表示 verification 缺失或不是合法 V2 envelope。UI 展示持久化状态；保存或同步前必须实际解包并载入 runtime 数据密钥。

## 团队协调与权限

完整团队列表获取成功后，对当前 scope upsert 当前团队、清理已退出团队缓存，并删除对应 runtime key。单团队成员列表失败时保留旧缓存和角色。远端版本变化则清空本地加密口令和验证时间。选择器和权限只读取当前 scope；创建者也必须先满足当前账号仍可访问团队。

## 云端删除确认

软删除发送 `Prefer: return=representation`：返回一条成功，零条返回不存在或无权限，多条返回协议错误。失败时保留 pending deletion。

## UI 与测试

UI 只显示未录入、已缓存、版本变化、格式无效。owner/admin 可初始化或轮换，普通成员只能录入或忘记本地口令；不存在 legacy badge、升级按钮或兼容提示。

严格按 TDD 覆盖唯一 V2 格式、初始化和轮换、cache scope、版本失效、团队协调、权限、删除基数和 UI 状态。完成前运行相关测试、`cargo check -p main`、clippy、定向格式检查、code review 和 completion verification。
