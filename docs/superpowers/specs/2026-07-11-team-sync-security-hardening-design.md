# Team Sync Security Hardening Design

## 目标

修复团队密钥、团队缓存、账号切换、团队退出和云端删除确认中的安全与一致性问题，同时保持现有团队可继续读取和同步。浏览器团队管理 URL 携带 access token / refresh token 的问题明确不在本次范围内。

## 验收范围

本次交付必须满足：

1. 新团队不再直接使用单次 SHA-256 派生的人类口令加密团队数据。
2. 旧团队仍可读取和同步，但 UI 明确显示“旧版加密，待升级”。
3. owner/admin 可显式升级旧团队；升级允许沿用原口令，也允许更换口令。
4. 团队密钥版本变化后，本地旧密钥不得继续显示为可用缓存。
5. 团队缓存、角色和密钥必须按云环境与云用户隔离。
6. 刷新团队列表后，已退出或已被移除的团队不得继续出现在选择器，也不得继续贡献编辑权限。
7. 团队密钥 UI 的持久化状态与运行时解锁状态不得混为同一个枚举。
8. 云端软删除必须确认实际影响了一条记录；零行更新不得被报告为成功。

## 非目标

- 不修改团队管理 Web 登录交接协议或 URL token 传递方式。
- 不改变 `onetcli` 二进制、`ONETCLI_*` 环境变量、`onetcli-mcp` 等兼容标识。
- 不把本机 SQLite 数据库改造成多操作系统用户的数据隔离容器；本次隔离的是云账号对应的团队元数据、角色、密钥和同步写权限。
- 不自动在后台批量改写旧团队的全部云端数据。

## 总体架构

团队口令不再直接充当数据加密密钥。新版团队初始化生成一个随机 256-bit 数据密钥，再使用 Argon2id 从用户口令派生的 key-encryption key 包装该数据密钥。`teams.key_verification` 保存一个带版本前缀的 envelope；成员输入团队口令后解开 envelope，得到真正用于团队记录加解密的高熵数据密钥。

旧版 `key_verification` 没有新版前缀，继续沿用现有验证和解密路径。旧团队升级时，用旧口令解析旧数据密钥，生成新版随机数据密钥和 Argon2id envelope，再通过现有事务 RPC 重加密该团队的全部同步记录。

团队缓存增加云环境、云用户和缓存密钥版本作用域。刷新团队元数据时执行集合协调，删除当前作用域中已经不在服务端团队列表内的缓存，并移除对应运行时密钥。

## 新版团队密钥 envelope

新增独立模块 `cloud_sync/team_key_envelope.rs`，负责版本化 envelope，不把 Argon2id 细节散布到同步和 UI 层。

文本格式使用固定前缀加 Base64 编码的 JSON：

```text
TEAMKEY2:<base64-json>
```

JSON 至少包含：

- `version`: 固定为 2。
- `kdf`: 固定为 `argon2id`。
- `memory_kib`、`iterations`、`parallelism`: 写入生成时参数，便于未来升级。
- `salt`: 每个 envelope 独立生成的随机 salt。
- `nonce`: 包装数据密钥使用的 AES-256-GCM nonce。
- `wrapped_data_key`: AES-256-GCM 密文，明文包含固定 magic 和随机数据密钥。

默认参数以桌面交互可接受的延迟为目标，初始使用 Argon2id 64 MiB、3 iterations、parallelism 1。测试使用显式低成本参数注入，生产常量不因测试而降低。

新版团队数据仍使用现有 AES-GCM blob 格式；传给现有 `encrypt_with_key` 的值变为随机数据密钥的 Base64 文本。现有函数内部额外做一次 SHA-256 不会降低随机数据密钥的熵，也避免一次性扩大个人主密钥和其他加密格式的迁移范围。

## 兼容与升级

`TeamKeyScheme` 分为：

- `Legacy`: 旧 verification 格式，运行时数据密钥等于用户输入的旧团队口令。
- `EnvelopeV2`: 从 Argon2id envelope 解出的随机数据密钥。

新团队初始化直接创建 `EnvelopeV2`。旧团队可继续载入；设置页显示“旧版加密，待升级”。普通成员只能录入并使用旧密钥，不能触发升级。owner/admin 可执行：

- “升级加密”：旧口令与新口令相同，仍生成全新的随机数据密钥并重加密全部记录。
- “轮换密钥”：旧口令与新口令不同，同时升级 envelope 和口令。

新团队初始化和真正更换为新口令时，新口令至少 12 个 Unicode 字符。为避免阻断旧团队升级，沿用原旧口令执行加密升级时不强制满足新长度。

事务 RPC 的参数和原子性保持不变：先准备全部新密文，再一次性更新团队 verification/version 和所有带乐观版本条件的记录。任何记录冲突都回滚整个升级。

## 缓存 schema 与账号隔离

新增 SQLite migration 重建 `team_key_cache`，主键改为：

```sql
PRIMARY KEY (cloud_environment, user_id, team_id)
```

记录字段包括：

- 当前云端 `key_version`。
- 本地密钥对应的 `cached_key_version`。
- `key_verification`。
- 使用个人主密钥加密后的团队口令。
- `last_verified_at` 和当前用户角色。

云环境使用规范化后的 Supabase project URL；用户使用认证 user ID。当前未带账号作用域的旧缓存不能安全判定属于哪个账号，migration 将丢弃这些缓存，保留云端数据和本地连接，用户需在对应账号下重新录入团队口令。

所有 repository `get/list/upsert/delete` API 都要求显式传入 `CloudAccountScope`。UI 不再读取全局无作用域缓存。

## 密钥版本和状态模型

持久化 UI 状态与运行时载入结果拆分：

```text
TeamKeyCacheStatus:
  Missing | Cached | VersionMismatch | LegacyNeedsUpgrade

TeamKeyLoadStatus:
  Unlocked | LegacyUnlocked | Missing | VersionMismatch
```

刷新云端团队元数据时：

- 远端版本与 `cached_key_version` 相同：保留本地密钥。
- 远端版本变化：清空加密口令和 `last_verified_at`，保留旧 `cached_key_version` 用于显示 `VersionMismatch`。
- verification scheme 为 legacy 且版本一致：显示 `LegacyNeedsUpgrade`。

设置页只显示 `TeamKeyCacheStatus`。保存连接前通过 manager 实际解密、验证并载入密钥，不能仅凭 UI 状态判定可保存。

## 团队列表协调与权限

`refresh_team_key_cache` 在成功获取完整团队列表后，以当前 `CloudAccountScope` 执行 reconcile：

1. 获取该作用域的本地团队 ID 集合。
2. upsert 服务端仍存在的团队和当前角色。
3. 删除 `local - remote` 的缓存。
4. 从当前同步服务移除这些团队的运行时数据密钥。

如果成员列表请求失败，该团队本轮不删除旧缓存，也不更新角色，避免将临时网络/RLS 错误误判为退出团队。只有完整团队列表成功且能确认当前成员角色时才更新该团队。

`can_edit_connection` 使用当前账号作用域查询角色。团队缓存不属于当前账号、团队已被协调删除或当前用户未登录时，均不可编辑。连接创建者判断仍保留，但必须先满足连接所属团队在当前作用域可访问，避免旧账号的 `owner_id` 或角色缓存泄漏权限。

本地连接记录仍属于本机数据；账号切换不会自动删除本地连接。失去团队访问权的连接保持本地可恢复，但团队编辑、重新分享和云端写入均被禁止。

## 云端删除确认

`delete_sync_data` 使用 `Prefer: return=representation` 并解析返回记录：

- 恰好一条：删除成功。
- 零条：返回明确的 `NotFoundOrForbidden`/conflict 错误，调用方保留 pending deletion，不得报告成功。
- 多条：视为协议错误。

软删除继续使用现有 delete-wins 语义，不在本次增加 version filter。核心要求是不能把 RLS 零行更新当作已删除。

## UI 行为

团队密钥设置页新增或调整以下状态：

- 未录入。
- 已缓存。
- 版本已变化，需要重新录入。
- 旧版加密，建议 owner/admin 升级。

旧版团队的 owner/admin 按钮显示“升级/轮换”；普通成员只能录入或忘记本地密钥。升级允许新旧口令相同。所有异步结果继续使用通知反馈，失败时不提前改变本地版本或缓存状态。

团队连接选择器只展示当前 `CloudAccountScope` 下 reconcile 后的团队。退出团队、其他账号团队和其他 Supabase 环境团队均不会出现。

## 错误处理

- Argon2 参数、Base64、JSON、AES-GCM 或 magic 校验失败统一映射为无敏感细节的无效团队密钥/envelope 错误。
- 不在日志中记录团队口令、数据密钥、wrapped key 或完整 verification。
- 新 envelope 创建失败时不调用云端初始化/轮换 RPC。
- RPC 成功但本地缓存写入失败时返回明确错误；下次刷新根据云端版本进入 `VersionMismatch`，不得继续使用旧密钥。
- 团队 reconcile 的单团队成员请求失败记录 warning，并保留该团队旧状态；完整团队列表失败时不执行删除协调。

## 测试策略

严格按 TDD 分阶段覆盖：

1. Envelope 单元测试：随机 salt、正确解包、错误口令、篡改、旧格式识别和生产/测试参数分离。
2. TeamKeyManager 测试：新团队保存、旧团队载入、同口令升级、换口令轮换、版本变化清空旧缓存。
3. Repository/migration 测试：复合主键隔离、旧无作用域缓存被清理、不同用户/环境相同 team ID 不互相覆盖。
4. Engine 测试：团队集合协调、成员请求失败保留、退出团队删除、运行时密钥移除。
5. 权限测试：账号匹配、账号切换、过期角色和创建者不得绕过当前团队访问检查。
6. Supabase fake HTTP 测试：删除一行成功、零行失败、多行协议错误。
7. UI 状态/结构测试：legacy、version mismatch、当前作用域选择器和升级按钮权限。
8. 完成前运行相关 crate tests、`cargo check -p main`、`cargo clippy -p one-core -p main --all-targets -- -D warnings`，并执行 code review 与 completion verification。

## 发布与回滚

SQLite migration 只清理无法安全归属的本地团队密钥缓存，不删除连接或云端数据。新版客户端仍能读取旧团队 verification；旧版客户端无法解析新版 envelope，因此团队完成升级后应要求团队成员升级客户端。UI 在执行升级前明确提示这一兼容影响。

若上线后需要回滚客户端，尚未升级的旧团队不受影响；已经升级为 V2 的团队必须继续使用支持 V2 的客户端，不能无损回退到旧客户端。
