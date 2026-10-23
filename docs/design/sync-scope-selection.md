# Navop 云同步「用户可自选同步数据范围」扩展方案

> 状态：**设计已完成，14 项开放问题已全部决策（2026-10-14），待进入实施**（见第 11 节决策记录）
> 目标：把当前固定同步「连接 / 工作空间 / 钥匙串」的云同步，扩展为**用户可在「设置 - 同步」中逐类型勾选**的可选范围，新增笔记、已保存 SQL、快捷命令、SFTP 收藏、各类设置项、LLM 提供商等类型。
> 全部新增类型延续现状：**使用主密钥做端到端加密**；按 Q3 决策，新增类型**一律仅个人**（`team_id = None`），不涉及团队密钥。
>
> **仍未改动任何代码。** 实施前请先看第 11.1 节的一处理解偏差确认。

---

## 0. 现状勘察结论（方案的事实基础）

### 0.1 两条同步通道并存

| 通道 | 入口 | 传输/存储 | 计划算法 | 冲突能力 |
|---|---|---|---|---|
| **Navop 云同步（OnetCloud）** | `crates/core/src/cloud_sync/engine.rs` + `supabase.rs` | Supabase `sync_data` 表，按 `data_type` 过滤拉取 | `generic_sync.rs::calculate_sync_plan`，**仅比较 `updated_at`，无冲突检测** | `sync_conflict_dialog.rs` 即时二选一，**无挂起机制** |
| **Personal 云同步** | `crates/core/src/cloud_sync/personal/` | 本地目录 / Git / WebDAV，包布局 `.onetcli-sync/records/<data_type>/<id>.json` + `tombstones/` | `planner.rs::PersonalSyncPlanner`，**checksum + updated_at + last_synced_at 三元组** | 有：`LocalDeletedRemoteModified` / `BothModified` 挂起到 `personal_sync_conflicts` 表，UI 在 `main/src/personal_sync_conflicts.rs` |

两条通道**共用同一套加密信封**（`CloudSyncData` + `CloudSyncService::encrypt_blob/decrypt_blob`），这是本次扩展能低成本复用的最大前提。

### 0.2 现有可同步类型

```rust
// crates/core/src/cloud_sync/models.rs:238-242
pub mod data_type {
    pub const CONNECTION: &str = "connection";   // OnetCloud + Personal
    pub const CREDENTIAL: &str = "credential";   // 仅 Personal
    pub const WORKSPACE:  &str = "workspace";    // OnetCloud + Personal
}
```

- OnetCloud 注册点：`engine.rs:100-108`（`WorkspaceSyncType` + `ConnectionSyncHandler`）
- Personal 分派点：`personal/local_source.rs`，靠 `local_id` 前缀（`connection:` / `credential:` / `workspace:`）在 `list_items` / `export_item` / `apply_remote` / `mark_synced` / `delete_item` 五个方法里分支

### 0.3 现有可复用的既有设施

| 设施 | 位置 | 本次是否复用 |
|---|---|---|
| 统一信封 `CloudSyncData`（id/owner_id/team_id/data_type/encrypted_data/key_version/checksum/version/updated_at/deleted_at） | `cloud_sync/models.rs:206-231` | ✅ 直接复用，不加字段 |
| 加密 `encrypt_blob(plaintext, team_id)` → AES-256-GCM，checksum = SHA256(明文) | `cloud_sync/service.rs:311-336` | ✅ 直接复用 |
| 变更检测 checksum | `service.rs:331` | ✅ 直接复用 |
| OnetCloud 类型抽象 `SyncTypeHandler` / `SyncableItem` | `cloud_sync/sync_type.rs` | ✅ 每个新 Item 类型实现一个 |
| 操作队列 + 失败重试（按 `queue_key` 持久化） | `cloud_sync/queue.rs` | ✅ 每类型分配一个 key |
| Personal 计划器 + 墓碑 | `personal/planner.rs`、`file_format.rs` | ✅ 直接复用 |
| 待删除列表 `PendingCloudDeletionRepository` | `storage/repository.rs:883` | ⚠️ 目前只支持 `connection`/`workspace` 两种 entity_type，**需扩展** |
| 冲突挂起 + 解决 UI | `personal/conflict_sink.rs`、`main/src/personal_sync_conflicts.rs` | ⚠️ 仅 Personal 有；OnetCloud 侧无 |

### 0.4 待接入数据类型的现状盘点

| # | 类型（建议 `data_type`） | 持久化 | 稳定 ID | `updated_at` | 内置/自定义区分 | 量级 | 接入难度 |
|---|---|---|---|---|---|---|---|
| 1 | `note` | **文件系统**（`data_local_dir()/navop/notes`，`notebook.json`+`documents.json`+`files/**/*.md`） | ✅ `Uuid` | ⚠️ 有但**正文保存不刷新**（`document_index.rs:19,76,92`） | N/A | 中~大，正文数十 KB | **高** |
| 2 | `saved_sql` | **文件系统**（`config/queries/<db_type>/<connection_id>/<db>/*.sql`） | ❌ 只有相对路径 | ❌ 仅 mtime | N/A | 小 | 中 |
| 3 | `quick_command` | SQLite `quick_commands` | ✅ `id` | ✅ | ❌ **无 `is_builtin` 列，30 条内置命令混在同表** | 小 | 中（**有污染风险**） |
| 4 | `sftp_favorite` | SQLite `sftp_favorite_paths` | ✅ `id`，业务键 `(connection_key, path)` | ✅ | N/A | 极小（≤20） | 中（`connection_key` 是本地键） |
| 5 | `settings_general` | `settings.json` | ❌ 单条 | ❌ | 部分 | 1 | 低 |
| 6 | `settings_database` | `settings.json` 散字段 | ❌ 单条 | ❌ | 部分 | 1 | 低 |
| 7 | `settings_terminal` | `settings.json` + **独立 `terminal-settings.json`** | ⚠️ 高亮规则有 `id` | ❌ | ⚠️ 内置规则被复制进 `custom_highlights` | 1 | 中 |
| 8 | `settings_agent` | `settings.json`（`AiChatSettings`） | ❌ 单条 | ❌ | 部分 | 1 | 低 |
| 9 | `settings_mcp` | `settings.json`（`McpSettings` + `ToolExposureSettings`） | ❌ 单条 | ❌ | ⚠️ 仅代码内默认值可辨 | 1 | 低 |
| 10 | `settings_keybindings` | `settings.json` 的 `custom_keybindings` | ✅ `action_id` 天然稳定 | ❌ | ✅ **天然只含用户改过的** | 极小 | **最低** |
| 11 | `llm_provider` | SQLite `llm_providers` | ✅ `id`（非自增） | ✅ | ✅ `ProviderType::is_builtin()` | 极小 | 中（**api_key 本地明文**） |

---

## 1. 核心设计决策（先看这三条，其余都是它的展开）

### D1. 引入「同步粒度」分类：`SyncKind::Item` vs `SyncKind::Singleton`

现有框架假设每条同步数据都是"一项一条记录、有 `updated_at`"。但 6 类设置项完全不符合这个假设（单条、无时间戳、无 ID）。强行给每条设置造 `updated_at` 语义成本高且容易出错。

因此把新增类型分成两类：

- **`Item`（逐条同步）**：笔记、已保存 SQL、快捷命令、SFTP 收藏、LLM 提供商 —— 每个实体一条云端 blob，有独立 ID，逐条增量。
- **`Singleton`（整包同步）**：通用 / 数据库 / 终端 / Agent / MCP / 快捷键设置 —— 整个分组**只对应一条**云端记录，`cloud_id` 由 `data_type` 稳定派生（如 `singleton:settings_general`），**变更检测只用 checksum，不用 `updated_at`**。

好处：设置类不需要给 `AppSettings` 造时间戳；`KeepBoth` 这类对单条记录无意义的冲突策略可以直接屏蔽。

### D2. 新增统一的同步基线表 `sync_item_state`，而不是给每个业务表加列

`quick_commands` / `sftp_favorite_paths` / `llm_providers` 都要新增 `cloud_id` / `last_synced_at` 才能接入增量同步；笔记和已保存 SQL 在文件系统里，根本没有表可以加列。

**提案**：新增一张跨通道共用的基线表，把"本地稳定键 → 云端 ID / checksum / 基线时间"外置：

```sql
CREATE TABLE sync_item_state (
    scope_env   TEXT NOT NULL,   -- Supabase project_url，或 "personal"
    user_id     TEXT NOT NULL,   -- 云端 user id，Personal 通道填 "personal"
    data_type   TEXT NOT NULL,   -- "note" / "settings_general" / ...
    local_key   TEXT NOT NULL,   -- 本地稳定键：SQLite 主键 / Uuid / 相对路径 / "singleton"
    cloud_id    TEXT,
    checksum    TEXT,            -- 上次同步时的明文 checksum
    content_ts  INTEGER,         -- 本地内容变更时间（秒）；无时间戳的类型填 checksum 变更时刻
    last_synced_at INTEGER,
    PRIMARY KEY (scope_env, user_id, data_type, local_key)
);
CREATE INDEX idx_sync_item_state_cloud ON sync_item_state (data_type, cloud_id);
```

**收益**
- 不污染业务表，`quick_commands` / `sftp_favorite_paths` / `llm_providers` **零 `ALTER TABLE`**
- 为所有新增类型（含没有 `updated_at` 的设置类）统一提供 `(checksum, content_ts, last_synced_at)` 三元组，直接满足 `PersonalSyncPlanner` 的输入契约
- 两条通道共用，避免 OnetCloud 与 Personal 各写一套基线
- `local_key` 是字符串，天然容纳文件系统类型（Uuid / 相对路径）

**代价与应对**
| 代价 | 应对 |
|---|---|
| `list_local` 多一次表扫描 | 表很小（每类型几十~几百行），全量读入内存建 `HashMap` |
| 业务行被删后 state 变孤儿 | 每轮 pass 结束后做一次 GC：`DELETE WHERE (data_type, local_key) NOT IN (本次 list_local 结果)`；GC 前必须确认该条不在删除传播中 |
| 与 `StoredConnection.cloud_id`（存在业务表里）两套并存 | 现状三类型**保持不变**（避免迁移风险），只有新增类型走 state 表。长期可考虑统一，本方案不做 |

> 备选方案（不推荐）：给每个业务表 `ALTER TABLE ADD COLUMN cloud_id / last_synced_at`。优点是和 `StoredConnection` 一致；缺点是要改 3 张表 + 笔记/已保存 SQL 无处安放 + 迁移不可逆。

### D3. 取消勾选的语义 = **停止同步 + 保留云端 + 保留本地**

用户取消某类型的勾选后：
1. **立即停止**该类型的上传 / 下载 / 删除传播（在 `list_local` 之前就短路，一轮里完全跳过该 handler）
2. **云端数据保留**（不删）：别的设备可能还在同步；用户重新勾选时可恢复
3. **本地数据保留**（绝不删用户数据）
4. 重新勾选时：不清空 state 表。因为有 `cloud_id` + `checksum` 基线，`PersonalSyncPlanner` 能正确判断"无变化 → mark_synced"，不会重复创建

---

## 2. 模块一：同步配置的数据结构与持久化

### 2.1 类型清单用编译期常量表描述（唯一事实来源）

```rust
// crates/core/src/settings.rs（新增，放在 PersonalSyncSettings 附近）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncScopeKey {
    Connection, Workspace, Credential,
    Note, SavedSql, QuickCommand, SftpFavorite,
    SettingsGeneral, SettingsDatabase, SettingsTerminal,
    SettingsAgent, SettingsMcp, SettingsKeybindings,
    LlmProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind { Item, Singleton }

pub struct SyncScopeDescriptor {
    pub key: SyncScopeKey,
    pub data_type: &'static str,       // CloudSyncData.data_type
    pub queue_key: &'static str,       // SyncOperationQueue 持久化 key
    pub kind: SyncKind,
    pub personal_only: bool,           // true = 仅 Personal 通道可用
    pub default_enabled: bool,
    pub i18n_title: &'static str,      // e.g. "Settings.Sync.Scope.note"
    pub i18n_desc: &'static str,
}

pub const SYNC_SCOPE_DESCRIPTORS: &[SyncScopeDescriptor] = &[
    // key,               data_type,              kind,       personal_only, default
    d(Connection,         "connection",           Item,       false, true),
    d(Workspace,          "workspace",            Item,       false, true),
    d(Credential,         "credential",           Item,       true,  true),
    d(Note,               "note",                 Item,       false, false),
    d(SavedSql,           "saved_sql",            Item,       false, false),
    d(QuickCommand,       "quick_command",        Item,       false, false),
    d(SftpFavorite,       "sftp_favorite",        Item,       false, false),
    d(SettingsGeneral,    "settings_general",     Singleton,  false, false),
    d(SettingsDatabase,   "settings_database",    Singleton,  false, false),
    d(SettingsTerminal,   "settings_terminal",    Singleton,  false, false),
    d(SettingsAgent,      "settings_agent",       Singleton,  false, false),
    d(SettingsMcp,        "settings_mcp",         Singleton,  false, false),
    d(SettingsKeybindings,"settings_keybindings", Singleton,  false, false),
    d(LlmProvider,        "llm_provider",         Item,       true,  false),
];
```

**为什么用 struct 字段而不是 `HashSet<String>`**：
- 编译期穷尽，新增类型时编译器会逼你补齐所有 `match`
- UI 顺序固定（数组顺序即展示顺序）
- `personal_only` / `default_enabled` / i18n key 与类型定义同处一处，UI 和同步引擎读同一份数据，避免"UI 里散落 if 判断"这类最容易漂移的写法

### 2.2 持久化：放在 `AppSettings`（`settings.json`）

```rust
// crates/core/src/settings.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncScopeSettings {
    /// 0 = 从未初始化（存量用户升级后的首次读取），1 = 当前版本
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)] pub connection: bool,
    #[serde(default)] pub workspace: bool,
    #[serde(default)] pub credential: bool,
    #[serde(default)] pub note: bool,
    #[serde(default)] pub saved_sql: bool,
    #[serde(default)] pub quick_command: bool,
    #[serde(default)] pub sftp_favorite: bool,
    #[serde(default)] pub settings_general: bool,
    #[serde(default)] pub settings_database: bool,
    #[serde(default)] pub settings_terminal: bool,
    #[serde(default)] pub settings_agent: bool,
    #[serde(default)] pub settings_mcp: bool,
    #[serde(default)] pub settings_keybindings: bool,
    #[serde(default)] pub llm_provider: bool,
}
```

挂载与两个附加字段（Q1 的"首次开启弹确认"需要"曾经开启过"的痕迹）：

```rust
pub struct SyncScopeSettings {
    #[serde(default)] pub schema_version: u32,
    /// 曾经开启过的类型 —— 用于判断「是否首次开启」从而决定是否弹 R2 确认（Q1）
    #[serde(default)] pub ever_enabled: Vec<String>,
    /// 用户勾选了「不再提示」后，Personal 下开启新类型不再弹确认（Q1）
    #[serde(default)] pub suppress_personal_warning: bool,
    // … 14 个 bool 字段
}
```

挂在 `AppSettings.sync_scope`（与 `sync_enabled` / `sync_provider` / `personal_sync` 并列，`settings.rs:1104-1112` 附近），走 `AppSettings::update_and_save` 自动落盘 `settings.json`。

**为什么放 settings.json 而不是 SQLite**
- 零迁移，`#[serde(default)]` 天然兼容旧文件
- 同步引擎在 `cx: &App` 上就能读到（`AppSettings::global(cx)`），不必穿 `StorageManager`
- **关键**：它本身就是"本机偏好"，语义上就该待在 `settings.json`

> ⚠️ **必须排除自身**：`SyncScopeSettings` 绝不能被「通用设置同步」包含（详见 R7）。

### 2.3 provider 切换时的强制取消（交互要求的落地）

```rust
impl SyncScopeSettings {
    /// 按当前同步方式计算"实际生效"的勾选集合：
    /// provider == OnetCloud 时，personal_only 项一律视为 false。
    pub fn effective(&self, provider: SyncProvider) -> HashSet<SyncScopeKey> {
        SYNC_SCOPE_DESCRIPTORS.iter()
            .filter(|d| !d.personal_only || provider == SyncProvider::Personal)
            .filter(|d| self.get(d.key))
            .map(|d| d.key)
            .collect()
    }

    /// 切到 OnetCloud 时把 personal_only 项写回 false 并落盘（保证"取消勾选"是真的持久化，
    /// 而不只是 UI 上置灰 —— 否则用户切回 Personal 时会看到幽灵勾选）。
    pub fn force_disable_personal_only(&mut self) { /* ... */ }
}
```

`sync_provider_item` 的 set 回调改为：

```rust
|val: SharedString, cx: &mut App| {
    AppSettings::update_and_save(cx, |settings| {
        settings.sync_provider = SyncProvider::from_str(&val);
        if settings.sync_provider == SyncProvider::OnetCloud {
            settings.sync_scope.force_disable_personal_only();
        }
    });
},
```

### 2.4 UI 实现

**位置**：`main/src/setting_tab.rs` 的 Sync 页（页索引 `SYNC_SETTINGS_PAGE_INDEX = 5`，分组工厂 `sync_setting_group()` 在 `:1078`）。

**做法**：新增 `main/src/settings/sync_scope_settings.rs`，导出
`fn sync_scope_items() -> Vec<SettingItem>`，复刻 `main/src/settings/tool_exposure_settings.rs:177-206` 的成熟模式（枚举 + `title_key()` + `SettingField::checkbox` 批量生成）。在 `sync_setting_group()` 里追加一个 `SettingGroup::new()` 承载这 14 项，或作为独立分组插入 Sync 页。

**置灰**：gpui-kit 的 `SettingItem::disabled(bool)` 已存在（`.../setting/item.rs:131`），且 `BoolField` 会把它转发给 `Checkbox::disabled()` / `Switch::disabled()`（`.../setting/fields/bool.rs:38-55`），所以：

```rust
let provider = AppSettings::global(cx).sync_provider;
let locked = descriptor.personal_only && provider == SyncProvider::OnetCloud;
SettingItem::new(
    t!(descriptor.i18n_title),
    SettingField::checkbox(
        move |cx: &App| AppSettings::global(cx).sync_scope.get(descriptor.key),
        move |val: bool, cx: &mut App| {
            AppSettings::update_and_save(cx, |s| s.sync_scope.set(descriptor.key, val));
        },
    ).default_value(descriptor.default_enabled),
)
.disabled(locked)
.description(t!(descriptor.i18n_desc).to_string())
```

**实时性**：`setting_pages` 每次渲染都会重建（详见 `setting_tab.rs:1069-1077` 的注释，WebDAV 条目就是靠这个机制动态增减的），切换 provider 后 checkbox 的 `disabled` 会自动重新求值，**不需要手动 refresh**。

> ⚠️ 若在 Sync 页之前插入新页面，必须同步修改 `setting_tab.rs:65-66` 的 `SYNC_SETTINGS_PAGE_INDEX` / `TEAM_KEYS_SETTINGS_PAGE_INDEX`。本方案只加分组，不动页序。

**i18n**：`main/locales/main.yml` 的 `Settings.Sync.Scope.*` 段，每个类型 `xxx` + `xxx_desc` 两个 key，en / zh-CN / zh-HK 三语齐全。注意仓库插值语法是 `%{var}`（`rust_i18n` 只认 `%{name}`）。

### 2.5 顺序与"置灰"规则（已按 Q12 决策更新）

分组内按"数据类 → 设置类"分两组展示，每组内顺序与 `SYNC_SCOPE_DESCRIPTORS` 一致。**每行右侧都带状态列**（见 2.6）：

```
同步数据范围
  ☑ 连接配置          [置灰·不可取消]   上次同步 5 分钟前
  ☑ 工作空间          [置灰·不可取消]   上次同步 5 分钟前
  ☑ 钥匙串条目        [Personal 可选 / OnetCloud 置灰取消]   ⚠ 同步失败：主密钥未解锁
  ☐ 笔记                               未同步
  ☐ 已保存的 SQL                       未同步
  ☐ 已保存的快捷命令                    未同步
  ☐ SFTP 收藏列表                      未同步
  ☐ LLM 提供商        [仅 Personal]     未同步
  ─────────────
  ☐ 通用设置                           未同步
  ☐ 数据库设置                         未同步
  ☐ 终端设置                           未同步
  ☐ Agent 设置                         未同步
  ☐ MCP 设置                           未同步
  ☐ 快捷键设置                         未同步
```

**两类置灰，语义不同，需要在 descriptor 里区分**：

```rust
pub enum LockMode {
    /// 始终启用且不可取消（连接 / 工作空间）：云同步的核心价值就在这两项，
    /// 允许取消会让"同步已开启但没有东西在同步"，是无意义的状态。
    AlwaysOn,
    /// 仅 Personal 可用；OnetCloud 下强制取消 + 置灰（钥匙串 / LLM 提供商）
    PersonalOnly,
    /// 正常可勾选
    Optional,
}
```

> ⚠️ **需要你确认的一处理解偏差**：Q12 里"默认勾选且置灰的默认同步项"，我理解为 **连接 / 工作空间 = 始终启用、置灰不可取消**，而**钥匙串保持可勾选**（Personal 下可由用户关掉，OnetCloud 下按你最初提的交互要求强制取消 + 置灰）。
> 若你的原意是"钥匙串也始终置灰不可取消"，或反过来"连接/工作空间也允许取消"，请指出 —— 这只改 `LockMode` 一行常量，成本很低。

### 2.6 每行状态展示（Q12）

**展示内容**（成功 / 失败二选一）：

| 状态 | 展示 | 数据来源 |
|---|---|---|
| 最近一次成功 | `上次同步 N 分钟前`（超过 1 天改为绝对时间） | `sync_item_state.last_success_at`（按 `data_type` 聚合取 max） |
| 最近一次失败 | `⚠ <错误摘要>`，hover 显示完整错误 | `sync_item_state.last_error`（该类型最近一次失败的错误码 + 消息） |
| 从未同步过 | `未同步` | 无记录 |
| 主密钥未解锁 | `⚠ 未同步：主密钥未解锁`（**所有行统一显示**，见 Q11） | `crypto::has_master_key()` |

**存储**：`sync_item_state` 表按 `data_type` 维度额外维护两个聚合字段（见 D2 表结构补充）：

```sql
ALTER TABLE sync_item_state ADD COLUMN ...  -- 见下方"按类型聚合"设计
```

因为状态是**按类型**展示而不是按条目展示，而 `sync_item_state` 是**按条目**的，所以另建一张轻表更清晰：

```sql
CREATE TABLE sync_type_status (
    scope_env     TEXT NOT NULL,
    user_id       TEXT NOT NULL,
    data_type     TEXT NOT NULL,
    last_success_at INTEGER,          -- 该类型最近一次完整成功的时间
    last_error_code TEXT,             -- "key_mismatch" / "network" / "size_limit" / ...
    last_error_at   INTEGER,
    last_error_msg  TEXT,
    PRIMARY KEY (scope_env, user_id, data_type)
);
```

- 成功一轮后写 `last_success_at` 并**清空** `last_error_*`（错误已被解决）
- 失败时写 `last_error_*`，**不动** `last_success_at`
- 未勾选的类型：显示上次成功时间（灰显），不显示失败

**UI 实现**：`SettingItem::render`（自定义渲染，因为要在右侧放一列状态），或 `SettingItem::new(...).description(...)` 把状态塞进 description。
**推荐用 `SettingItem::render`** —— 状态要右对齐、要带颜色（成功灰 / 失败橙红），description 做不到。

**刷新时机**：`setting_pages` 每次渲染重建（见 2.4），状态从 SQLite 读；同步完成后由同步运行时触发一次 `cx.notify()` / `cx.refresh()`。

> 注意：状态表的读写在 `cx: &App` 上完成，需要 `cx.try_global::<GlobalStorageState>()` 拿 `StorageManager`（`main/src/personal_sync_runtime.rs` 已有同款取法）。

---

## 3. 模块二：加密方案

### 3.1 完全复用现有通道，不引入新算法

```rust
// 每个新增类型的 prepare / decrypt 都照抄 credential 的写法（service.rs:436-474）
let plaintext = serde_json::to_string(&plain_data)?;
let checksum  = CloudSyncService::calculate_blob_checksum(&plaintext);
let encrypted = service.encrypt_blob(&plaintext, None)?;  // 个人数据 team_id = None
```

- 算法：AES-256-GCM，`base64(nonce + ciphertext)`（现状）
- 密钥：个人数据用主密钥（`select_encrypt_key(None)`），团队数据用团队密钥 —— **本方案新增类型一律 `team_id = None`**（见 Q3）
- `key_version`：`select_key_version(None, teams)` → `self.key_version`
- checksum：明文 SHA-256（**加密前**计算），与现状一致

### 3.2 各类型明文载荷定义

统一约定：
- 每个 `*PlainData` **必须带 `format_version: u32`**（照 `WorkspacePlainData.format_version` / `CredentialPlainData.format_version` 的先例），为将来字段演进留口
- 反序列化时所有新增字段用 `#[serde(default)]`，保证旧载荷可读

```rust
pub struct SavedSqlPlainData {
    pub format_version: u32,
    pub name: String,                    // 文件名（不含扩展名）
    pub db_type: String,                 // 复合键 part 1
    pub connection_cloud_id: Option<String>, // 复合键 part 2（见 3.4）
    pub database: Option<String>,        // 复合键 part 3
    pub content: String,                 // SQL 正文
}

pub struct QuickCommandPlainData {
    pub format_version: u32,
    pub name: String,
    pub command: String,
    pub description: Option<String>,
    pub pinned: bool,
    pub sort_order: i32,
    pub group_name: Option<String>,
    pub group_color: Option<String>,
    pub shortcut: Option<String>,
    pub connection_cloud_id: Option<String>,  // 本地 connection_id → cloud_id
}

pub struct SftpFavoritePlainData {
    pub format_version: u32,
    pub path: String,
    pub sort_order: i32,
    pub connection_cloud_id: Option<String>,  // 见 3.4
}

pub struct NotePlainData {               // 每篇文档一条记录
    pub format_version: u32,
    pub notebook_id: String,             // Uuid
    pub notebook_name: String,
    pub document_id: String,             // Uuid
    pub relative_path: String,           // 笔记本内相对路径，用于还原树形结构
    pub title: String,
    pub content: String,                 // markdown 正文
}

// 6 个设置类 Singleton 共用同一个信封，payload 是白名单字段的 JSON 子集
pub struct SettingsPlainData {
    pub format_version: u32,
    pub group: &'static str,             // "general" / "database" / ...
    pub payload: serde_json::Value,      // 仅白名单字段
}

pub struct LlmProviderPlainData {        // ⚠️ 不派生 Debug（含 api_key）
    pub format_version: u32,
    pub name: String,
    pub provider_type: String,
    pub api_key: Option<String>,         // 用户明确要求同步
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub model: String,
    pub models: Vec<String>,
    pub max_tokens: Option<i32>,
    pub temperature: Option<f32>,
    pub enabled: bool,
    pub is_default: bool,
}
```

### 3.3 敏感字段的 Debug 派生禁令

现状 `CredentialPlainData` 已明确注释"不要为它派生 `Debug`，避免密码和私钥内容被调试日志意外输出"（`models.rs:415-417`）。**新增规则**：

- `LlmProviderPlainData`（含 api_key）**不派生 Debug**
- 其余不含密钥的可以派生，但建议统一不派生，降低未来漏改风险
- 所有 `tracing::info!("...{:?}")` 不得打印这些结构（review 检查项）

### 3.4 本地 ID → 云端 ID 的转换（跨设备正确性的关键）

现状已有成熟范式（`local_source.rs:517-559` 的 `enrich_credential_references`）：**拿不到 cloud_id 时置空/置 0，绝不上传仅本机有效的整数 ID**，否则目标设备会误绑同号条目。

新增类型中两类有此问题：

| 类型 | 本地字段 | 处理 |
|---|---|---|
| SFTP 收藏 | `connection_key`，默认 `local:{id}`（`sftp_favorite_path.rs:17-25`） | 上传时用 `connection_id` 查 `StoredConnection.cloud_id`；查不到则整条**跳过上传**并记 warning（收藏脱离连接没有意义） |
| 快捷命令 | `connection_id`（外键到 `connections.id`） | 同样转 `cloud_id`；查不到则 `connection_cloud_id = None`（命令本身仍可同步，只是不绑定连接） |
| 已保存 SQL | 路径里含 `<connection_id>/<database>` | 上传时把 `<connection_id>` 段替换为 `cloud:{cloud_id}`；目录名本身就要做这层映射 |

### 3.5 主密钥轮换必须覆盖新类型

现状：`service.rs:210-240` 的轮换按传入的 `cloud_data_list` 逐条 `re_encrypt_sync_data`，**不按 data_type 过滤**。因此只要轮换入口拉取的是全量（`list_sync_data(None, ...)`），新增类型会自动被覆盖。

**必须验证的点**：确认主密钥轮换入口确实是 `data_type = None` 全量拉取，而不是按 `connection`/`workspace` 两个类型分别调用。若不是，需要把新类型加入轮换列表，否则轮换后新类型会永久解密失败（**P0 级**）。

---

## 4. 模块三：增量同步与冲突合并

### 4.1 统一的增量判据：`sync_item_state` 提供三元组

每轮 pass 开始时：

```
state = SELECT * FROM sync_item_state WHERE scope_env=? AND user_id=?
for each enabled descriptor:
    items = list_local(descriptor)              // 读业务数据
    for item in items:
        s = state.get(descriptor.data_type, item.local_key)
        item.checksum      = sha256(明文)
        item.updated_at    = 业务 updated_at 或 s.content_ts（无时间戳时）
        item.last_synced_at= s.last_synced_at
        item.cloud_id      = s.cloud_id
```

这样 `PersonalSyncPlanner`（`planner.rs:137-166`）的三元组判据可以直接工作：

| 条件 | 判定 |
|---|---|
| `checksum == remote.checksum` | 已同步 → `to_mark_synced`（**这一条让设置类 Singleton 不需要 `updated_at` 也能正确跳过**） |
| 仅本地变 | `to_update_cloud` |
| 仅远端变 | `to_update_local` |
| 双方都变 | `conflicts.push(BothModified)` |
| 本地无 cloud_id | `to_upload` |

### 4.2 OnetCloud 侧接入

新增类型各实现一个 `SyncTypeHandler`（`sync_type.rs:76-206`），通过 `engine.register_type()` 或改 `engine.rs:100-108` 的 handlers 数组注册。

**但现状 `generic_sync::calculate_sync_plan` 只比较 `updated_at`，不比较 checksum**（`generic_sync.rs:497-506`）。对 Singleton 类型（无 `updated_at`）这会出问题。

**提案**：给 `SyncTypeHandler` 增加一个带默认实现的钩子：

```rust
/// 判定本地与云端的同步方向。默认按 updated_at；Singleton 类型覆盖为按 checksum。
fn compare(&self, local: &Self::Item, cloud: &CloudSyncData) -> SyncDirection {
    // 默认：把 cloud_id / last_synced_at 已在 state 表中解析好的值用于比较
}
```

或者更省事的做法：**让 OnetCloud 侧也直接复用 `PersonalSyncPlanner` 的判据**（它更完善，有冲突检测）。这是更彻底的方案但属于引擎重构，本方案**先只做"新类型能拿到正确基线"**，把"统一两条通道的计划算法"列为后续项（见 Q8）。

每个新类型分配独立 `queue_key`（如 `"sync:quick_command"`），自动获得 `SyncOperationQueue` 的失败重试与持久化（`queue.rs`）。

### 4.3 Personal 侧接入

在 `PersonalSyncLocalRepositorySource` 里为每个新类型加前缀常量与五个分支方法（`list_items` / `export_item` / `apply_remote` / `mark_synced` / `delete_item`）。

`local_id` 前缀设计：

| 类型 | 前缀 | local_key |
|---|---|---|
| 笔记 | `note:` | document `Uuid` |
| 已保存 SQL | `savedsql:` | 相对路径（`db_type/conn/database/name.sql`） |
| 快捷命令 | `quickcmd:` | 表主键 |
| SFTP 收藏 | `sftpfav:` | 表主键 |
| 设置（6 类） | `settings:` | `"general"` / `"database"` / …（固定值） |
| LLM 提供商 | `llmprovider:` | 表主键 |

顺序约束（沿用现状注释：被引用方必须先落地）：`credential → workspace → connection → llm_provider → quick_command → sftp_favorite → saved_sql → note → settings`。

### 4.4 冲突合并策略

分三类，策略不同：

**(a) 逐条 Item 类型（快捷命令 / SFTP 收藏 / LLM 提供商 / 已保存 SQL）**
- 复用 `BothModified` 冲突 → Personal 侧挂起到冲突表，`main/src/personal_sync_conflicts.rs` 让用户选 `UseCloud` / `UseLocal` / `KeepBoth`
- OnetCloud 侧无挂起机制，走 `sync_conflict_dialog.rs` 即时二选一（现状行为，本方案不扩展）

**(b) Singleton 设置类型 —— Q8 已定：做「字段级三方合并」**

`KeepBoth` 对"通用设置"这种单条记录没有意义（不能存在两份主题），整体二选一又会丢一边修改。按 Q8 决策，实现标准三方合并：

```
base   = 上次同步成功时的 payload（新增列 base_payload 存）
local  = 当前本地 payload
remote = 云端解密出的 payload

对每个 key:
    local_changed  = local[key]  != base[key]
    remote_changed = remote[key] != base[key]
    ├─ 仅 local_changed  → 取 local[key]
    ├─ 仅 remote_changed → 取 remote[key]
    ├─ 都变但值相同      → 取该值（无冲突）
    ├─ 都变且值不同      → 记为冲突字段
    └─ 都没变            → 取 base[key]

冲突字段为空   → 自动合并，写回本地并上传合并结果，静默完成
冲突字段非空   → 降级为整体二选一（UseCloud / UseLocal），
                 冲突弹窗里逐字段列出「字段名 + 本地值 + 云端值」
```

**存储**：`sync_item_state` 增加 `base_payload TEXT`（仅 Singleton 类型用），在每次**同步成功**后更新为"本次合并后的最终 payload"。

**注意点**：
- payload 需先展平成 `Map<String, Value>`（扁平 key，如 `ui_scale_percent`、`terminal_font_size`），嵌套结构按路径展开成 `a.b.c` 形式
- 新增字段（base 里没有的 key）：视为"两边都没变"，若只有一边有则采用有的那边
- 删除字段（base 里有、某边没有）：按"该边显式删除"处理，另一边没变则采纳删除
- 设置项基本是 bool / enum / number / string，实现不难，**但 `custom_keybindings` 是 `HashMap<String, Vec<String>>`**，展平后 key 是 `action_id`，value 是数组 —— 需要按 action_id 逐项做同样的三方合并（**这个类型用三方合并收益最大**：A 机改了 A 快捷键、B 机改了 B 快捷键，应自动合并）

**(c) 笔记**
- 笔记正文是 markdown，三方合并成本过高
- **v1 策略**：冲突时生成冲突副本 —— `<title> (冲突副本 · <设备名> · <时间>).md`，两边内容都保留（等价于 `KeepBoth`）
- `crates/notes` 已有 `markdown_conflict.rs`，可复用其命名/写入逻辑

### 4.5 删除传播

新增类型必须接入删除传播，否则"在 A 机删除 → B 机还在"会变成幽灵数据，且"取消勾选再勾选"会复活旧数据。

| 通道 | 机制 | 需改动 |
|---|---|---|
| Personal | `tombstones/<data_type>/<id>.json` + `apply_remote_tombstones`（`worker.rs:302`） | 新类型自动可用（只要 `delete_item` 实现了）；但要注意 `LocalDeletedRemoteModified` 基线从 state 表取 |
| OnetCloud | `pending_cloud_deletions` 表 + `deleted_at` 软删除 | `PendingCloudDeletionRepository` **当前只支持 `connection` / `workspace` 两种 entity_type**（`sync_type.rs:180-185` 的 if/else），需扩展为通用的 `entity_type` 过滤 |

**SQLite 类型的删除难题**：`quick_commands` / `sftp_favorite_paths` / `llm_providers` 都是物理删除，删完就查不到 `cloud_id` 了。
**解法**：删除业务行时，先读 state 表拿到 `cloud_id`，写入 `pending_cloud_deletions`，**再**删业务行（同一事务）。state 表那一行延迟到云端删除确认后再 GC。

### 4.6 「取消勾选」的执行点

在 `list_local` 之前短路。两条通道各加一层：

- OnetCloud：`SyncEngine` 的 handlers 遍历处，或 `generic_sync` 开头 —— 若 `!scope.effective(provider).contains(descriptor.key)` 直接返回空 `SyncResult`
- Personal：`PersonalSyncLocalRepositorySource::list_items` 里按 descriptor 过滤；且 `apply_remote` 遇到未勾选的 `data_type` 直接 `Ok(())` 跳过（**不是报错**）

⚠️ 注意区分「未勾选」与「未知类型」：
- 未勾选 → 静默跳过
- 未知类型（`data_type` 不认识）→ 记 warning 跳过，**绝不报错**（见 R2）

---

## 5. 模块四：新旧版本兼容性

### 5.1 存量用户的默认勾选策略（关键）

用 `schema_version` + `initialized` 区分首次运行：

```
读 settings.json
  ├─ sync_scope.schema_version == 0（字段不存在 / 旧文件）
  │     → 按「首次初始化策略」填默认值，写回 settings.json，schema_version = 1
  └─ schema_version >= 1
        → 直接用（缺失字段由 #[serde(default)] 兜底）
```

**首次初始化策略（Q1 已定：统一默认全关 + Personal 下首次开启弹确认）**：

| 类型 | 默认 | 理由 |
|---|---|---|
| 连接 / 工作空间 | **开**（且置灰不可取消） | 存量用户升级后**绝不能掉数据**。关掉会导致"升级后不再同步已有类型"，是最危险的回归 |
| 钥匙串 | **开**（Personal 下可取消；OnetCloud 下强制取消 + 置灰） | 同左 |
| 新增 11 项 | **关**（**两个通道都关**） | ① 用户没有预期，突然上传笔记 / SQL / LLM api-key 属于隐私意外；② **仅 Personal 通道**：见 R2，旧版本客户端遇到新 `data_type` 会让**整轮**同步失败；③ 保守起步、可逐步放开 |

**Personal 下首次开启的确认弹窗**（Q1 决策的落地）：

- 触发条件：`sync_provider == Personal` 且该类型此前从未开启过（用 `SyncScopeSettings` 里一个 `ever_enabled: HashSet<SyncScopeKey>` 或按位标记记录）
- 弹窗内容：「开启『<类型名>』同步后，本机与其他设备都需要升级到 vX.Y 以上才能正常同步。未升级的设备会遇到同步失败。是否继续？」
- OnetCloud 下**不弹**（该通道不存在 R2，见下方澄清）
- 用户勾选「不再提示」后写进 `AppSettings`，后续开启同类项不再弹

**关于 R2 的适用范围（重要澄清）**：

- **OnetCloud 通道：新类型默认开也是安全的**。`generic_sync.rs:45` 是 `list_sync_data(Some(handler.data_type()), ...)` —— **按类型过滤拉取**。旧版只会请求 `connection` / `workspace`，服务端不会返回新类型记录，旧版**根本感知不到**新类型的存在，原有同步完全不受影响。你的直觉在这条通道上是对的。
- **Personal 通道：会出现"连原有数据一起挂"**。原因是它不做类型过滤，失败链路是：

  ```
  worker.rs:222      list_records(None, None)            // 全量，三后端均无类型白名单
    → planner.rs:98  远端 (data_type, cloud_id) 在本地找不到 → 无条件 to_download
    → worker.rs:435  self.local.apply_remote(record, None).await?   // ← ? 抛出
    → local_source.rs:385  other => Err(SyncStoreError::Parse(...))
    → 冒泡：apply_downloads → apply_plan → run_locked_pass → run_pass → 整轮失败
  ```

  连带后果（`apply_plan` 顺序见 `worker.rs:388-402`，uploads → cloud_updates → local_updates → downloads → synced_marks → conflicts）：
  - `apply_downloads` 的循环在**第一条**新类型记录处就 `?` 中断 → 排在它后面的下载全部跳过（**包括别的设备新增的连接**）
  - `apply_synced_marks`（`:440`）不执行 → `last_synced_at` 不更新 → 下一轮 planner 把条目重新判为"自上次同步后有变动"，可能反复上传 / 反复冲突
  - `apply_conflicts`（`:458`）不执行 → 新冲突不会被挂起，用户看不到冲突提示
  - `main/src/personal_sync_status.rs:59-74` 的 `health_from_error` 把 `Parse` 落到 `_` 分支 → `SyncStoreHealth::PausedAfterRepeatedFailures`，UI 显示"多次失败后已暂停"

- **无法在旧版侧修复**：旧版已经发布，只能靠"默认不开"避免制造这种记录。可选的改善（**不能解决问题，只改善可诊断性**）：把 manifest 的 `schema_version` 在启用新类型时升到 2，旧版的 `PersonalSyncManifest::validate()`（`personal/models.rs:18-26`）会返回 `SchemaUnsupported`，给出"同步包版本过高"的清晰提示而不是莫名其妙的 Parse error —— 但结果**仍然是同步失败**，所以这只是把"静默的半吊子失败"变成"明确的失败"，是否要做见 Q1。

**首次开启新类型时的提示**：因为 R2 只作用于 Personal 通道，提示应**按当前同步方式动态显示** —— 仅当 `sync_provider == Personal` 时才提示"开启后，未升级到本版本的客户端将无法完成同步"；OnetCloud 下不提示。

### 5.2 载荷向前兼容

- 所有新 `*PlainData` 的新增字段必须 `#[serde(default)]`
- `format_version` 从 1 开始；解密时若 `format_version > 当前支持` → 记 warning 跳过该条（**不要**整轮失败），照 `WorkspaceParentLink::from_payload` 的三态处理思路
- 「本轮遇到无法处理的记录」要在同步状态里体现（例如 `SyncStoreStatus::ready_with_message`），别静默

### 5.3 降级兼容

**仅 Personal 通道**（OnetCloud 按类型拉取，不存在此问题）：

用户从新版退回旧版、或同一同步包上有未升级的设备时：
- 旧版不认识新类型 → 全量拉取会拿到新类型记录 → 整轮报错（R2，链路见 5.2）
- **无法在旧版侧修复**，只能靠"默认关闭"避免制造这种记录

可选的改善（**不解决问题，只改善可诊断性**，见 Q1）：启用新类型时把 manifest 的 `schema_version` 升到 2，旧版的 `PersonalSyncManifest::validate()`（`personal/models.rs:18-26`）会返回 `SchemaUnsupported` → `SyncStoreHealth::SchemaUnsupported`，UI 给出"同步包版本过高"的明确提示，而不是莫名其妙的 `Parse error` / "多次失败后已暂停"。代价是**结果仍然是同步失败**，且一旦升级 manifest，旧版连原有的 connection 同步也一起停（本来是"半吊子失败"，变成"明确全停"）。

我自己倾向于**不做**这个 manifest 升级：它把局部失败放大成全量失败，且收益只是文案更清楚。更值得做的是在新版里把未知类型改成 warning 跳过（`local_source.rs:385`），为**未来**的版本演进铺路。

---

## 6. 模块五：性能影响

### 6.1 每轮 pass 的成本增量

| 阶段 | 增量 | 评估 |
|---|---|---|
| `list_local` | 14 个类型各扫一次（6 个 Singleton 只是读 `AppSettings` 序列化） | 小，但**必须缓存**：Singleton 的 checksum 只在 `AppSettings::update_and_save` 时变，可用 dirty 标记避免每轮重算 |
| state 表读取 | 一次全表读（几百行） | 可忽略 |
| 加密/解密 | 每条记录一次 AES-GCM | 与记录数线性相关，见 6.2 |
| 网络 | OnetCloud 每记录 1 次 REST；Personal WebDAV 每记录 1 次 PUT | **主要瓶颈** |

### 6.2 笔记是唯一可能失控的类型（Q4 已定：每篇一条记录、单篇上限 1 MB、Git 后端也同步）

粒度与上限**已确认**：每篇文档一条记录；单篇正文 **> 1 MB 拒绝同步**（记 error、在状态列显示"单篇超过 1 MB"，**不重试**）；**Git 后端同样同步笔记**（原方案"Git 先不同步"的建议已按 Q4 否决）。

成本估算：
- 首次同步：数百次 REST + 数百次 AES-GCM → 数十秒级
- 后续：靠 checksum 跳过未变的，成本回到线性于"变更篇数"

**缓解措施**：
1. **单篇上限 1 MB**（Q4 已定）—— 超限跳过并提示
2. **总量软上限**：单一账号笔记记录数 > 500 时给出提示（不阻断）
3. **增量靠 checksum**：已在 4.1 覆盖，未变动的笔记不产生网络请求
4. **并发上限**：Personal WebDAV 后端的 PUT 并发限制在 4~8（现状 WebDAV store 的并发度需确认）
5. **大文件单独提示**：接近上限（如 > 800 KB）时在状态列给出警告，让用户提前知道

### 6.3 Personal 后端的额外影响（含 Git 同步笔记的新增风险）

- **Git 后端 —— 现在需要正面处理**（Q4 决定同步笔记）：
  - 每篇笔记 = `records/note/<uuid>.json` 一个文件，几百篇 = 几百个文件
  - 每次 commit 的变更集会显著变大；`git_store.rs` 目前是 `directory.list_records` 的委托 + 自动 push，无分批
  - **缓解**：① 笔记正文与索引元数据同文件（不额外拆文件，避免文件数翻倍）；② 单轮 commit 的记录数设上限，超出则分多次 commit；③ `git.auto_push` 默认开启，大批量提交建议合并为单次 commit 后再 push
  - **建议加一条专项手动测试**：Git 后端 + 200 篇笔记的首轮同步耗时与 commit 体积
- **目录后端**：文件数增加，目录扫描成本线性增长
- **锁 TTL**：pass 变长可能超过现有锁 TTL → 需确认/调大（`SyncStoreLock` 依赖 TTL 自愈）。Git 后端同步笔记后这一项**必须实测**

### 6.4 设置类 Singleton 的收益

6 个 Singleton 每轮最多 6 条记录，成本几乎为零 —— 这是把它们设计成 Singleton 的额外好处。

---

## 7. 模块六：失败重试与错误处理

### 7.1 复用现有机制

| 通道 | 机制 | 新类型 |
|---|---|---|
| OnetCloud | `SyncOperationQueue`（按 `queue_key` 持久化 + `retry_failed()` + `mark_failed`） | 分配新 `queue_key` 即可自动获得 |
| Personal | `personal_sync_retry_queue` 表 + `SyncStoreHealth::PausedAfterRepeatedFailures` | 自动获得 |

`generic_sync.rs:177-330` 的循环已经是"逐条 dequeue + 单条失败 `mark_failed` 不阻断整轮"，新类型沿用即可。

### 7.2 需要新增的错误分类（区分"可重试"与"不可重试"）

| 错误 | 是否重试 | 处理 |
|---|---|---|
| 网络超时 / 5xx | ✅ 重试 | 现有队列机制 |
| 版本冲突（`version` 不匹配，乐观并发） | ✅ 重试（重新拉取后重算） | 需确认 `update_sync_data` 的冲突返回是否已被识别 |
| **解密失败**（`key_version` 不匹配 / 主密钥错误） | ❌ **不重试** | 单独归类 `SyncError::KeyMismatch`，提示用户检查主密钥；重试只会反复失败 |
| 笔记正文超限 | ❌ 不重试 | 跳过该条，记 error，在同步结果里体现 |
| 未知 `data_type` | ❌ 不重试 | warning 跳过（见 R2） |
| 未勾选的类型 | N/A | 静默跳过，不算成功也不算失败 |
| 笔记/已保存 SQL 文件被外部占用（Windows 文件锁） | ✅ 重试 | Windows 上常见，需专门处理 |
| **主密钥未设置 / 未解锁** | ❌ **不重试，且整轮阻断** | 见 7.5（Q11 决策） |

### 7.3 用户可见性（Q12 已定：状态展示到每一行）

- 单条失败要在同步结果里能定位到「类型 + 条目名」，不能只报总数
- **每个勾选行右侧显示状态**（Q12 决策，实现见 2.6）：
  - 成功 → `上次同步 N 分钟前`
  - 失败 → `⚠ <错误摘要>`（hover 看完整错误，含条目名）
  - 从未同步 → `未同步`
- 错误码建议：`key_mismatch` / `network` / `size_limit` / `file_locked` / `unknown_type` / `decrypt_failed`

### 7.4 取消勾选时的清理

按 D3 / Q2（**云端数据保留、不删**）。需要：
- state 表**保留**（不删），否则重新勾选会重复创建
- 该类型的 pending deletion **停止处理**（避免把别的设备还在用的记录删掉）
- 重新勾选后按 Q2 语义恢复：`cloud_id` + `checksum` 基线仍在 → planner 判定"无变化 → mark_synced"，**不会重复创建**

### 7.5 主密钥未解锁 = 整轮阻断（Q11 决策）

Q11 明确：**主密钥未设置 / 未解锁时，无法加密 → 无法同步任何数据到云端**。这不是"新类型跳过、老类型照常"，而是**全部类型一起停**。

现状行为本来就如此（`CloudSyncService::select_encrypt_key(None)` 在 `master_key` 为 `None` 时返回 `Err(NotUnlocked)`），所以**不需要改同步引擎**，只需要把 UI 表现做对：

1. 设置页 Sync 分组**顶部**显示一条明确提示：「未设置主密钥，云同步已停止。设置主密钥后才能同步任何数据。」+ 跳转主密钥设置的按钮
2. **每一行**的状态列统一显示 `⚠ 未同步：主密钥未解锁`（Q11 + Q12 的交集）
3. 不进入 `PausedAfterRepeatedFailures` 状态 —— 那是"重试多次仍失败"的语义，主密钥缺失是**配置问题**，不是故障。建议新增 `SyncStoreHealth::MasterKeyLocked` 或复用现有的提示通路
4. 主密钥解锁后立即触发一次 FullScan

> ⚠️ 实现时要确认：`crypto::has_master_key()` 的判断要在 UI 侧和同步侧用同一个口径，避免"UI 说未解锁、引擎却在跑"。

按 D3，取消勾选**不删云端**。但需要：
- state 表**保留**（不删），否则重新勾选会重复创建
- 该类型的 pending deletion **停止处理**（避免把别的设备还在用的记录删掉）

---

## 8. 模块七：验证与测试要点

仓库已有强测试惯例（`*_tests.rs` 与源文件同目录 + `#[path]` 挂载，如 `planner.rs` 内联 tests、`local_source_tests.rs`）。新增测试按同样组织。

### 8.1 单元测试（必须）

**Planner 层（纯函数，最优先）** —— 扩展 `planner.rs` 内联 tests：
- checksum 相等 → `to_mark_synced`（**这条是 Singleton 设置类不需要 `updated_at` 的依据，必须覆盖**）
- 仅本地变 → `to_update_cloud`；仅远端变 → `to_update_local`；双边变 → `BothModified`
- 未勾选类型不出现在任何计划分支
- `personal_only` 类型在 OnetCloud provider 下被剔除

**Scope 配置层**：
- `SyncScopeSettings::effective(OnetCloud)` 不含 `credential` / `llm_provider`
- `force_disable_personal_only()` 幂等
- `schema_version = 0` 的旧 settings.json 反序列化 → 默认值正确（连接/工作空间/钥匙串 = true，其余 = false）
- 缺失部分字段的 JSON → `#[serde(default)]` 兜底

**加密 round-trip（每类型一个）**：
- `encrypt → decrypt` 恒等（含 `LlmProviderPlainData.api_key`）
- 不同 `key_version` 下解密失败被正确归类为 `KeyMismatch` 而非"数据损坏"

**字段级三方合并（Q8，纯函数，最优先）**：
- 仅本地变 → 取本地；仅远端变 → 取远端；两边同变同值 → 取该值
- 两边都变且不同 → 该字段进冲突集，其余字段照常合并
- 冲突集为空 → 返回"自动合并成功"；非空 → 返回冲突字段列表（含本地值 / 远端值）
- base 缺失 → **降级为二选一**，绝不静默丢数据（R14）
- `custom_keybindings` 按 `action_id` 逐项合并：A 改 `app.quit`、B 改 `sql.run_query` → 两者都保留

**local_source 扩展**：
- 每个新前缀的 `export → apply_remote` 幂等（同一条 apply 两次不产生重复）
- 本地 ID → cloud_id 转换：拿不到 cloud_id 时 SFTP 收藏被跳过、快捷命令 `connection_cloud_id = None`
- 未知 `data_type` 的 `apply_remote` 返回 `Ok(())`（**R2 的回归保护**）

**删除传播**：
- 删除快捷命令 → pending deletion 记录了正确的 cloud_id
- 云端确认删除后 state 行被 GC

### 8.2 迁移专项测试（内置默认污染，R1）

- `ALTER TABLE quick_commands ADD COLUMN is_builtin` 后，30 条内置命令被正确回填（`is_builtin = 1`）
- 迁移**幂等**：重复执行不会把用户自建命令误标为内置，也不会重复插入
- 只同步 `is_builtin = 0` 的行：两台全新设备互相同步后，快捷命令总数**不增加**（这是最关键的回归断言）

### 8.3 兼容性测试

- 旧 payload（`format_version` 缺失 / 字段缺失）能被正确反序列化
- `format_version` 高于支持值 → 跳过该条且**整轮不失败**

### 8.4 手动 E2E（必须，无法自动化替代）

用两个 profile / 两台设备，覆盖：

1. 全新安装 → 全部新类型默认关 → 逐个开启 → 双向同步一致
2. 存量升级（构造含 `cloud_id` 的 connection/workspace）→ 三旧类型仍为开且同步正常（**无回归**）
3. 勾选「快捷命令」→ A 机加一条 → B 机出现 → A 机删除 → B 机消失
4. A 机改主题、B 机改字号（设置类）→ 双向同步后**两者都保留**（**Q8 字段级三方合并**的核心验收）
5. A 机改主题、B 机改主题为不同值 → 弹出冲突、列出冲突字段与两边的值 → 选一边 → 结果正确
6. 取消勾选「笔记」→ A 机改笔记 → B 机不变 → 重新勾选 → **不产生重复**（Q2 核心验收）
7. 切到 OnetCloud → 钥匙串 / LLM 提供商置灰且已被取消勾选 → 切回 Personal → 恢复可选（勾选状态为 false，不复活）
8. Personal 三种后端（Folder / Git / WebDAV）各跑一遍 1-6
9. 断网 / WebDAV 401 / 主密钥错误 三种失败路径的提示与恢复
10. **主密钥未解锁**（Q11）：顶部提示 + 每行显示「未同步：主密钥未解锁」→ **确认连接/工作空间也没有同步** → 解锁后自动触发一次 FullScan
11. **Git + 200 篇笔记首轮同步**（Q4 / R11 / R13）：记录耗时、commit 体积、是否触发锁 TTL 过期
12. 单篇 > 1 MB 的笔记 → 被跳过 + 状态列显示"单篇超过 1 MB" + **不重试**
13. **状态列回显**（Q12）：同步成功后每行的"上次同步时间"更新；制造一次失败后该行显示 `⚠ <摘要>`，hover 能看到条目名
14. **Personal 下首次开启确认弹窗**（Q1）：首次开启弹出且文案正确 → 勾选"不再提示"后再开启其他类型不弹 → OnetCloud 下不弹

### 8.5 Review 检查项

- 所有含密钥的 `*PlainData` 未派生 `Debug`
- 无 `tracing::info!("{:?}", plain_data)` 泄露
- `SyncScopeSettings` 自身不在任何设置类同步白名单内（R7 / Q9）
- 设置白名单逐字段审过，不含设备相关项（R8）
- `settings_mcp` 与 `settings_agent` 的工具暴露字段**无重叠**（R16）
- 终端 `custom_highlights` 的剥离逻辑不会误删用户规则（R15）
- 主密钥未解锁走的是"配置未就绪"通路，**不是** `PausedAfterRepeatedFailures`（Q11）

---

## 9. 实施分期建议

决策已全部确定（见第 11 节），分期如下。注意 **R1 的迁移必须在 P1 之前或随 P1 一起发布**，否则快捷命令同步一上线就会污染。

| 期 | 范围 | 关键依赖 / 理由 |
|---|---|---|
| **P0** | **R1 迁移**：`quick_commands` 加 `is_builtin` 列 + 回填（Q13 规则） | **必须先于任何快捷命令同步上线**；单独发布、单独验证幂等 |
| **P1** | scope 配置数据结构 + 持久化 + 设置页 UI（含 **Q12 每行状态列**、`LockMode` 置灰）+ 勾选过滤 + **Q11 主密钥未解锁整轮阻断的 UI**；同步侧先只接 `settings_keybindings` 和 `quick_command` | 跑通"配置 → 过滤 → 同步 → 状态回显"全链路，验证 D1/D2/D3。快捷键天然只含用户自定义、快捷命令有 `updated_at`，是最简单的两个 |
| **P2** | `sftp_favorite` + `saved_sql` + `llm_provider` | 需要处理本地 ID → cloud_id 转换（R5/R6）。`llm_provider` 含 api_key，注意不派生 `Debug` |
| **P3** | 6 个设置类 Singleton + **Q8 字段级三方合并**（含 `base_payload` 列与降级路径 R14）+ **Q6 终端高亮剥离**（R15）+ **Q7 工具暴露归属**（R16） | 逐字段审定白名单（R8）+ 合并算法，工作量大但风险低 |
| **P4** | `note` | 难度最高（文件系统树形结构 + 正文冲突副本 + 1 MB 上限 + **R11 Git 后端文件膨胀**），放最后 |

**跨期前置项**（建议 P1 就做，避免后面返工）：
- `sync_item_state` + `sync_type_status` 两张表（D2 + 2.6）
- `PendingCloudDeletionRepository` 通用化（R12）
- 未知 `data_type` 改为 warning 跳过（R2 的未来兼容部分）
- 确认主密钥轮换入口是全量拉取（R3）

每期独立可发布，**新增 11 项默认全关**（Q1），逐步放开。

---

## 10. 风险清单

| ID | 级别 | 风险 | 说明 | 缓解 |
|---|---|---|---|---|
| **R1** | **P0** | **快捷命令内置默认污染** | `20260720000001_default_quick_commands.sql` 把 30 条内置命令直接 INSERT 进业务表且**无 `is_builtin` 列**。整表同步会让每台设备把各自的默认副本推上去再拉下来 → **"同步一次多 30 条"** 的指数级重复 | 必须先加 `is_builtin` 列并回填，只同步 `is_builtin = 0` 的行。回填规则复用现有 SQL 里的 `LOWER(TRIM(command))` 比对逻辑 |
| **R2** | **P0（仅 Personal 通道）** | **旧客户端遇到未知 `data_type` 会整轮失败** | **只影响 Personal 通道**：`worker.rs:222` 是 `list_records(None, None)` **全量拉取**（目录/Git/WebDAV 三后端都不做类型白名单过滤），旧版会拉到新类型记录 → `planner.rs:98-107` 无条件进 `to_download` → `worker.rs:433-438` 的 `apply_remote` 用 `?` → `local_source.rs:385-388` 对未知类型返回 `Err(Parse)` → **整轮 pass 失败**。连带：`apply_downloads` 循环中断（后续下载全跳过，含别的新增连接）、`apply_synced_marks` 与 `apply_conflicts` 不执行（`last_synced_at` 不更新、新冲突不挂起）、`health_from_error` 把 `Parse` 落到 `_` 分支 → UI 显示"多次失败后已暂停"。<br>**OnetCloud 通道不受影响**：`generic_sync.rs:45` 是 `list_sync_data(Some(data_type))` **按类型过滤拉取**，旧版只会请求 connection/workspace，根本看不到新类型，原有同步照常 | ① Personal 通道新类型默认全关 + 首次开启时提示；② 把未知类型改为 warning + 跳过（**对已发布旧版无效**，只对未来版本间兼容有价值）；③ OnetCloud 通道可以放心默认开 |
| **R3** | P1 | **主密钥轮换可能不覆盖新类型** | 若轮换入口是按 `data_type` 分别拉取而非全量，新类型轮换后会永久解密失败 | 实现前先确认轮换入口是 `data_type = None`；若不是，补齐类型列表。**必须验证** |
| **R4** | P1 | **笔记 `updated_at` 不随正文更新** | `document_index.rs` 的 `updated_at` 只在新建/重命名时刷新，改正文不刷新 → 改了内容检测不到 | 笔记同步改用「正文 checksum」而非 `updated_at` 作为变更判据（走 D2 的 state 表已有 content/checksum，天然满足）；或给笔记加落盘钩子刷新 `updated_at` |
| **R5** | P1 | **SFTP 收藏 `connection_key` 是设备本地键** | 默认 `local:{id}`，本地整数 ID 在目标设备无意义 → 跨设备错绑到同 ID 的其他连接 | 转 `connection_cloud_id`；查不到则跳过该条上传 |
| **R6** | P1 | **已保存 SQL 无稳定 ID、路径含本地 `connection_id`** | 跨设备连接 ID 不同 → 路径无法直接复用 | 用复合键 `(db_type, connection_cloud_id, database, filename)`；上传时替换路径中的 `connection_id` 段 |
| **R7** | P1 | **同步范围配置自身被同步 → 回环** | 若 `SyncScopeSettings` 落入"通用设置"同步白名单，A 机的勾选会覆盖 B 机，且 provider 切换的强制取消逻辑会互相打架 | 白名单**显式排除** `sync_scope`；加单测断言 |
| **R8** | P1 | **设置同步会带走设备相关的本地偏好** | 通用设置里混着窗口尺寸、UI 缩放、自定义字体路径、终端 profile 路径、启动页等**不该跨设备同步**的项 | 设置类同步必须走**字段白名单**（不是整包序列化 `AppSettings`），逐个字段审定。这是 P3 的主要工作量 |
| **R9** | P2（**Q10 已定：本次不修**） | **LLM api_key 本地 SQLite 明文存储** | 现状 `llm_providers.api_key` 明文落库（`init.sql:60`）。同步本身会用主密钥加密上传（E2E 安全），但**本地明文问题依旧** | **已知不修，本次不做本地加密**。记录下来，后续单开一项 |
| **R10** | P2 | **OnetCloud 侧无冲突挂起机制** | Personal 有 `personal_sync_conflicts` 表 + UI，OnetCloud 只有即时二选一弹窗。新类型走 OnetCloud 时冲突体验不一致 | 本方案不扩展 OnetCloud 冲突机制（属引擎重构）；接受体验差异，并在文档说明 |
| **R11** | **P1（升级：Q4 决定 Git 也同步笔记）** | **Git 后端文件数膨胀** | 每篇笔记 = `records/note/<uuid>.json`，数百篇 = 数百个文件，每次 commit 变更集很大；`git_store.rs` 目前无分批 | ① 正文与元数据同文件（不拆）；② 单轮 commit 记录数上限，超出分批 commit；③ 大批量合并为单次 push。**必须做 Git + 200 篇笔记的专项实测**（见 8.4） |
| **R12** | P2 | **`PendingCloudDeletionRepository` 只支持两种 entity_type** | `sync_type.rs:180-185` 是硬编码 if/else | 扩展为通用 `entity_type` 过滤 |
| **R13** | **P1（升级：Git 同步笔记后更可能触发）** | **Personal 锁 TTL vs pass 变长** | 新类型（尤其笔记）让单轮 pass 显著变长，可能超出现有锁 TTL，导致锁被误判过期、两实例并发写 | 实现时确认 TTL；长 pass 考虑分类型分批持锁。**Git + 笔记场景下必须实测** |
| **R14** | P1（新增） | **字段级三方合并的 base 丢失** | Q8 的三方合并依赖 `base_payload`。若用户清了本地 DB、或 state 行被 GC、或跨通道切换，base 会缺失 → 三方合并退化 | base 缺失时**降级为二选一**（并在冲突弹窗说明"无法自动合并"），绝不静默丢数据 |
| **R15** | P2（新增） | **终端 `custom_highlights` 内置规则剥离** | Q6 要求只同步"与内置规则不同的部分"，但 `TerminalLocalSettings::default()` 把 `builtin_highlight_rules()` 直接塞进了 `custom_highlights`（`crates/terminal_view/src/settings.rs:142-150`），无法靠字段区分 | 按 `TerminalHighlightRule.id` 与 `builtin_highlight_rules()` 比对剥离；内置规则的 `id` 必须是稳定常量，否则剥离会误删用户规则。**实现前先确认内置规则 id 的稳定性** |
| **R16** | P2（新增） | **MCP/工具暴露白名单边界** | Q7 决定「工具暴露」纳入同步，但 `ToolExposureSettings` 分 `mcp` / `agent` 两个 toolset，且默认值不同（`public_mcp_default()` 保守 / `agent_default()` 全开） | `settings_mcp` 含 mcp toolset，`settings_agent` 含 agent toolset。**必须在白名单里逐字段写明归属**，避免两边重复同步同一字段 |

---

## 11. 决策记录（14 项已全部确认，2026-10-14）

> 本节已由"待决策"转为"决策归档"。每条记录：**结论 → 落地位置 → 派生的影响**。

| # | 决策 | 落地位置 | 派生影响 |
|---|---|---|---|
| **Q1** | **统一默认全关**（两个通道都关）；**Personal 下首次开启弹确认** | 5.1 | OnetCloud 下不弹确认（该通道无 R2）。需要 `ever_enabled` 标记 + "不再提示"开关 |
| **Q2** | 取消勾选后**云端数据保留、不删**；重新勾选可恢复 | D3 / 7.4 | state 表不清空 → 重新勾选不重复创建；该类型 pending deletion 停止处理 |
| **Q3** | 新增类型**一律仅个人**（`team_id = None`）；团队仍只同步连接/工作空间 | 3.1 | 新类型全部走主密钥分支；团队密钥路径不涉及，回归面缩小 |
| **Q4** | 笔记**每篇文档一条记录**；单篇上限 **1 MB**；**Git 后端也同步** | 6.2 / 6.3 | 原"Git 先不同步"建议作废 → **R11 从 P2 升到 P1**，必须做 Git + 200 篇笔记专项实测；R13 锁 TTL 同样升级 |
| **Q5** | 已保存 SQL **只同步默认目录 `config/queries/`**，不含 `query-directories.json` 挂载的外部目录；**不含 SQL 执行历史** | 3.2 / 4.3 | 复合键用 `(db_type, connection_cloud_id, database, filename)`；外部目录条目不进 `list_items` |
| **Q6** | `settings_terminal` = `settings.json` 的 `terminal_*` 字段 **+** `terminal-settings.json` 的 `custom_highlights`，但**只同步与内置规则不同的部分** | 3.2 / R15 | 新增风险 **R15**：需先确认内置高亮规则 `id` 的稳定性才能剥离 |
| **Q7** | MCP 设置**包含「工具暴露」** | 3.2 / R16 | `settings_mcp` = `McpSettings` + mcp toolset；`settings_agent` = `AiChatSettings` + agent toolset。新增风险 **R16**：白名单需逐字段写明归属，避免重复同步 |
| **Q8** | 设置类冲突**做字段级三方合并** | 4.4 | `sync_item_state` 加 `base_payload` 列；新增风险 **R14**：base 缺失时降级为二选一。两条通道计划算法的统一仍**不在本次范围** |
| **Q9** | 「同步范围」配置**本身不跨设备同步** | R7 | 白名单显式排除 `sync_scope`，加单测断言 |
| **Q10** | LLM api_key 本地加密**暂时不做** | R9 | R9 标记为"已知不修"，后续单开一项。同步侧 E2E 加密仍然做 |
| **Q11** | 主密钥未设置/未解锁 → **无法同步任何数据**（不是只跳新类型） | 7.5 | 需新增"主密钥未解锁"的明确 UI 通路，**不要**走 `PausedAfterRepeatedFailures`（那是故障语义，不是配置语义）。与 Q12 的交集：每行状态列统一显示 `⚠ 未同步：主密钥未解锁` |
| **Q12** | 每个勾选行**和**置灰行**都显示状态**：成功显示上次同步时间，失败显示最近失败警告 | 2.5 / 2.6 / 7.3 | 新增 `sync_type_status` 表；用 `SettingItem::render` 自定义渲染（状态要右对齐 + 带颜色，description 做不到） |
| **Q13** | `is_builtin` 回填**复用现有迁移 SQL 的判定逻辑**（`LOWER(TRIM(command))` 比对） | R1 / 8.2 | 接受"用户自建了与内置同文本的命令会被误标为内置"这一代价。迁移必须幂等 |
| **Q14** | `data_type` 命名**确认**：`note` / `saved_sql` / `quick_command` / `sftp_favorite` / `settings_general` / `settings_database` / `settings_terminal` / `settings_agent` / `settings_mcp` / `settings_keybindings` / `llm_provider` | 1 / 2.1 | **已冻结**，上线后不可改 |

### 11.1 仍需你确认的一处理解偏差

Q12 里"默认勾选且置灰的默认同步项"，我理解为：

- **连接 / 工作空间** = `LockMode::AlwaysOn`（默认勾选、**置灰不可取消**）
- **钥匙串** = `LockMode::PersonalOnly`（Personal 下可取消；OnetCloud 下强制取消 + 置灰）

若你的原意是"钥匙串也始终置灰不可取消"，或反过来"连接/工作空间也允许取消"，请指出 —— 这只改 `LockMode` 一行常量。

### 11.2 决策带来的实施顺序变化

Q4（Git 同步笔记）和 Q8（三方合并）把原本的低风险项抬高了，因此分期调整为：

```
P0  R1 迁移（is_builtin 列 + 回填）        ← 必须先于快捷命令同步上线
P1  配置 + UI + 状态列 + 主密钥阻断 + 先接 keybindings / quick_command
P2  sftp_favorite / saved_sql / llm_provider
P3  6 个设置类 Singleton + 三方合并 + 高亮剥离 + 工具暴露归属
P4  note（含 Git 后端专项实测）
```

详见第 9 节。

---

## 附：关键文件索引

| 用途 | 文件 |
|---|---|
| 数据类型常量 | `crates/core/src/cloud_sync/models.rs:238-242` |
| 统一信封 | `crates/core/src/cloud_sync/models.rs:206-231` |
| 加解密服务 | `crates/core/src/cloud_sync/service.rs:311-336`（blob）、`:436-474`（credential 范式） |
| OnetCloud 类型抽象 | `crates/core/src/cloud_sync/sync_type.rs:16-206` |
| OnetCloud 通用流程 | `crates/core/src/cloud_sync/generic_sync.rs` |
| OnetCloud 注册点 | `crates/core/src/cloud_sync/engine.rs:100-108` |
| Personal 计划器 | `crates/core/src/cloud_sync/personal/planner.rs:137-166` |
| Personal 本地源（分派点） | `crates/core/src/cloud_sync/personal/local_source.rs:18-20, 331-470` |
| Personal worker（全量拉取点） | `crates/core/src/cloud_sync/personal/worker.rs:222` |
| Personal 包布局 | `crates/core/src/cloud_sync/personal/file_format.rs` |
| 同步设置数据模型 | `crates/core/src/settings.rs:729-808`（SyncProvider / PersonalSyncSettings）、`:1104-1112`（AppSettings 挂载点） |
| 设置页 | `main/src/setting_tab.rs:65-66`（页索引）、`:1078-1141`（sync_setting_group）、`:1159-1187`（provider 下拉） |
| checkbox 组参考实现 | `main/src/settings/tool_exposure_settings.rs:177-206` |
| gpui-kit 禁用能力 | `.../gpui-kit/crates/component/src/setting/item.rs:131`（`SettingItem::disabled`）、`fields/bool.rs:38-55`（转发到 Checkbox） |
| i18n | `main/locales/main.yml` `Settings.Sync:` 段（`:3521` 起） |
| 迁移目录 | `crates/core/migrations/`，注册在 `crates/core/src/storage/migration.rs:4-122` |
| 待删除仓储（需扩展） | `crates/core/src/storage/repository.rs:883`、`cloud_sync/sync_type.rs:180-185` |
