# NavOp 对象化数据库备份格式与恢复架构设计

| 项目 | 内容 |
| --- | --- |
| 状态 | Proposed（提案） |
| 日期 | 2026-07-26 |
| 适用范围 | NavOp 数据库备份、恢复、增量备份与备份存储集成 |
| 公开格式工作名 | ODBF（Open Database Backup Format） |
| 目标数据库 | MySQL、PostgreSQL、SQL Server、SQLite；可扩展到其它关系型和非关系型数据库 |
| 读者 | `db`、`db_view`、extension/driver、CLI、云存储与安全实现维护者 |

> 本文是设计方案，不代表当前仓库已经实现了 ODBF、Parquet 写入、原生日志解析或点时间恢复。文中以“当前能力”标注已经存在的 NavOp 合同，以“建议新增”标注需要实现的能力。

## 摘要

NavOp 的备份应采用公开、可移植、对象化的 **ODBF**，而不是把所有内容拼成一个不可寻址的大 SQL 文件。一个备份集由一个 `manifest.json`、若干对象元数据文件、表数据 artifact、可选日志 artifact、校验清单和可选签名/加密信息组成。对象之间通过稳定身份和依赖边连接，恢复由依赖图驱动，并按“结构—数据—约束—程序对象—校验”的阶段执行。

ODBF 的规范真相是目录树；ZIP、TAR、Zstd archive 或对象存储只是传输封装。任何第三方只要获得 `manifest.json` 和对象文件，就可以在不依赖 NavOp 私有协议的前提下解析、校验和实现恢复。数据库特有能力（例如 binlog、WAL、transaction log）只作为显式声明的 adapter 能力，不被伪装成跨数据库通用 SQL。

本文同时定义：

- 公开格式和版本兼容规则；
- Manifest、对象元数据、逻辑类型和 artifact 引用；
- Parquet、Arrow IPC、NDJSON、CSV、SQL 的定位和 fallback；
- 快照一致性、全量备份、恢复阶段和依赖循环处理；
- 原生日志增量与 Snapshot + Diff 的边界；
- Rust crate、`BackupManager`、`RestoreManager` 和 driver/IPC 扩展接口；
- 校验、压缩、加密、取消、断点续作、云同步和版本管理；
- 首版范围、验收标准和未决问题。

## 1. 背景与问题

数据库客户端通常同时承担连接管理、元数据浏览、数据导入导出和备份恢复。若备份只产生一个 SQL 文件，会出现以下问题：

1. 大表无法并行读取、上传、校验或部分恢复；
2. 二进制、精确小数、时区、数组、空间类型等容易被文本化而丢失语义；
3. 只恢复一个表或一个视图时必须重新扫描整个文件；
4. 对象依赖、权限、程序体、索引和外键的顺序无法可靠表达；
5. 增量备份只能通过字符串 diff，无法表示删除、schema 变更或日志位置；
6. 私有封装会阻碍跨平台、云同步、长期归档和第三方恢复工具。

NavOp 当前的 `crates/db` 已有 `DatabasePlugin`、`DbConnection`、schema 元数据查询、DDL 构造以及 `data/export`、`data/import_*` 流式能力；`extension-protocol` 也已有 row IR、Parquet 枚举、blob/stream/event 资源合同。这些能力适合作为 adapter 和传输基础，但还不足以描述一个完整的备份集：它们没有统一的快照生命周期、对象身份、依赖 DAG、校验/签名、增量日志、恢复阶段或跨数据库逻辑类型。

因此，备份编排不应继续把所有逻辑塞入不断膨胀的 `DatabasePlugin` trait，也不应把备份格式绑定到某个 driver 的私有 wire contract。需要增加一个独立的、可版本化的备份领域层。

## 2. 目标与非目标

### 2.1 目标

- 定义公开且可独立实现的 ODBF 1.x 规范；
- 以对象为单位保存元数据、数据和依赖关系；
- 支持 MySQL、PostgreSQL、SQL Server、SQLite 的全量快照，并为更多数据库预留 adapter；
- 默认采用 Parquet 保存大表数据，保留 Arrow IPC/NDJSON 等可流式 fallback；
- 能恢复单个对象、一个 schema 或整个数据库，并提供 dry-run/plan；
- 将原生日志增量、Snapshot + Diff、元数据增量明确区分；
- 在无稳定行身份或无日志权限时诚实降级，不宣称不可靠的任意点恢复；
- 让备份过程有界、可取消、可重试、可断点续作，并且不会发布半成品；
- 提供文件级、chunk 级、对象级和备份级完整性验证；
- 支持 zstd/gzip 压缩、AEAD 加密、可选签名和密钥轮换；
- 支持本地目录、SFTP、WebDAV、S3-compatible 等云同步后端；
- 让 UI、CLI、Public MCP 和未来的自动化 agent 共用同一个 manager contract。

### 2.2 非目标

- 首版不实现所有数据库的原生日志解析器；
- 首版不把 SQL 文本作为唯一或默认数据格式；
- 不承诺不同数据库之间的 schema 自动迁移完全无损；
- 不在没有权限、日志保留或一致性保证时静默伪造增量能力；
- 不把用户密码、token、私钥或完整连接串写入 Manifest、日志或错误上下文；
- 不把 GPUI 组件直接放入核心备份 crate；
- 不要求把所有现有导入导出 API 一次性重写；
- 不把云服务的对象命名、认证协议或版本索引作为 ODBF 格式真相；
- 不在首版自动恢复 users/roles/grants，除非用户显式选择并且目标 adapter 声明支持。

## 3. 设计原则与规范用语

本文使用以下规范词：

- **MUST**：格式或实现必须满足，否则不符合 ODBF；
- **SHOULD**：默认应满足，若有明确理由可以偏离，但必须记录原因；
- **MAY**：可选能力；
- **当前能力**：截至本文日期仓库中已有的接口或行为，不代表已支持 ODBF；
- **建议新增**：需要通过后续实现、协议评审和测试落地的能力。

核心原则：

1. **公开优先**：文件布局、字段语义、类型映射和错误行为都公开；不依赖 NavOp 私有恢复协议。
2. **可寻址**：每个对象和 artifact 都可以独立定位、校验、上传、下载和恢复。
3. **语义保真**：物理编码可以变化，但逻辑类型、NULL、精度、时区、二进制和原始定义不得无故丢失。
4. **能力显式**：数据库能力、权限、快照级别和增量覆盖范围必须写入 Manifest；不支持时返回 typed `NotSupported` 或结构化 warning。
5. **失败关闭**：发现 hash 不匹配、日志 gap、路径越界、签名无效或依赖无法满足时，默认停止相关阶段，而不是静默跳过。
6. **流式和有界**：大数据经过有界 chunk、背压和可取消 stream；不要求把整表或整份日志读入内存。
7. **幂等可恢复**：发布采用临时目录和原子提交；恢复采用计划指纹、checkpoint 和明确冲突策略。
8. **分层解耦**：纯格式/校验代码不依赖 GPUI 或具体数据库 SDK；连接 adapter 通过 trait 注入。
9. **最小权限**：备份和恢复分别声明所需权限；默认不触碰权限提升、用户凭证或系统表中的敏感字段。
10. **可演进**：未知字段可保留，未知对象类型可跳过或转为 opaque object，但不能破坏已知对象的校验。

## 4. 术语与公开格式定位

### 4.1 术语

| 术语 | 定义 |
| --- | --- |
| 备份集（backup set） | 一个由 Manifest、对象和 artifact 组成的不可变逻辑快照 |
| 快照（snapshot） | 数据库在一个可描述的一致性边界上的读取会话 |
| 对象（object） | table、view、function、procedure、trigger、event 等数据库实体 |
| artifact | 一个独立文件或流，例如 `meta.json`、Parquet part、blob、WAL 段 |
| `object_id` | 该备份集内的 UUID；只在本次备份集内引用对象 |
| `identity_key` | 尽可能跨备份稳定的对象身份，用于匹配 rename、增量和恢复 |
| native cursor | 数据库原生日志的位点，如 GTID、LSN 或 binlog position |
| restore point | 可恢复的时间、事务位点或日志位点 |
| 规范化 JSON | 按 ODBF 规定的 UTF-8、LF、排序和 canonicalization 规则编码的 JSON |

### 4.2 ODBF 的格式边界

- `format.id` MUST 为 `odbf`；
- `format.version` 使用 `major.minor`，首版为 `1.0`；同一 major 内新增可选字段不得破坏旧 reader；
- reader MUST 拒绝不支持的 major，遇到更高 minor 时可在忽略未知字段的前提下读取；
- 目录树是规范真相。压缩包必须能无损展开为同语义的目录树，不能把 archive 的顺序或文件名编码当作业务语义；
- 文件路径 MUST 使用 `/` 分隔、相对路径、UTF-8、大小写敏感规则由格式统一定义，且禁止绝对路径、`..`、NUL、重复分隔符和隐式符号链接；
- JSON 文件 MUST 使用 UTF-8、LF、无 BOM。需要签名或 hash 的 JSON MUST 使用 RFC 8785（JCS）或 ODBF 指定的等价 canonical JSON 实现；
- vendor 扩展 MUST 位于 `extensions` 或 `x-<vendor>` 命名空间。核心 reader 不得把未知扩展解释成安全敏感动作。

## 5. 整体架构

### 5.1 分层图

```text
┌─────────────────────────────────────────────────────────┐
│ UI / CLI / Public MCP / Agent                          │
└───────────────────────┬─────────────────────────────────┘
                        │ request, plan, progress
┌───────────────────────▼─────────────────────────────────┐
│ BackupManager / RestoreManager                         │
│ policy · snapshot orchestration · phases · checkpoint  │
└───────────────┬───────────────────────┬─────────────────┘
                │                       │
┌───────────────▼──────────────┐ ┌──────▼────────────────┐
│ db-backup-core                │ │ storage backends       │
│ model · format · graph        │ │ local/SFTP/S3/WebDAV   │
│ integrity · crypto · errors   │ │ staging/publish/GC     │
└───────────────┬──────────────┘ └────────────────────────┘
                │ BackupSource / RestoreTarget
┌───────────────▼─────────────────────────────────────────┐
│ Database adapters                                        │
│ PostgreSQL · MySQL · SQL Server · SQLite · external     │
└───────────────┬─────────────────────────────────────────┘
                │ DbConnection / DatabasePlugin / IPC
┌───────────────▼─────────────────────────────────────────┐
│ crates/db · extension-protocol · extension-host          │
│ native driver sidecar（可选）                            │
└─────────────────────────────────────────────────────────┘
```

### 5.2 各层职责

**`db-backup-core`（建议新增）**：只依赖 serde、UUID、hash、IO 抽象和必要的密码学接口；定义 Manifest/metadata/artifact、格式读写、canonical JSON、依赖 DAG、校验、压缩/加密描述和错误模型。该层不能依赖 GPUI、`db_view` 或具体数据库 SDK。

**`db-backup`（建议新增）**：实现 manager、快照会话、数据库 adapter、进度和恢复计划；通过 trait 接收 `DatabasePlugin`/`DbConnection`，避免修改现有 `DatabasePlugin` 以承载所有备份策略。

**现有 `crates/db`（当前能力）**：继续负责连接、schema introspection、DDL 构造、查询和导入导出。备份 adapter 复用这些方法，并在无法表达 snapshot/log/blob 语义时通过 capability 返回缺口。

**`extension-protocol` / `extension-host`（当前能力 + 建议扩展）**：复用 JSON-RPC lifecycle、`schema/*`、`data/export`、`data/import_*`、`stream/*`、`blob/*`、有界 event stream 和 `ProcessRpcSession`；建议增加备份专用 snapshot/log 方法，且保持 chunk 有界、可取消、可关闭。

**Storage backend（建议新增）**：提供 staging、随机读取、原子 publish、multipart 上传、远端 head/list/delete 和垃圾回收，不解释数据库语义。

### 5.3 当前能力与新增能力边界

| 能力 | NavOp 当前可复用部分 | ODBF 需要新增的边界 |
| --- | --- | --- |
| 对象发现 | `DatabasePlugin` 的 schema/object 方法、IPC `schema/*` | 统一对象 envelope、稳定身份、依赖引用、opaque object |
| 数据读取 | `DbConnection::query/execute_streaming`、`data/export`、row IR | snapshot session、逻辑类型 schema、artifact writer、LOB 外置 |
| 数据导入 | `data/import_begin/chunk/commit/abort` | 恢复 phase、目标映射、checkpoint、校验、冲突策略 |
| 流传输 | `stream/*`、`blob/*`、有界 event stream | 每个 artifact 的 hash、压缩/加密、背压和断点 |
| 生命周期 | `extension-host::ProcessRpcSession` | backup capability negotiation、日志 cursor、恢复专用能力 |
| UI | `db_view` 可复用数据库对象和进度模式 | 备份计划预览、警告、点时间和验证报告 |

## 6. ODBF 目录结构

### 6.1 规范目录

```text
backup/                                      # 逻辑根目录（名称可由外层存储决定）
├── manifest.json                            # 必选：备份集描述
├── objects/                                 # 必选：对象目录
│   ├── database/
│   │   └── <object-id>/meta.json
│   ├── schema/
│   │   └── <object-id>/meta.json
│   ├── table/
│   │   └── <object-id>/
│   │       ├── meta.json
│   │       ├── schema.arrow.json            # 逻辑列 schema
│   │       ├── data/
│   │       │   ├── part-00000.parquet
│   │       │   └── part-00001.parquet
│   │       └── blobs/
│   │           └── <sha256>.bin
│   ├── view/<object-id>/meta.json
│   ├── function/<object-id>/meta.json
│   ├── procedure/<object-id>/meta.json
│   ├── trigger/<object-id>/meta.json
│   ├── event/<object-id>/meta.json
│   └── extensions/<vendor>/<kind>/<object-id>/...
├── logs/                                    # 可选：原生日志或逻辑变更流
│   ├── mysql-binlog/
│   ├── postgresql-wal/
│   └── mssql-tlog/
├── checksums.json                            # 必选：文件 hash 清单
├── signature/                                # 可选：签名及公钥标识
│   ├── manifest.sig
│   └── key-info.json
├── encryption/                               # 可选：加密包级元数据（不含密钥）
│   └── envelope.json
└── README.txt                                # 可选：人类可读说明，不参与业务恢复
```

`objects/<kind>/<object-id>/` 是推荐布局；reader MUST 以 Manifest 中的 `meta_path`/artifact path 为准，不能仅根据目录名猜测对象。目录名使用小写规范化 kind，未知 kind 仍可放在 `extensions/` 或 `objects/<opaque-kind>/`。

### 6.2 路径、文件和发布规则

1. 所有 artifact 路径都相对于备份根；解析后必须进行 realpath/规范化检查，拒绝逃逸根目录。
2. 解包器 MUST 默认拒绝符号链接、硬链接、设备文件、稀疏文件声明和超过策略限制的压缩比，以防路径穿越和 zip bomb。
3. 写入先进入同一文件系统的 staging 目录，例如 `.incomplete/<backup-id>/`；只有校验、签名和 `COMMITTED` 标记写完后才原子 rename 到最终路径。
4. 已发布备份集不可原地修改。修复、重加密或追加增量都生成新的 backup id，并通过 `parent` 建立关系。
5. `README.txt` 不得成为机器恢复的必要输入；可缺省、可翻译、可追加。
6. artifact 文件名必须是稳定、可移植的 ASCII 子集；对象的显示名称只存在于 metadata，不直接拼入路径。

## 7. Manifest 设计

### 7.1 示例

以下示例只展示核心字段，UUID 和版本字符串均为示例值，不代表真实连接信息：

```json
{
  "format": {
    "id": "odbf",
    "version": "1.0",
    "min_reader_version": "1.0"
  },
  "backup_id": "018f2e5a-5f6f-7d0e-9d1a-7c8a0b1f2e3d",
  "backup_type": "full",
  "created_at": "2026-07-26T10:20:30Z",
  "source": {
    "database_type": "postgresql",
    "server_version": "16.3",
    "database": "app",
    "schemas": ["public"],
    "server_uuid": "server-instance-id-if-available",
    "tool": "NavOp",
    "tool_version": "0.1.0"
  },
  "scope": {
    "include_objects": ["table", "view", "function", "procedure", "trigger"],
    "exclude_patterns": ["pg_catalog.*"],
    "include_data": true,
    "include_security": false
  },
  "consistency": {
    "mode": "transaction_snapshot",
    "snapshot_id": "driver-defined-snapshot-id",
    "captured_at": "2026-07-26T10:20:31Z",
    "transaction_id": "optional-driver-value",
    "lsn": "optional-native-position",
    "gtid": null,
    "warnings": []
  },
  "capabilities": {
    "transactional_snapshot": true,
    "native_log": true,
    "row_identity": "primary_key",
    "consistent_non_transactional_objects": false
  },
  "parent": null,
  "objects": [
    {
      "object_id": "018f2e5a-5f70-7c00-8a1b-111111111111",
      "identity_key": "postgresql/app/public/table/users",
      "kind": "table",
      "qualified_name": {
        "catalog": null,
        "database": "app",
        "schema": "public",
        "name": "users"
      },
      "meta_path": "objects/table/018f2e5a-5f70-7c00-8a1b-111111111111/meta.json",
      "schema_path": "objects/table/018f2e5a-5f70-7c00-8a1b-111111111111/schema.arrow.json",
      "data_artifacts": ["artifact-users-00000"],
      "dependencies": [],
      "content_hash": "sha256:object-content-hash",
      "status": "complete",
      "warnings": []
    }
  ],
  "artifacts": [
    {
      "artifact_id": "artifact-users-00000",
      "path": "objects/table/018f2e5a-5f70-7c00-8a1b-111111111111/data/part-00000.parquet",
      "kind": "table_data",
      "format": "parquet",
      "logical_schema_hash": "sha256:schema-hash",
      "size_bytes": 123456,
      "row_count": 4096,
      "compression": {"codec": "zstd", "level": 3},
      "digests": {
        "stored": "sha256:exact-file-bytes-hash",
        "content": "sha256:pre-envelope-content-hash"
      },
      "encryption": null
    }
  ],
  "integrity": {
    "algorithm": "sha256",
    "canonicalization": "jcs",
    "checksum_file": "checksums.json",
    "signature_file": "signature/manifest.sig"
  },
  "encryption": null,
  "extensions": {}
}
```

示例中的 hash 是占位值；实现不得生成看似真实但无法验证的 hash。`digests.stored` 始终校验包内路径上的精确字节（加密时即 ciphertext），`digests.content` 校验完成格式编码和 artifact 级压缩、但尚未做 envelope encryption 的字节。对象级 `content_hash` 负责更高层的 metadata/schema/artifact 组合语义。`server_uuid`、`transaction_id`、`lsn`、`gtid` 只在 adapter 能可靠获得时填写。

### 7.2 必选和可选字段

| 字段 | 必选 | 语义 |
| --- | --- | --- |
| `format` | 是 | 格式 id、版本和最低 reader 版本 |
| `backup_id` | 是 | UUID；同一输出目录不可重复 |
| `backup_type` | 是 | `full`、`incremental_log`、`incremental_diff` 或 `metadata_delta` |
| `created_at` | 是 | RFC 3339 UTC 时间 |
| `source` | 是 | 数据库类型、版本、数据库范围和工具信息；不得含凭证 |
| `scope` | 是 | 对象/数据/安全项过滤结果 |
| `consistency` | 是 | 快照级别及 native 位点；未知值必须显式标记 |
| `objects` | 是 | 对象引用清单；空列表仅在明确的 metadata-only 备份中允许 |
| `artifacts` | 是 | 所有 payload 的引用和物理属性 |
| `integrity` | 是 | hash 算法、canonicalization 和清单路径 |
| `parent` | 增量必选 | 父备份、基线和链校验信息 |
| `capabilities` | 是 | 本次实际使用和未使用的能力 |
| `encryption` | 加密时必选 | 算法、KDF、key id 和 AAD 规则，不含秘密 |
| `extensions` | 否 | 命名空间扩展 |

Manifest 中的数组顺序必须稳定：对象按 `kind`、`identity_key`、`object_id` 排序；artifact 按 `path` 排序。reader 不应依赖顺序，但稳定顺序便于复现和签名。

### 7.3 Object 引用

每个 `objects[]` 条目 MUST 包含：

- `object_id`：本备份内唯一 UUID；
- `identity_key`：稳定匹配键；
- `kind`：核心对象类型或扩展类型；
- `qualified_name`：catalog/database/schema/name 分量，而不是拼接字符串；
- `meta_path`：对象元数据路径；
- `dependencies`：被依赖对象的 `object_id`，以及无法解析时的外部 qualified name；
- `status`：`complete`、`partial`、`opaque`、`failed`；
- 至少一个 `content_hash`，或对 `partial/opaque` 说明不可计算原因。

表对象通常还应包含 `schema_path`、`data_artifacts`、`row_identity` 和 `statistics`。没有数据的 view、procedure 等对象可以不含 data artifact，但仍需保留定义和依赖。

### 7.4 增量字段

当 `backup_type` 不是 `full` 时，`parent` MUST 至少包含：

```json
{
  "backup_id": "parent-backup-id",
  "base_backup_id": "root-full-backup-id",
  "base_snapshot_id": "snapshot-id",
  "coverage": {
    "from": "native-position-or-time",
    "to": "native-position-or-time",
    "complete": true,
    "gaps": []
  },
  "schema_epoch": "schema-fingerprint-or-epoch"
}
```

`coverage.complete=false` 或 `gaps` 非空时，默认不得提供“任意时间点恢复”按钮；CLI 只能恢复到明确标注的最近有效点，除非用户显式接受数据缺口。

## 8. Object Metadata 设计

### 8.1 通用 envelope

每个对象的 `meta.json` 使用统一 envelope；数据库特有字段放在 `extensions`，不能改变核心字段含义：

```json
{
  "object_id": "018f2e5a-5f70-7c00-8a1b-111111111111",
  "identity_key": "postgresql/app/public/table/users",
  "kind": "table",
  "name": "users",
  "qualified_name": {
    "catalog": null,
    "database": "app",
    "schema": "public",
    "name": "users"
  },
  "source": {
    "database_type": "postgresql",
    "native_id": "optional-oid",
    "raw_type": "BASE TABLE"
  },
  "ddl": {
    "drop": "DROP TABLE ...",
    "create": "CREATE TABLE ...",
    "pre_data": null,
    "post_data": null,
    "raw_definition": null
  },
  "columns": [],
  "indexes": [],
  "constraints": [],
  "dependencies": [],
  "data": null,
  "options": {},
  "extensions": {}
}
```

`ddl.create`、`ddl.drop` 等文本是可审计的原始定义，不是跨数据库执行保证。恢复时 adapter MUST 重新 quote 标识符、检查目标 capability，并根据 phase 选择执行片段。若驱动能提供结构化 DDL，结构化字段优先，原始 DDL 作为兼容 fallback。

### 8.2 表和列

表对象至少保存：

- 列的声明顺序和稳定 `column_id`（没有 native id 时由规范化名称和 ordinal 组成）；
- `name`、`quoted_name`（可选）、`raw_type`、`logical_type`；
- `nullable`、`default_expression`、`generated_expression`、`identity`；
- length、precision、scale、fractional seconds precision；
- charset、collation、comment；
- row identity 策略：`primary_key`、`unique_key`、`rowid`、`synthetic_hash`、`none`；
- 数据 artifact 的列映射、排序、分区和统计信息。

列示例：

```json
{
  "column_id": "users.id",
  "ordinal": 0,
  "name": "id",
  "raw_type": "bigint",
  "logical_type": {"kind": "int", "signed": true, "bits": 64},
  "nullable": false,
  "default_expression": "nextval('users_id_seq')",
  "generated": null,
  "identity": {"kind": "sequence", "sequence_name": "users_id_seq"},
  "charset": null,
  "collation": null,
  "comment": null
}
```

### 8.3 索引、约束和程序对象

`indexes[]` MUST 表达名称、唯一性、方法/类型、列或表达式顺序、过滤谓词、包含列、排序方向和 `create_phase`。`constraints[]` 至少区分 primary、unique、check、foreign_key、exclusion（若支持），并记录引用对象/列、match/action、验证状态和是否可延迟。

view、materialized view、function、procedure、trigger、event、policy、rule 等对象 MUST 保留：

- 原始数据库类型和完整定义；
- 参数名、参数逻辑类型、返回类型（适用时）；
- 执行时机、事件、作用表（trigger/event）；
- 依赖的对象和扩展语言；
- `security_definer`、owner 等安全属性，但恢复默认不自动提升权限；
- 无法解析的 body 作为 opaque 文本，不擅自改写。

### 8.4 数据引用和 BLOB

表对象的 `data` 可采用以下结构：

```json
{
  "format": "parquet",
  "schema_path": "objects/table/<id>/schema.arrow.json",
  "artifacts": ["artifact-users-00000", "artifact-users-00001"],
  "row_identity": {
    "kind": "primary_key",
    "columns": ["id"],
    "ordering": "primary_key_ascending"
  },
  "null_encoding": "native",
  "lob_policy": "external_over_1_mib",
  "statistics": {"row_count": 100000, "estimated_bytes": 8388608}
}
```

超过策略阈值的 BLOB/CLOB/JSON/geometry 可以写入 content-addressed blob：`blobs/<sha256>.bin`。表数据单元格保存 `{ "blob_ref": "sha256:...", "media_type": "...", "length": ... }`。blob 的 plaintext hash、长度和 MIME/逻辑类型都必须进入 artifact 清单；恢复时若 blob 缺失必须失败，不得插入空值代替。

### 8.5 扩展字段

扩展键 SHOULD 使用反向域名或 `x-<vendor>-<name>`，例如 `x-postgresql-toast`。核心 reader：

- MUST 保留未知扩展，便于 round-trip；
- MAY 忽略未知扩展的恢复动作；
- MUST 在 restore report 中报告被忽略的扩展；
- 不得将扩展中的字符串直接当作 shell 命令或未经确认的权限操作。

## 9. 对象身份、命名和依赖

### 9.1 身份模型

显示名称不是全局唯一身份。实现 MUST 同时保存：

- `object_id`：本备份内的引用 ID；
- `identity_key`：跨备份匹配键；
- `qualified_name`：用户可读和目标映射键；
- `source.native_id`：数据库提供的 OID、object_id、内部 UUID 等（如有）；
- `source.server_uuid`：避免不同实例的 native id 碰撞。

`identity_key` 的推荐生成顺序：

1. 数据库 native object id + source server identity；
2. 规范化的 database/schema/kind/name + creation fingerprint；
3. 无稳定信息时使用规范化名称，并把 rename detection 标记为 best effort。

名称规范化 MUST 明确大小写、Unicode normalization、引用标识符和 catalog 规则；不能假设所有数据库都按同一种大小写折叠。对象重命名时，增量应设置 `renamed_from` 或 `identity_aliases`，而不是创建一个看似全新的对象并默默删除旧对象。

### 9.2 依赖图

边语义为“**A depends_on B**”，恢复必须先处理 B。依赖可分为：

- `hard`：缺失会使对象无法创建（例如函数引用的类型）；
- `soft`：可以延后或重新编译（例如 view 引用尚未建好的索引）；
- `external`：依赖未纳入本备份范围，必须报告并由用户映射；
- `ordering_only`：只表达建议顺序。

对象列表应先在内存构建 DAG，再进行拓扑排序。循环依赖处理顺序：

1. 将可延迟的 foreign key/check/trigger 拆到 `post_data` 阶段；
2. 使用两阶段 DDL：先创建壳对象，再补定义；
3. 如果目标支持，暂时禁用约束并在最后重新验证；
4. 若仍无法安全打破，返回 `DependencyCycle`，不得静默跳过。

依赖引用同时带 `object_id` 和可选 qualified name，方便第三方工具在裁剪对象后诊断断链。

## 10. 数据文件格式选择

### 10.1 决策矩阵

| 格式 | 优点 | 缺点 | ODBF 定位 |
| --- | --- | --- | --- |
| Parquet | 列式压缩好；row group 可独立校验/读取；生态广；适合对象存储与统计 | 写入端较复杂；不天然表达所有数据库语义；单行随机恢复不理想 | **大表默认持久化格式** |
| Arrow IPC Stream/File | 与内存列式模型接近；适合跨进程流；无需先生成完整文件 | 长期归档生态和压缩比通常弱于 Parquet；stream 不便随机访问 | **sidecar/中间传输与可流式 fallback** |
| NDJSON | 人类可读；逐行处理；实现简单；错误定位容易 | 体积大；类型需要额外 schema；二进制和精度处理繁琐 | **兼容/调试 fallback** |
| CSV/TSV | 工具普及；人工交换方便 | NULL/空串、换行、编码、二进制、时区和复杂类型难以无损 | **人工交换；不作为可逆主格式** |
| SQL INSERT | 可直接查看；某些目标可执行 | 体积大、解析慢、容易受方言/quote/sql_mode 影响，LOB 与增量困难 | **DDL/程序体表达与末级兼容 fallback** |
| 数据库原生物理页/文件 | 同库同版本恢复快 | 强绑定版本、平台、存储引擎和部署；不能对象化跨库 | **不属于 ODBF 逻辑核心，可作为 opaque extension artifact** |

首选策略：

1. 大表数据 SHOULD 写为 Parquet，默认每个 part 包含有限 row group；
2. driver/sidecar 与 Host 之间 SHOULD 使用 Arrow IPC 或现有 row batch 流，Host 再写 Parquet；
3. 无法可靠映射到 Parquet 时 MAY 使用 Arrow IPC 或 NDJSON，但 Manifest 必须记录实际格式和 `lossiness`；
4. CSV 和 SQL 不得在没有显式用户选择时成为默认数据备份；
5. DDL、view/procedure/function/trigger body 可以是 SQL 文本，但必须与结构化 metadata 同时存在或标记为 opaque。

### 10.2 逻辑 schema 是语义真相

Parquet physical schema 不能单独承担跨数据库语义。每个含数据的表 MUST 有 `schema.arrow.json` 或同等的 ODBF logical schema，至少保存：

- 列顺序和列 identity；
- ODBF logical type；
- 原始数据库类型；
- nullable、precision、scale、length；
- 时间单位、时区和日历；
- 字符集、collation；
- extension type 和 raw fallback；
- Parquet/Arrow field 的映射；
- schema fingerprint。

恢复 adapter 先根据 logical schema 决定目标类型，再读取物理数据。若目标数据库不能无损容纳某类型，dry-run MUST 产生 `lossy_conversion`，默认策略可要求用户确认。

### 10.3 核心类型映射

| 源语义 | ODBF 表达 | 要求 |
| --- | --- | --- |
| NULL | native validity bitmap / JSON `null` | 不能用空串、0 或 `\N` 猜测 |
| signed/unsigned integer | `int{bits,signed}` | 目标不支持 unsigned 时先做范围检查 |
| Decimal/Numeric | `decimal{precision,scale}`，value 无损编码 | 不经 IEEE-754 float；超规格值保存 string/raw |
| Float | `float{bits}` + NaN/Inf policy | hash/校验需规范化 NaN 和 signed zero 策略 |
| Text | UTF-8 或声明源编码 + raw bytes fallback | 非 UTF-8 内容不能静默替换字符 |
| Binary | bytes/blob ref | 不尝试按 UTF-8 解码 |
| Date/Time/Timestamp | unit、precision、timezone、calendar | 区分 timestamp with/without time zone |
| UUID | 16 bytes + canonical text | 保留源排序/variant 信息（如有） |
| JSON | text/binary + validated flag | 保留原始字节或规范化策略，不能擅自重排后用于 byte hash |
| Array/Map/Struct | Arrow nested type + raw type | 目标不支持时需要显式转换策略 |
| Interval/Duration | months/days/nanos 分量 | 避免把月简单换算成秒 |
| Geometry/Geography | WKB/EWKB + SRID + source type | 保留 endian、dimension、SRID |
| Custom/Domain/Enum | logical base + type identity + raw bytes/text | 先恢复 type/domain，再恢复引用列 |

`extension_protocol::row::Value` 已能表达 Decimal、Bytes、UUID、Date/Time/Datetime、Array、Map、Geo、Custom 等类型，可作为 wire IR 的起点；ODBF 仍需补齐持久化 schema、精度、时区、原始类型和可逆性元数据。

### 10.4 分片、排序和确定性

- 默认 part 目标大小 SHOULD 可配置，例如 128–512 MiB；row group 可采用 32–128 MiB，但最终值需以 benchmark 为准；
- 每个 part MUST 记录行数、字节数、schema hash、可选 min/max 和 row identity range；
- 有主键或稳定唯一键时 SHOULD 按该键确定性排序，便于 diff、断点和复现；
- 无稳定顺序时必须标记 `ordering=unspecified`，不能用文件字节 hash 推断业务数据未变化；
- writer MUST 限制并发表数、每表缓冲和总内存；
- Parquet 已使用 zstd/snappy 等列压缩时，外层默认不再 gzip，避免双重压缩。

## 11. 快照一致性模型

### 11.1 一致性级别

Manifest 的 `consistency.mode` MUST 是以下之一：

| 模式 | 语义 | 默认恢复保证 |
| --- | --- | --- |
| `transaction_snapshot` | 同一个数据库原生一致性快照；跨表读取共享同一边界 | 可声明事务一致 |
| `native_backup_snapshot` | 使用数据库 Online Backup/backup API 得到一致性镜像，再对象化读取 | 由 adapter 声明保证 |
| `locked_snapshot` | 短时或全程锁定相关对象 | 一致但可能影响业务 |
| `crash_consistent` | 文件/日志级可恢复，但未必跨表事务一致 | 只能保证崩溃恢复语义 |
| `best_effort` | 各对象分别读取，可能跨事务边界 | 必须显示强警告，不能标为一致备份 |

策略请求可设置：

- `required`：达不到指定级别即失败；
- `prefer`：优先指定级别，允许记录原因后降级；
- `best_effort`：用户明确接受非一致性结果。

任何自动降级都 MUST 写入 Manifest 和 report；UI 必须在开始前和完成后显示。

### 11.2 数据库策略

| 数据库 | 全量一致性建议 | 增量位点 | 主要限制/前置条件 |
| --- | --- | --- | --- |
| PostgreSQL | 单事务 `REPEATABLE READ`；多连接读取时使用可共享 exported snapshot；记录 snapshot/transaction 与 LSN | WAL、logical decoding、LSN | replication slot/WAL 权限与保留；DDL 和 sequence 语义需单独记录 |
| MySQL | InnoDB `REPEATABLE READ` + `START TRANSACTION WITH CONSISTENT SNAPSHOT`；必要时锁定元数据 | binlog file/position 或 GTID | 非事务表可能不一致；需 binlog 开启、row image/权限/保留满足要求 |
| SQL Server | snapshot isolation、database snapshot 或受支持的 backup API；记录 database/transaction LSN | full/diff/log backup chain、LSN | 部署版本和权限差异大；不能假设普通查询账户可读取 transaction log |
| SQLite | 优先 SQLite Online Backup API 或稳定 read transaction；记录 journal/WAL 状态 | 通常 Snapshot + Diff | 直接复制 `.db` 在活跃 WAL 下不一定完整；没有通用 server log 增量 |

以上是 adapter 策略，不是 ODBF reader 的硬编码。adapter 在 `inspect_capabilities` 阶段返回实际可用性、所需权限和降级原因。

### 11.3 Schema 与数据边界

对象 metadata 和表数据 MUST 来自同一快照边界，或者 Manifest 明确标记不同 capture time。为了避免“DDL 在数据导出中途改变”：

1. snapshot 开始后读取 schema epoch/fingerprint；
2. 导出前后再次检查；
3. 若 schema 变化且数据库不能保证 metadata snapshot，则失败或把受影响对象标为 partial；
4. 增量日志必须包含覆盖期间的 schema change event，或者创建新的 full/snapshot baseline。

sequence/identity 当前值通常不是普通表事务快照的一部分，必须作为单独 artifact/metadata 捕获，并在 consistency warning 中说明其语义。

## 12. 全量备份流程

### 12.1 状态机

```text
Created
  → Preflight
  → SnapshotAcquired
  → ObjectsDiscovered
  → MetadataWritten
  → DataExporting
  → Finalizing
  → Verified
  → Published

任意中间状态 → Cancelling/Failed → StagingRetained 或 Cleaned
```

### 12.2 详细步骤

1. **解析请求**：验证 scope、格式、输出后端、压缩、加密、签名和一致性策略；生成 `task_id` 与 `backup_id`。
2. **建立 staging**：创建权限受限的临时目录/remote multipart session，写入 `journal.json`；最终路径尚不可见。
3. **Preflight**：
   - 检查数据库版本、对象能力、snapshot/log 权限；
   - 检查输出空间、最大文件数、路径策略和 KMS/密码来源；
   - 估算行数/字节（允许未知）；
   - 根据策略决定失败、降级或警告。
4. **开始快照**：通过 `BackupSource::begin_snapshot` 获得 `SnapshotSession`，记录一致性信息和 native cursor 起点。
5. **发现对象**：按 filter 枚举 database/schema/table/view/type/function/procedure/trigger 等，分配 `object_id`、计算 `identity_key`，建立依赖图。
6. **写对象 metadata**：读取结构化 metadata 和原始 DDL，写 canonical JSON；无法解析的对象保留 opaque definition。
7. **导出表数据**：
   - snapshot session 流式读取 row/Arrow batch；
   - 类型映射、LOB 外置、Parquet 分片；
   - 增量计算 chunk/artifact hash 和统计；
   - 通过有界 channel 施加背压；
   - 每个完成的 part 写入 journal。
8. **结束快照**：确保所有快照内读取完成，记录 native cursor 终点，调用 `finish`；异常时显式 rollback/close。
9. **生成 Manifest**：按稳定顺序写对象和 artifact 引用；不得引用尚未完成的文件。
10. **完整性收尾**：校验所有文件大小/hash，生成 `checksums.json`，可选签名和加密 envelope。
11. **自验证**：用独立 reader 重新打开 staging，执行路径、版本、引用、hash、DAG 和解密元数据检查。
12. **原子发布**：fsync（本地后端）、写完成标记并 rename；对象存储后端先上传 immutable artifacts，最后提交 manifest/ref。
13. **报告**：生成 `BackupReport`，包含实际一致性、对象成功/失败、行数、字节、警告和可恢复点。

### 12.3 部分失败策略

默认 `strict=true`：任何选中对象失败都不发布备份。用户可以显式选择 `allow_partial`；此时：

- 失败对象仍进入 Manifest，`status=failed/partial`；
- report 必须给出错误码和是否影响依赖；
- `recoverability` 不能标为 `complete`；
- 恢复 UI 默认不把该备份展示为完整灾备点；
- 权限或读取错误不能被悄悄当成“对象不存在”。

## 13. 恢复流程

### 13.1 先计划、后执行

`RestoreManager` 必须提供 `plan()`/dry-run。计划阶段不修改目标数据库，输出：

- 备份格式、签名、hash、加密和依赖检查结果；
- 源/目标数据库类型与版本兼容性；
- database/schema/table 的映射和 rename；
- 将创建、替换、合并、跳过和冲突的对象；
- 类型转换、丢失语义、外部依赖和所需权限；
- phase DAG、预计数据量和不可回滚操作；
- 增量链、目标 restore point 和日志连续性。

计划产生稳定 `plan_hash`。执行阶段必须确认输入 backup content hash、目标 fingerprint、mapping 和 plan hash 与计划一致，防止 UI 预览后目标发生变化。

### 13.2 恢复阶段

推荐 phase 顺序：

1. 打开包并验证路径、版本、Manifest、hash、签名和解密参数；
2. 验证整个增量链和目标 restore point；
3. 解析 target mapping、object selection、external dependency；
4. 创建 database、schema、type/domain、collation、基础 sequence；
5. 创建 table 主体，暂不创建外键、非必要索引和 trigger；
6. 导入表数据；
7. 恢复 identity/sequence 当前值；
8. 创建 primary/unique/普通/全文/空间等索引；
9. 创建并验证 check、foreign key 和 exclusion constraint；
10. 创建 view/materialized view；必要时先建 placeholder 后 replace；
11. 创建 function、procedure 和其它程序对象；
12. 创建 trigger、event、policy、rule；
13. 可选恢复 owner、user、role、grant；默认需要显式确认；
14. 应用连续的增量日志或 row delta 到目标 restore point；
15. 执行 schema/data/integrity 校验；
16. 写 `RestoreReport` 和最终 checkpoint。

对象实际顺序由 DAG 决定，phase 只限定大边界。adapter 可声明某数据库必须把 function 放在 view 之前等额外规则。

### 13.3 冲突策略

每类对象支持显式策略：

| 策略 | 语义 | 限制 |
| --- | --- | --- |
| `fail` | 目标已存在即停止 | 默认、最安全 |
| `skip` | 保留目标对象，跳过源对象 | 依赖和数据可能不完整，必须报告 |
| `replace` | 删除/替换目标对象 | 需展示级联影响和不可回滚风险 |
| `rename` | 按 mapping 创建新名称 | 必须重写结构化依赖，opaque body 需人工确认 |
| `merge` | 对表做 insert/upsert/append | 需要稳定 key；不是通用 schema 合并 |
| `truncate_and_load` | 保留表结构，清空后导入 | 目标 schema 必须兼容且用户确认 |

`replace` 不得默认使用无界 `CASCADE`。`merge` 在没有主键/唯一键时必须拒绝或要求明确 append-only 策略。

### 13.4 事务、checkpoint 与恢复后校验

- 小对象 DDL MAY 按 phase 使用事务；大表数据通常按 part/batch 提交，不能假设整个恢复可单事务回滚；
- checkpoint 至少记录 `(backup_id, plan_hash, target_fingerprint, phase, object_id, artifact_id, offset/batch, checksum)`；
- resume 时必须验证已经完成的目标对象和 artifact，不能只信 journal；
- trigger/index 暂停只能在 adapter 声明支持且最终有补偿步骤时使用；
- 失败报告必须区分已提交与已回滚内容，并给出可安全重试点；
- 恢复完成不等于验证完成。若数据验证失败，最终状态应为 `restored_with_verification_errors`，不能显示“成功”。

## 14. 增量备份与点时间恢复

### 14.1 三类增量

#### A. 原生日志增量（优先）

- MySQL：binlog file/position 或 GTID，优先 row-based event；
- PostgreSQL：WAL 或 logical decoding change stream，以 LSN 表示覆盖范围；
- SQL Server：受支持的 transaction log/full-diff-log backup chain，以 LSN 校验连续性；
- SQLite：没有通用 server log，通常不采用此模式。

原始日志可以保存在 `logs/`，但必须由对应 source/restore adapter 解释。核心 ODBF reader 只校验 artifact 和覆盖区间，不能把原生日志假设为可跨数据库执行的 SQL。

#### B. Snapshot + Diff

在无法访问原生日志时：

1. 以前一份完整或 diff snapshot 为父；
2. 比较对象 metadata fingerprint；
3. 对有稳定 row identity 的表，按 key range、watermark、change tracking 或 chunk hash 识别变化；
4. 用 upsert row set 表示新增/更新，用 tombstone key set 表示删除；
5. 没有稳定 row identity 时整表替换，不能宣称可靠的行级 diff；
6. 对无序表的相等判断采用规范化 row multiset/hash 或 whole-object replacement，不使用 part 文件字节相等作为唯一依据。

`updated_at` 只能在用户声明其完整、单调、所有写路径都维护时作为优化线索；它不能默认检测删除，也不能单独构成正确性保证。

#### C. 对象级 metadata delta

记录 `create`、`alter`、`rename`、`drop`、`replace` 事件及 schema epoch。结构化 patch 只用于可可靠建模的对象；程序体或数据库私有属性 SHOULD 以完整 metadata replacement 保存，避免不完整 patch。

### 14.2 增量 Manifest

增量必须记录：

- `parent.backup_id` 与 `base_backup_id`；
- `coverage.from/to` 和单位（time、GTID、LSN 等）；
- `source_instance_fingerprint`，防止把不同服务器日志串到同一链；
- `schema_epoch` 和 schema change event；
- 数据增量策略及 row identity；
- `restore_points[]`；
- 所需前置 artifact/hash；
- gap、截断、过滤表和未覆盖对象；
- 日志时区、timestamp precision 和事务边界。

### 14.3 链完整性与任意点恢复

恢复链校验 MUST：

1. 从 full baseline 开始；
2. 每一段 parent id 和 parent tree hash 与实际父备份匹配；
3. native position 连续且方向单调；
4. source instance、database id、timeline/GTID domain 等一致；
5. schema event 在数据 event 之前应用；
6. 目标时间不能落在已知 gap；
7. 事务必须原子应用，不能在事务中间截断。

检测日志缺口时默认 fail closed。允许的显式降级只有：

- 恢复到 gap 前最近有效点；
- 选择新的 full baseline；
- 用户接受 best-effort，并在 report 中永久标记不完整。

### 14.4 数据库特定注意事项

**PostgreSQL**：WAL 物理重放与逻辑对象恢复语义不同。ODBF 首选 logical decoding 作为对象级增量；原始 WAL 可归档为 opaque native artifact，但只有兼容版本/实例 adapter 才能重放。replication slot 会阻止 WAL 回收，manager 必须监控磁盘并在取消/失败时清理临时 slot。

**MySQL**：需要记录 binlog format、row image、server UUID、GTID set 或 file/position。statement-based event 受 SQL mode、time zone 和非确定函数影响，应降级或拒绝精确对象级 replay。非事务表与 DDL auto-commit 要单独标记。

**SQL Server**：transaction log 的读取和恢复通常需要更高权限与数据库 backup chain 支持。普通 SQL driver 不一定能提供该能力；adapter 必须进行 capability/edition/permission preflight，不能通过未公开内部结构“猜测”日志。

**SQLite**：默认使用周期性一致性 snapshot + page/object diff 或整表 replacement。SQLite WAL 是数据库文件恢复机制，不等价于通用跨版本逻辑变更流；除非实现专门且兼容性受控的 adapter，不对外宣称 PITR。

## 15. 校验与完整性

### 15.1 四层校验

1. **文件级**：每个 metadata、data、blob、log 文件都有 SHA-256 和 size；
2. **chunk 级**：每个 Parquet row group、Arrow batch 或日志 segment 可有独立 hash、行数/事件数；
3. **对象级**：canonical metadata + logical schema hash + artifact refs + row count 形成 `content_hash`；
4. **备份级**：`checksums.json` 的规范化 entries 形成 `tree_hash`，可选 Ed25519 签名。

未来可以增加 BLAKE3 等算法，但 ODBF 1.0 reader MUST 支持 SHA-256。算法名必须带在值中，例如 `sha256:<hex>`，避免长度猜测。

### 15.2 避免循环 hash

ODBF 1.0 采用以下规则：

1. `manifest.json` 只声明 `checksums.json` 路径，不内嵌该文件的 hash；
2. `checksums.json.entries` 包含 `manifest.json` 和所有 payload，排除 `checksums.json` 自身与签名文件；
3. `tree_hash` 是对按 path 排序的 canonical entries（排除 `tree_hash` 字段）计算的 hash；
4. `signature/manifest.sig` 签名 `(format_id, format_version, backup_id, tree_hash)`；
5. unsigned 备份仍可验证 payload，但不能证明 `checksums.json` 来源；外层 archive/object-store ETag 仅是传输辅助，不替代 ODBF hash。

示例：

```json
{
  "format": "odbf-checksums-1",
  "algorithm": "sha256",
  "entries": [
    {"path": "manifest.json", "size_bytes": 4096, "sha256": "sha256:..."},
    {"path": "objects/table/<id>/meta.json", "size_bytes": 2048, "sha256": "sha256:..."}
  ],
  "tree_hash": "sha256:..."
}
```

### 15.3 数据语义校验

可配置的源/目标验证包括：

- schema fingerprint、列/索引/约束数量；
- 表行数；
- 主键/唯一键计数和 duplicate 检查；
- 每列 NULL/non-NULL、min/max、长度分布；
- 按 key range 的规范化 row checksum；
- 外键 orphan 检查；
- sequence/identity 范围；
- native log position continuity；
- 抽样读取和目标端 query validation。

浮点 NaN、collation、无序行、JSON 文本格式、时区和并发变更会影响比较。每项验证必须记录算法、类型规范化、scope 和置信级别；不能只显示一个无上下文的“checksum 相等”。

## 16. Rust 模块划分

### 16.1 推荐 crate 边界

长期建议拆成两层；首个 PoC 可以先在一个 crate 内保持相同模块边界，稳定后再拆分：

```text
crates/db-backup-core/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── model/
    │   ├── manifest.rs
    │   ├── object.rs
    │   ├── artifact.rs
    │   ├── identity.rs
    │   ├── logical_type.rs
    │   └── version.rs
    ├── format/
    │   ├── reader.rs
    │   ├── writer.rs
    │   ├── canonical_json.rs
    │   ├── parquet.rs
    │   ├── arrow.rs
    │   └── fallback.rs
    ├── graph/
    │   ├── dependency.rs
    │   └── restore_plan.rs
    ├── integrity/
    │   ├── checksum.rs
    │   ├── object_hash.rs
    │   └── signature.rs
    ├── crypto/
    │   ├── envelope.rs
    │   ├── kdf.rs
    │   └── aead.rs
    ├── storage/
    │   ├── mod.rs
    │   ├── local.rs
    │   └── content_addressed.rs
    └── error.rs

crates/db-backup/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── adapter/
    │   ├── mod.rs
    │   ├── mysql.rs
    │   ├── postgresql.rs
    │   ├── mssql.rs
    │   ├── sqlite.rs
    │   └── external.rs
    ├── incremental/
    │   ├── native_log.rs
    │   ├── snapshot_diff.rs
    │   ├── chain.rs
    │   └── restore_point.rs
    ├── storage/
    │   ├── sftp.rs
    │   ├── webdav.rs
    │   └── s3.rs
    ├── backup_manager.rs
    ├── restore_manager.rs
    ├── checkpoint.rs
    ├── progress.rs
    ├── policy.rs
    └── error.rs
```

依赖方向：

```text
db-backup-core             # 不依赖 db / GPUI / extension-host
       ▲
       │
db-backup ───────► db
       ├──────────► extension-protocol
       └──────────► extension-host（仅 external sidecar adapter）
       ▲
       │
db_view / main / CLI
```

`db` 不应反向依赖 `db-backup`，否则容易形成循环。若以后 `db` 需要公开共同 trait，可把最小 contract 下沉到 `db-backup-core` 或新的 `db-contracts`，由上层完成 adapter 注入。

### 16.2 模块职责

| 模块 | 责任 | 禁止事项 |
| --- | --- | --- |
| `model` | serde DTO、版本、逻辑类型、对象和 artifact 身份 | 不做数据库查询 |
| `format` | ODBF reader/writer、canonical JSON、Parquet/Arrow codec | 不管理 UI/连接 |
| `graph` | 依赖解析、拓扑排序、phase 和 cycle 诊断 | 不执行 DDL |
| `integrity` | hash、tree hash、签名、验证报告 | 不持有用户密码 |
| `crypto` | envelope、AEAD、KDF 参数与 key wrapping | 不把密钥序列化入 Manifest |
| `storage` | staging、atomic publish、range/multipart、GC | 不解释表和日志语义 |
| `adapter` | source/target capability、snapshot、schema/data/log | 不决定 UI policy |
| manager | 编排、取消、重试、checkpoint、report | 不包含数据库方言硬编码 |
| `incremental` | chain、cursor、diff/tombstone、restore point | 不假设所有数据库有日志 |

### 16.3 异步运行时约束

所有数据库 Future MUST 在 Tokio runtime 上运行。GPUI 层发起任务时，应通过 NavOp 已有的 Tokio 绑定入口（例如 `one_core::gpui_tokio::Tokio::spawn_result`）桥接结果，不能把数据库 Tokio Future 直接放入 GPUI `background_spawn`。CPU 密集的 Parquet 编码、压缩、hash 可以进入专用 bounded blocking pool，但其输入输出 channel 仍需背压和取消。

核心 reader/writer 应支持纯 async IO，也可为本地文件提供 sync facade。不要在 driver callback 内嵌新的 runtime 或阻塞当前 Tokio worker。

## 17. `BackupManager` 接口设计

以下接口是建议的 Rust contract 草图，用于明确责任，不要求第一版逐字采用相同泛型：

```rust
#[derive(Debug, Clone)]
pub struct BackupRequest {
    pub task_id: Uuid,
    pub mode: BackupMode,
    pub scope: BackupScope,
    pub consistency: ConsistencyPolicy,
    pub format: DataFormatPolicy,
    pub destination: StorageLocation,
    pub integrity: IntegrityPolicy,
    pub encryption: Option<EncryptionPolicy>,
    pub failure_policy: FailurePolicy,
    pub resume: ResumePolicy,
}

#[derive(Debug, Clone)]
pub enum BackupMode {
    Full,
    IncrementalLog { parent: BackupRef, until: Option<RestorePoint> },
    IncrementalDiff { parent: BackupRef },
    MetadataOnly { parent: Option<BackupRef> },
}

#[derive(Debug, Clone)]
pub struct BackupCapabilities {
    pub snapshot_modes: Vec<ConsistencyMode>,
    pub object_kinds: BTreeSet<ObjectKind>,
    pub data_formats: BTreeSet<ArtifactFormat>,
    pub native_log: Option<NativeLogCapabilities>,
    pub stable_row_identity: RowIdentityCapabilities,
    pub concurrent_readers: u16,
    pub supports_resume: bool,
    pub warnings: Vec<CapabilityWarning>,
}
```

### 17.1 Source 和 snapshot session

```rust
#[async_trait]
pub trait BackupSource: Send + Sync {
    async fn inspect_capabilities(
        &self,
        request: &BackupRequest,
    ) -> Result<BackupCapabilities, BackupError>;

    async fn begin_snapshot(
        &self,
        request: &SnapshotRequest,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn SnapshotSession>, BackupError>;

    async fn open_incremental(
        &self,
        request: &IncrementalRequest,
        cancel: &CancellationToken,
    ) -> Result<Option<Box<dyn IncrementalSession>>, BackupError>;
}

#[async_trait]
pub trait SnapshotSession: Send {
    fn consistency(&self) -> &ConsistencyInfo;

    async fn list_objects(
        &mut self,
        filter: &BackupFilter,
    ) -> Result<Vec<ObjectDescriptor>, BackupError>;

    async fn read_object_meta(
        &mut self,
        object: &ObjectDescriptor,
    ) -> Result<ObjectMeta, BackupError>;

    async fn stream_table_data(
        &mut self,
        request: TableReadRequest,
        sink: &mut dyn RowBatchSink,
        cancel: &CancellationToken,
    ) -> Result<TableReadResult, BackupError>;

    async fn capture_native_cursor(
        &mut self,
    ) -> Result<Option<NativeLogCursor>, BackupError>;

    async fn finish(self: Box<Self>) -> Result<(), BackupError>;
    async fn abort(self: Box<Self>) -> Result<(), BackupError>;
}
```

`SnapshotSession` 持有数据库快照生命周期。manager 不能在 session 外另开不共享快照的连接读取表数据，除非 adapter 声明该方式仍一致。`abort` 必须清理 transaction、replication slot、temporary snapshot 或 driver stream。

### 17.2 Artifact writer

```rust
#[async_trait]
pub trait ArtifactSink: Send {
    async fn begin(
        &mut self,
        descriptor: ArtifactDescriptor,
    ) -> Result<ArtifactWriterId, BackupError>;

    async fn write_chunk(
        &mut self,
        writer: ArtifactWriterId,
        chunk: Bytes,
    ) -> Result<WriteReceipt, BackupError>;

    async fn commit(
        &mut self,
        writer: ArtifactWriterId,
        summary: ArtifactSummary,
    ) -> Result<ArtifactRef, BackupError>;

    async fn abort(
        &mut self,
        writer: ArtifactWriterId,
    ) -> Result<(), BackupError>;
}
```

`commit` 只有在 size/hash/encryption tag 与 summary 一致时成功。writer id 不得成为长期 artifact id；重试时可生成新 writer，最终 artifact 由内容/hash 和 manifest ref 标识。

### 17.3 Manager

```rust
pub struct BackupManager<S, D> {
    storage: S,
    source_factory: D,
    limits: BackupLimits,
}

impl<S, D> BackupManager<S, D>
where
    S: BackupStorage,
    D: BackupSourceFactory,
{
    pub async fn plan(
        &self,
        request: &BackupRequest,
    ) -> Result<BackupPlan, BackupError>;

    pub async fn run(
        &self,
        request: BackupRequest,
        progress: &dyn BackupProgressSink,
        cancel: CancellationToken,
    ) -> Result<BackupReport, BackupError>;

    pub async fn verify(
        &self,
        location: &StorageLocation,
        policy: VerifyPolicy,
        progress: &dyn BackupProgressSink,
        cancel: CancellationToken,
    ) -> Result<VerificationReport, BackupError>;
}
```

Manager 负责 policy/filter、快照编排、依赖图、staging、artifact writer、hash/签名/加密、atomic publish、journal、取消、重试和报告。具体 adapter 负责数据库事实；storage 负责持久化；任何一层都不能偷偷接管另一层的权限决策。

### 17.4 进度事件

进度不是普通日志字符串，而是稳定的结构化事件：

```rust
pub enum BackupEventKind {
    Started,
    PreflightCompleted,
    SnapshotAcquired,
    ObjectDiscovered,
    ObjectMetaWritten,
    DataChunkWritten,
    ObjectFinished,
    PhaseStarted,
    Warning,
    Retrying,
    Cancelling,
    Cancelled,
    Failed,
    Completed,
}

pub struct BackupProgressEvent {
    pub task_id: Uuid,
    pub sequence: u64,
    pub kind: BackupEventKind,
    pub phase: BackupPhase,
    pub object_id: Option<Uuid>,
    pub artifact_id: Option<String>,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub rows_done: u64,
    pub rows_total: Option<u64>,
    pub warning: Option<BackupWarning>,
    pub occurred_at: DateTime<Utc>,
}
```

事件 stream 必须有界；UI 消费慢时可合并高频 `DataChunkWritten`，但不能丢失状态转换、warning、failed/completed。`sequence` 用于去重和恢复 UI 状态。

## 18. `RestoreManager` 接口设计

### 18.1 Request、plan 与 report

```rust
#[derive(Debug, Clone)]
pub struct RestoreRequest {
    pub task_id: Uuid,
    pub source: BackupLocation,
    pub restore_point: Option<RestorePoint>,
    pub selection: ObjectSelection,
    pub mapping: TargetMapping,
    pub conflicts: ConflictPolicies,
    pub verification: RestoreVerifyPolicy,
    pub security_items: SecurityRestorePolicy,
    pub resume: ResumePolicy,
}

#[derive(Debug, Clone)]
pub struct RestorePlan {
    pub plan_id: Uuid,
    pub plan_hash: String,
    pub backup_tree_hash: String,
    pub target_fingerprint: String,
    pub phases: Vec<RestorePhasePlan>,
    pub conversions: Vec<TypeConversion>,
    pub conflicts: Vec<ObjectConflict>,
    pub external_dependencies: Vec<ExternalDependency>,
    pub warnings: Vec<RestoreWarning>,
    pub irreversible_actions: Vec<IrreversibleAction>,
}
```

### 18.2 Target 和 loader

```rust
#[async_trait]
pub trait RestoreTarget: Send {
    async fn inspect_target(
        &mut self,
        request: &RestoreRequest,
    ) -> Result<TargetCapabilities, RestoreError>;

    async fn fingerprint(&mut self) -> Result<TargetFingerprint, RestoreError>;

    async fn apply_ddl(
        &mut self,
        request: DdlRequest,
        cancel: &CancellationToken,
    ) -> Result<DdlResult, RestoreError>;

    async fn begin_table_load(
        &mut self,
        request: TableLoadRequest,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn TableLoader>, RestoreError>;

    async fn set_identity_state(
        &mut self,
        request: IdentityStateRequest,
    ) -> Result<(), RestoreError>;

    async fn apply_native_incremental(
        &mut self,
        request: NativeIncrementalRequest,
        cancel: &CancellationToken,
    ) -> Result<NativeIncrementalResult, RestoreError>;

    async fn verify(
        &mut self,
        request: TargetVerifyRequest,
        cancel: &CancellationToken,
    ) -> Result<TargetVerification, RestoreError>;
}

#[async_trait]
pub trait TableLoader: Send {
    async fn write_batch(
        &mut self,
        batch: LogicalRowBatch,
        cancel: &CancellationToken,
    ) -> Result<LoadReceipt, RestoreError>;

    async fn checkpoint(&mut self) -> Result<LoaderCheckpoint, RestoreError>;
    async fn commit(self: Box<Self>) -> Result<TableLoadSummary, RestoreError>;
    async fn abort(self: Box<Self>) -> Result<(), RestoreError>;
}
```

`apply_ddl` 接受经过验证的结构化 request，而不是 UI 拼接的任意 SQL。opaque DDL 必须走单独的 `allow_opaque_ddl` policy，并在 dry-run 展示完整文本和 hash。

### 18.3 Manager

```rust
impl RestoreManager {
    pub async fn inspect(
        &self,
        source: &BackupLocation,
        policy: VerifyPolicy,
    ) -> Result<BackupInspection, RestoreError>;

    pub async fn plan(
        &self,
        request: &RestoreRequest,
        target: &mut dyn RestoreTarget,
    ) -> Result<RestorePlan, RestoreError>;

    pub async fn run(
        &self,
        request: RestoreRequest,
        accepted_plan_hash: &str,
        target: &mut dyn RestoreTarget,
        progress: &dyn RestoreProgressSink,
        cancel: CancellationToken,
    ) -> Result<RestoreReport, RestoreError>;

    pub async fn validate_only(
        &self,
        source: &BackupLocation,
        target: &mut dyn RestoreTarget,
        policy: RestoreVerifyPolicy,
    ) -> Result<VerificationReport, RestoreError>;
}
```

`run` 必须拒绝过期的 `accepted_plan_hash`。恢复中目标 capability 改变、schema fingerprint 漂移或 backup tree hash 改变时，需要重新 plan。

## 19. IPC 与 Driver 扩展

### 19.1 可直接复用的当前能力

NavOp 当前 `extension-protocol` 已提供：

- `init`、`shutdown`、capability/method 声明；
- `schema/*` 对象和 DDL introspection；
- `data/export` + `stream/read/close`；
- `data/import_begin/chunk/commit/abort`；
- `blob/open/read/close`；
- 有界 `event/open/read/close`；
- 跨语言 row IR 和 `DataFormat::Parquet` 枚举；
- `extension-host::ProcessRpcSession` 的进程、framing、timeout、cancel 和 shutdown 基础。

这表示可以先用现有方法实现 `best_effort`/单连接 full backup PoC，但 **Parquet 枚举不等于已经实现 ODBF**：仍缺 snapshot session、logical schema、artifact integrity、dependency DAG 和 restore phases。

### 19.2 建议新增的标准方法

建议在 `extension-protocol` 经过版本评审后增加：

```text
backup/capabilities
backup/snapshot_begin
backup/snapshot_keepalive
backup/snapshot_end
backup/objects
backup/object_meta
backup/table_open
backup/table_read
backup/table_close
backup/log_cursor
backup/log_open
backup/log_read
backup/log_close

restore/capabilities
restore/ddl_apply
restore/table_begin
restore/table_write
restore/table_commit
restore/table_abort
restore/identity_set
restore/native_log_apply
restore/verify
```

如果正式标准方法尚未落地，实验实现只能使用 `x/odbf/...` 命名空间，并在 capability negotiation 中声明版本，不能把实验 method 当成永久公共合同。

### 19.3 Capability 示例

```json
{
  "api": "database-backup",
  "version": "1.0",
  "snapshot_modes": ["transaction_snapshot", "best_effort"],
  "object_kinds": ["table", "view", "function", "trigger"],
  "data_streams": ["arrow_ipc", "row_batch"],
  "native_incremental": {
    "kind": "mysql_binlog",
    "positions": ["gtid", "file_offset"],
    "supports_point_in_time": true
  },
  "limits": {
    "max_chunk_bytes": 4194304,
    "max_open_streams": 4
  }
}
```

Host 必须检查 driver manifest 中公开声明的方法，unsupported 能力返回 typed `NotSupported`。不要通过“调用失败后换另一种方式”的 silent fallback 掩盖认证、权限、日志 gap 或版本不兼容。

### 19.4 Wire 约束

- 所有 method request 都携带 `conn_id`、`task_id`，快照内请求另带 `snapshot_id`；
- snapshot id/stream id 是不透明 capability，不能由 Host 猜测或跨连接复用；
- chunk 必须有最大字节数、sequence、EOF、可选 checksum；
- `$/cancelRequest` 与资源 `close/abort` 都要实现，取消后不得继续向无消费者的队列写入；
- driver process 退出时 Host 必须把 stderr 截断、脱敏后放入诊断，不能记录密码、token、连接串或行数据；
- SSH tunnel 继续由 Host 建立，sidecar 只接收已解析的 host/port 和短生命周期 secret reference；
- 大数据不得塞进一个 JSON frame。Arrow/Parquet bytes 通过 blob/stream，JSON-RPC 只传控制和有界 metadata；
- event queue 有界，进度可合并，但 warning/error/lifecycle 不可丢；
- sidecar 需在 snapshot keepalive 超时、Host 断开或 shutdown 时释放数据库事务、slot 和临时文件。

## 20. 压缩与加密

### 20.1 压缩

- Parquet 默认使用其内部 zstd codec；推荐默认 level 3，具体值由 benchmark 决定；
- Arrow IPC、NDJSON、SQL、metadata archive MAY 使用 zstd；
- gzip 仅作为兼容 fallback；
- artifact metadata MUST 记录 codec、level、未压缩大小（若可得）和压缩后大小；
- reader MUST 对解压大小、压缩比、嵌套层级和 CPU 时间设置上限；
- 已压缩的 JPEG/PNG/PDF/zip-like BLOB SHOULD 标记 `compression=none`，避免浪费 CPU；
- 外层 archive 压缩不能替代 artifact 级属性。云同步需要按 artifact range/内容寻址时，优先不使用一个整体不可随机访问的大压缩包。

### 20.2 Envelope encryption

建议采用“每个备份一个数据密钥、每个 artifact 独立 nonce”的 envelope encryption：

1. 生成随机 256-bit DEK；
2. 数据先压缩，再用 AEAD 加密；
3. 每个 artifact 使用唯一 nonce，并将 `backup_id`、`relative_path`、`format_version`、plaintext hash/length 绑定为 AAD；
4. DEK 由以下方式之一包装：
   - 用户密码通过 Argon2id 派生 KEK；
   - OS keychain/secure enclave 提供 KEK；
   - 外部 KMS 对 DEK wrap；
5. Manifest 只保存算法、KDF 参数、salt、key id、wrapped DEK、nonce 规则和 AAD 版本，不保存密码、KEK、未包装 DEK 或私钥。

推荐 AEAD：

- AES-256-GCM：硬件加速普遍，生态成熟；
- XChaCha20-Poly1305：长 nonce、软件环境表现稳定；
- ODBF 1.0 可以把其中一个设为 REQUIRED、另一个为 OPTIONAL，但最终需由安全评审决定。

nonce 绝不能在同一 key 下复用。断点重试不能在相同 nonce 下重写不同 plaintext；最安全做法是重新生成 artifact encryption id/nonce，再原子替换 staging 引用。

### 20.3 Hash 的明文/密文语义

加密 artifact SHOULD 同时支持：

- `digests.stored`：对包内精确字节计算，无需解密即可验证传输完整性；
- `digests.content`：对 envelope encryption 之前的已编码/已压缩内容计算，解密后验证内容身份。

但 content hash 会泄露相同内容关系。高隐私策略 MAY 只把 `digests.content` 放入加密 metadata 内，外层只暴露 `digests.stored`。Manifest 必须声明采用的策略，不能让字段含义随实现变化。

### 20.4 签名与密钥轮换

- 签名和加密独立：未加密备份也可以签名，加密备份也不一定有来源签名；
- 推荐 Ed25519 签名 `tree_hash` 上下文，`key-info.json` 保存 public key fingerprint/证书链引用，不保存私钥；
- reader 默认先验证 ciphertext/hash，再解密，再验证 plaintext/object hash；
- key rotation 只重新 wrap DEK 和签署新的 envelope/ref，不应要求重新编码全部 Parquet；
- 修改 envelope 后生成新的 backup version/ref，保留审计链，不能原地改变已发布不可变备份而不留记录。

## 21. 错误、取消、重试、幂等和断点续作

### 21.1 结构化错误

建议稳定错误码：

| 错误码 | 含义 | 默认可重试 |
| --- | --- | --- |
| `not_supported` | adapter/driver 不支持请求能力 | 否；可重新 plan 降级 |
| `permission_denied` | 数据库、文件、KMS 或远端权限不足 | 条件式 |
| `credential_required` | 需要交互提供秘密 | 条件式，不记录秘密 |
| `snapshot_unavailable` | 无法获得所需一致性快照 | 条件式 |
| `snapshot_lost` | 快照事务/slot/driver session 中断 | 通常需重建备份 |
| `schema_changed` | capture 期间 schema epoch 改变 | 通常需重启相关对象/备份 |
| `log_gap` | 增量日志不连续或已回收 | 否；需要新 baseline |
| `format_version_unsupported` | reader 不支持 major/min reader | 否 |
| `path_unsafe` | 路径穿越、链接或 archive 异常 | 否 |
| `checksum_mismatch` | artifact/hash 不匹配 | 重新下载后可重试 |
| `signature_invalid` | 签名或 key trust 失败 | 否，除非 trust 配置有误 |
| `decrypt_failed` | key、tag、AAD 或密文错误 | 条件式；不得区分过多 oracle 信息 |
| `dependency_cycle` | 无法安全拆分依赖循环 | 需人工映射/adapter 改进 |
| `target_conflict` | 目标对象与策略冲突 | 重新 plan |
| `target_incompatible` | 类型、版本、extension 不兼容 | 重新 mapping/target |
| `storage_full` | 本地/远端容量不足 | 清理/扩容后可续作 |
| `rate_limited` | 远端后端限流 | 是，带 backoff |
| `cancelled` | 用户或上层取消 | 由 policy 决定是否保留 staging |
| `internal` | 未分类实现错误 | 谨慎，不盲重试 |

错误对象至少包含 `code`、`stage`、`object_id`/`artifact_id`（如适用）、安全的 message、source 分类、retry hint 和 remediation。禁止把 SQL 行数据、密码、token、完整连接 URI 或加密 key 放入错误链。

### 21.2 取消

取消是状态转换而不是强杀线程：

1. manager 设置 cancellation token；
2. 停止发现新对象和新写入；
3. 让 in-flight driver/stream 收到 cancel；
4. flush 或 abort 当前 artifact；
5. 关闭 snapshot/log slot/transaction；
6. 写 journal 的 cancelled 状态；
7. 按 policy 保留可续作 staging 或安全删除；
8. 最终已发布 ref 不受影响。

在取消确认前 UI 显示“正在取消”，不能立即显示已停止。超时后可以 kill sidecar，但仍需记录资源可能残留并在下次启动进行 cleanup audit。

### 21.3 重试与幂等

- 只对明确可重试的网络、rate limit、远端 multipart 错误做指数退避和 jitter；
- 数据库 snapshot 丢失后不能从中间表继续假装同一一致性边界，应重新开始新 backup id 或显式降级；
- artifact 上传以 content hash/part id 幂等；相同 hash 已存在时用 `HEAD` 校验 size/hash 后复用；
- DDL 操作的幂等由 restore plan + target inspection 决定，不能简单在错误时加 `IF EXISTS`；
- row load checkpoint 必须能区分“服务器已提交但客户端未收到响应”，优先使用 staging table、batch id 或目标端 receipt 去重；
- 重试次数、退避、最终错误都进入 report。

### 21.4 Journal

备份 journal 是 staging 内部实现文件，不属于已发布 ODBF 的必要内容。建议字段：

```text
task_id, backup_id, request_hash, source_fingerprint,
snapshot_identity, phase, object/artifact states,
writer offsets, multipart upload ids, hash states,
created_at, updated_at, cancellation state
```

含 snapshot capability 或 upload id 的 journal 应权限受限并按敏感数据处理。resume 之前重新验证 source、parent、目标输出和已写 artifact hash；不允许把旧 journal 套用到另一实例。

恢复 journal SHOULD 存在 NavOp 安全的任务存储中，而不是修改只读备份包。其 key 至少包含 `backup_tree_hash + target_fingerprint + plan_hash`。

## 22. 云同步与版本管理

### 22.1 内容寻址布局

推荐远端逻辑布局：

```text
remote/
├── objects/
│   └── sha256/
│       ├── ab/cd/<full-hash>
│       └── ...
├── manifests/
│   └── <backup-id>/
│       ├── manifest.json
│       ├── checksums.json
│       └── signature/...
├── refs/
│   ├── latest
│   ├── daily
│   └── database/<source-fingerprint>/latest
├── indexes/
│   └── <source-fingerprint>.json
└── leases/
    └── <writer-id>
```

artifact 使用不可变 content-addressed object；Manifest 引用 logical path 与 content hash。远端 backend 可以把多个 logical path 映射到同一物理对象，但下载后必须能重建标准 ODBF 目录。

### 22.2 `BackupStorage` 最小合同

```rust
#[async_trait]
pub trait BackupStorage: Send + Sync {
    async fn begin_staging(&self, backup_id: Uuid) -> Result<StagingHandle, StorageError>;
    async fn put_part(&self, request: PutPartRequest) -> Result<PutReceipt, StorageError>;
    async fn head(&self, object: &ObjectAddress) -> Result<Option<ObjectMetadata>, StorageError>;
    async fn read_range(&self, request: ReadRangeRequest) -> Result<Bytes, StorageError>;
    async fn commit_manifest(&self, request: CommitManifestRequest) -> Result<(), StorageError>;
    async fn compare_and_swap_ref(&self, request: RefUpdateRequest) -> Result<(), StorageError>;
    async fn abort_staging(&self, handle: StagingHandle) -> Result<(), StorageError>;
    async fn list_reachable(&self, root: &BackupRef) -> Result<ReachabilityGraph, StorageError>;
}
```

不同 backend 可映射到：

- 本地目录/外接磁盘；
- SFTP；
- WebDAV；
- S3-compatible object storage；
- 未来的企业 backup server。

认证是 storage backend 的 Host 配置，不写入 ODBF。

### 22.3 上传和发布协议

1. 上传 content-addressed artifact；存在时 `HEAD` + hash/size 验证后跳过；
2. 支持 multipart 和断点，part receipt 写 journal；
3. 上传 `manifest.json`、`checksums.json` 和 signature；
4. 远端重新读取关键 range/hash 做抽验；
5. 以 compare-and-swap 更新 `refs/latest`；
6. 只有 ref 提交成功才对普通列表可见；
7. 并发 writer 通过 lease 或 optimistic concurrency 解决，不覆盖彼此 Manifest。

### 22.4 版本 DAG、保留和 GC

每个备份不可变，`parent` 形成 DAG（正常增量链通常是线性）。保留策略可以表达：

- 最近 N 份；
- 每日/每周/月度 full；
- 最短 PITR 窗口；
- legal hold；
- 已 pin 的 release/ref；
- 依赖链可达性。

GC 只能删除从所有 live refs、parent chain、legal hold 不可达的 artifact，并需要 grace period。删除 parent 前必须先 rebase/compact 增量链或连同依赖子链一起删除。GC report 要列出将删除的备份、对象、字节和仍被引用原因，并支持 dry-run。

### 22.5 Cloud 与格式解耦

云索引、lease、multipart id 都不是 ODBF 恢复必要输入。将一个备份下载成目录后，第三方 reader 不需要远端 API 即可校验和恢复。反之，第三方实现只要遵循 ODBF manifest 和 artifact 语义，就可以使用自己的同步协议。

## 23. 安全与隐私

### 23.1 备份包是不可信输入

reader 在连接目标数据库之前必须完成：

- 路径规范化和 archive 安全检查；
- 文件数、总大小、单文件大小、解压比、JSON 深度/字符串长度上限；
- JSON schema、版本、UUID、枚举和引用完整性检查；
- hash 和可选签名验证；
- 加密 tag/AAD 验证；
- DDL 和 opaque body 的 policy 检查；
- 依赖图资源上限，防止恶意 DAG 消耗 CPU/内存。

恢复不能把 artifact 路径传给 shell，也不能根据 `README.txt` 执行动作。

### 23.2 数据和凭证

- Manifest 不含数据库密码、token、SSH 私钥、KMS secret、cookie 或完整带凭证 URI；
- source 地址默认可脱敏为 fingerprint；是否保留 hostname/user name 由隐私策略决定；
- 临时文件目录权限应仅当前用户可读，删除时遵循平台可实现的安全清理语义；SSD 上不承诺多次覆盖保证；
- tracing 默认只记录对象 id、类型、行数/字节数和安全错误码，不记录行内容；
- progress/event 经过相同脱敏规则；
- OS keychain/KMS 获取秘密时使用短生命周期 handle，不将秘密 clone 到 UI state 或 journal；
- 备份包本身含生产数据，UI 必须提示敏感性，并支持默认加密 policy。

### 23.3 恢复安全

- users/roles/grants/owners、`security_definer`、external function、event scheduler 默认不自动启用；
- 恢复程序对象前展示来源签名状态和 opaque DDL；不受信任包默认禁用 opaque executable object；
- 禁止默认恢复到系统 schema，目标 mapping 需限制 reserved namespace；
- `replace/cascade`、禁用约束/trigger、启用 extension 等高风险动作必须在 plan 中单列；
- adapter 使用参数化查询或可靠的 identifier quoting；不能把对象名称当成普通 SQL 字符串拼接；
- 远端恢复账户遵循最小权限，不建议直接使用超级管理员。

### 23.4 密码学实现要求

- 使用经过审计的 Rust crypto crate，不自创算法；
- 所有 nonce/salt/key 使用 CSPRNG；
- 密钥材料使用 zeroize/受保护容器（在依赖和平台允许时）；
- KDF 参数写入 envelope 并设最低安全阈值，同时允许未来调高；
- 解密错误消息避免成为密码/AEAD oracle；
- 设计和实现合入前需要独立安全 review 和加密 golden vector。

## 24. 性能、背压与可观测性

### 24.1 流水线

```text
DB snapshot reader
  → bounded row/Arrow batches
  → logical type normalization / LOB split
  → Parquet encoder
  → compression
  → plaintext hash
  → optional encryption / ciphertext hash
  → storage multipart writer
```

每个 stage 都接收 cancellation token 和预算。`BackupLimits` 至少包含：

- 最大并发表、每表并发读取数；
- batch rows/bytes；
- 全局内存预算；
- open file/stream 上限；
- compression/hash worker 数；
- remote in-flight bytes；
- 单 blob、单对象、总备份上限；
- snapshot keepalive 与 stall timeout。

当 storage 变慢时，背压必须一路传回数据库 reader；不允许用无界 channel 吸收全部行。adapter 可在安全前提下暂停 cursor，不能暂停时需要调低读取 concurrency。

### 24.2 并行性

- 只有数据库 snapshot 支持多个共享 reader 时才能并行表读取；
- 同一表可按稳定 key range 分片，前提是 adapter 能保证无重叠/无缺口；
- 元数据、data、index/constraint restore 的并行度分别控制；
- SQL Server/PostgreSQL/MySQL 的锁和长事务影响不同，默认值按 adapter 提供；
- SQLite 单文件连接默认保守串行，避免与写者争用；
- hash/compression CPU 池与数据库 async runtime 隔离。

### 24.3 指标和日志

建议指标：

```text
backup_rows_total{task,object_kind}
backup_bytes_read_total
backup_bytes_written_total
backup_compression_ratio
backup_artifact_seconds
backup_snapshot_age_seconds
backup_inflight_bytes
backup_retry_total{code}
restore_rows_total
restore_verification_failures_total
cloud_deduplicated_bytes_total
```

指标 label 不包含数据库名称、表名、hostname 或用户数据，避免高基数和隐私泄露。对象级诊断使用 task-scoped 日志或 report，而不是全局 metrics label。

## 25. UI、CLI 与自动化集成

### 25.1 备份 UI

建议向导：

1. **选择范围**：连接、database/schema、对象种类、是否包含数据/权限；
2. **一致性与增量**：显示 adapter capability、权限和降级后果；
3. **格式与存储**：Parquet/兼容格式、分片、压缩、本地/云；
4. **安全**：加密方式、签名、key source；不让密码进入普通文本配置；
5. **计划预览**：对象数、估算大小、预计影响、warning；
6. **执行**：phase、对象、行/字节、速率、ETA、取消；
7. **报告**：一致性、可恢复性、失败/partial、tree hash、验证和目标路径。

不能用单一绿色“完成”掩盖 `best_effort`、partial 或签名未验证。状态建议区分：`complete_verified`、`complete_unverified`、`partial`、`cancelled`、`failed`。

### 25.2 恢复 UI

- 打开备份后先离线 inspect，不立即连接/修改目标；
- 对象树支持选择、依赖自动补全和“为什么被选中”说明；
- 展示 source→target mapping、冲突策略和不可逆动作；
- 增量链用时间线显示 full、increment、gap、可恢复点；
- 必须提供 dry-run、只验证、从 checkpoint 继续；
- 恢复权限/程序对象时显示额外安全确认；
- 完成页可导出机器可读和人类可读 report。

### 25.3 CLI 草案

```text
navop backup plan      --connection <id> --output <location> [...]
navop backup create    --connection <id> --output <location> [...]
navop backup inspect   <backup-location>
navop backup verify    <backup-location> [--deep]
navop backup sync      <backup-location> <remote>
navop backup gc        <remote> --policy <file> --dry-run

navop restore plan     <backup-location> --target <connection-id> [...]
navop restore run      <backup-location> --target <connection-id> --accept-plan <hash>
navop restore verify   <backup-location> --target <connection-id>
```

CLI 默认不从命令行参数接收密码，避免 shell history 泄露；使用交互输入、stdin fd、OS keychain 或 secret reference。机器模式输出 JSON progress/event，并保持错误码稳定。

### 25.4 Public MCP / Agent

自动化接口只调用 manager，不直接暴露任意 DDL/文件路径：

- `backup.plan` 为只读；
- `backup.create`、`restore.run`、`backup.gc` 是有副作用操作，遵循现有 permission policy；
- `restore.run` 需要 plan hash 和目标确认；
- tool result 返回 task id、report location、结构化 warning，而不是大量行数据；
- 长任务通过有界事件/轮询获取进度，可取消；
- secret 通过 Host credential flow 获取，不进入模型上下文。

## 26. 测试策略与验收标准

### 26.1 格式和模型测试

- ODBF JSON schema 和 serde round-trip；
- canonical JSON golden vector，跨 Rust/JavaScript/Python 得到相同 hash；
- unknown minor fields/extension round-trip；
- unsupported major/min reader version 拒绝；
- manifest/artifact/reference/DAG consistency；
- checksum/tree hash/Ed25519/AEAD/KDF golden vectors；
- 压缩/解压边界和资源限制；
- path traversal、symlink、zip bomb、重复 path、Unicode/大小写碰撞；
- fuzz Manifest、metadata、Parquet/Arrow reader 和 archive parser；
- 加密错误不泄露过细 oracle 信息。

### 26.2 Adapter contract 测试

每个数据库 adapter 使用同一 contract suite：

1. capability 与实际行为一致；
2. snapshot 内跨表一致性；
3. schema 在 capture 中变化时正确失败/警告；
4. 所有支持对象的 metadata 可序列化；
5. Decimal、binary、Unicode、NULL、时区、LOB、JSON 和 custom type 无损或明确标记 lossiness；
6. cursor/stream 有界、可取消、关闭后资源释放；
7. 无权限返回 `permission_denied/not_supported`，不伪装为空对象；
8. sidecar 崩溃、timeout、Host 断线时事务/slot/临时文件清理；
9. SSH tunnel 路径不向 sidecar 泄露 SSH secret；
10. database Future 只在 Tokio runtime 执行。

### 26.3 备份—恢复矩阵

至少覆盖：

| 维度 | 组合 |
| --- | --- |
| 数据库 | MySQL 当前支持版本、PostgreSQL 当前支持版本、SQL Server 当前支持版本、SQLite 当前支持版本 |
| 大小 | 空库、单行、边界 row group、多 part、超大 LOB |
| 对象 | table/view/materialized view/type/function/procedure/trigger/sequence/foreign key/cycle |
| 一致性 | transaction、online backup、best effort 降级、schema change |
| 格式 | Parquet、Arrow IPC fallback、NDJSON fallback |
| 安全 | 未加密、密码加密、KMS wrap、签名、错误 key、篡改 |
| 恢复 | 全库、schema、单表、rename、skip/replace/merge、resume |
| 存储 | 本地、模拟 SFTP/WebDAV/S3、断网、multipart resume、CAS 冲突 |
| 增量 | 连续链、日志 gap、父 hash 不匹配、schema event、无主键整表替换 |

优先做同数据库同版本的无损 round-trip；跨数据库恢复属于额外能力，必须有明确类型转换预期，而不是复用同一“无损”断言。

### 26.4 故障注入

在以下时点注入 crash/cancel/disk full/network reset：

- snapshot 获取后；
- metadata 写一半；
- Parquet row group/LOB 写入中；
- multipart 某 part 后；
- checksums/签名之间；
- Manifest 上传完成但 ref 尚未更新；
- table batch 已提交但 receipt 丢失；
- index/foreign key 创建中；
- 增量事务边界中。

验收要求是：不会发布引用缺失 artifact 的完整备份；resume 不重复/遗漏已确认数据；无法判定提交状态时返回结构化冲突而非盲重试。

### 26.5 性能测试

- 1 GiB、100 GiB、1 TiB 级模拟数据的吞吐和峰值内存；
- 可压缩/不可压缩、窄表/宽表、small/large LOB；
- 慢数据库、慢磁盘、慢对象存储下背压；
- 并发表数与数据库负载/锁影响；
- Parquet row group/part 大小 benchmark；
- 增量 dedup ratio、恢复随机读取和 range GET；
- UI progress 在高频事件下保持响应且不无界积压。

具体性能阈值由基准硬件与产品目标另行确定，但峰值内存必须由 `BackupLimits` 有界控制。

### 26.6 ODBF 1.0 最小验收标准

实现被声明为 ODBF 1.0 full backup 可用前，MUST 满足：

- 一个公开 schema/说明足以让独立测试 reader 枚举并校验备份；
- Manifest、对象 metadata、logical schema、artifact 和 checksums 引用闭合；
- 路径、版本、hash、安全资源限制测试通过；
- 支持至少 table + index + PK/FK + view + sequence/identity 的阶段恢复；
- 数据覆盖 NULL、binary、Decimal、Unicode、timestamp/timezone 和大 LOB；
- 备份写入流式有界，取消能释放 snapshot/stream；
- 同数据库同版本 full backup→restore→deep verify 通过；
- partial/best-effort/unsigned 状态不会显示成 complete verified；
- 文档和 sample fixture 不包含凭证或私钥；
- 第三方或独立进程 reader 不依赖 NavOp 私有 API 即可完成格式验证。

原生日志增量/PITR 需单独通过：连续性、事务边界、schema event、gap fail-closed、retention/slot cleanup 和目标位点验证，不能随 full backup 一起默认宣称支持。

## 27. 分阶段实施路线

### Phase 0：规范和核心 reader/writer

- 冻结 ODBF 1.0 draft、JSON schema、logical type 和目录规范；
- 新建 `db-backup-core`，实现 model、canonical JSON、local storage、checksum/tree hash；
- 提供 golden fixture、独立 inspect/verify CLI；
- 完成安全路径和资源上限测试；
- 不连接数据库即可读/写/校验 sample package。

**退出条件**：独立 reader 可以完整枚举对象/artifact，检测篡改和坏引用。

### Phase 1：同库 Full Backup PoC

- 新建 `db-backup` manager 和 adapter contract；
- 先接入 SQLite Online Backup/read snapshot 与 PostgreSQL/MySQL transaction snapshot；
- 表 metadata、logical schema、Parquet、LOB、进度、取消、staging/atomic publish；
- 只实现同数据库同版本恢复，权限对象默认排除；
- SQL Server adapter 先完成 capability/preflight 和 metadata contract，再按受支持环境启用 snapshot。

**退出条件**：目标数据库矩阵中的基础对象可 full round-trip，所有降级可见。

### Phase 2：完整对象 DAG 与恢复体验

- view/function/procedure/trigger/type/event、dependency graph、cycle split；
- restore plan/hash、mapping、conflict policy、checkpoint/resume；
- deep verification 和 machine-readable report；
- UI/CLI/Public MCP 接入 manager；
- opaque DDL 安全 policy。

**退出条件**：全库/部分对象恢复可 plan、可续作、可验证，失败报告可操作。

### Phase 3：加密、签名和云存储

- zstd/gzip policy、AEAD envelope、Argon2id/KMS wrapping、Ed25519；
- SFTP/WebDAV/S3-compatible backend；
- multipart resume、CAS ref、content-addressed dedup、retention/GC；
- key rotation 和 cloud fault injection。

**退出条件**：断网/重试/并发 writer 不损坏已发布备份，密钥不落普通配置或日志。

### Phase 4：增量和 PITR

- 先实现 metadata delta + 有稳定 PK 的 Snapshot + Diff；
- MySQL binlog、PostgreSQL logical decoding、SQL Server 受支持 log chain 逐一作为独立 capability 落地；
- SQLite 采用 snapshot/diff，不虚构通用日志重放；
- chain compaction、gap diagnosis、restore timeline。

**退出条件**：每个已声明 native incremental adapter 都通过连续性、事务、DDL、gap、retention 和恢复点 contract suite。

### Phase 5：生态与长期兼容

- 发布 ODBF schema、示例和兼容策略；
- 提供轻量 standalone verifier/restore SDK；
- 添加更多数据库/对象/存储 backend；
- 建立跨版本 fixture 仓库，保证旧备份长期可读；
- 评估格式标准化或由多个实现共同维护。

## 28. 首版范围与能力矩阵

为避免“文档写了支持”被误解为“所有数据库都无条件具备”，产品和 adapter 应分别展示目标、当前实现和本次实际能力。建议用三态：`supported`、`supported_with_limits`、`not_supported`，并附 reason。

ODBF 1.0 的建议产品范围：

| 能力 | 首版承诺 | 后续扩展 |
| --- | --- | --- |
| 公开目录/Manifest/metadata/checksum | 是 | schema registry、更多实现 |
| Table 数据 | Parquet 默认；Arrow/NDJSON fallback | 更多 extension type |
| View/index/PK/FK/sequence | 是，按 adapter capability | 更复杂 expression/index |
| Function/procedure/trigger/event | 保存定义；恢复受版本/权限约束 | 结构化跨库转换 |
| Full snapshot | 四个目标数据库均提供 adapter；一致性级别逐实例协商 | 更多数据库 |
| 部分对象恢复 | 是；自动补依赖 | 跨库 mapping assistant |
| Snapshot + Diff | 仅稳定 row identity；否则整表替换 | CDC/change tracking 优化 |
| Native incremental/PITR | 不作为统一首版承诺；逐 adapter 开启 | MySQL/PostgreSQL/SQL Server |
| 压缩 | zstd；gzip fallback | 算法扩展 |
| 加密/签名 | 可在 Phase 3 启用 | 企业 KMS/证书策略 |
| 云同步 | 本地先行；远端逐 backend | 分布式索引、跨区复制 |
| users/roles/grants | 默认不恢复 | 经安全评审的显式模式 |

每份 Manifest 只描述“这一次真实做到了什么”。即使某 adapter 理论上支持 native log，本次因权限不足而退化，也必须写本次实际 `capabilities.native_log=false` 和 warning。

## 29. 关键设计决策

### D1：目录是规范真相，archive 只是封装

**选择**：ODBF reader 处理逻辑目录；ZIP/TAR/Zstd/object store 都映射为同一个 `ArtifactStore`。

**原因**：支持随机读取、部分恢复、云去重和第三方实现，避免把单一容器格式锁死为产品协议。

### D2：Parquet 是默认数据格式，logical schema 独立存在

**选择**：表数据默认 Parquet，同时保存 `schema.arrow.json`/ODBF logical schema。

**原因**：Parquet 有成熟压缩和分析生态，但 physical schema 无法单独承载所有数据库类型、时区和原始属性。

### D3：编排层不扩张现有 `DatabasePlugin`

**选择**：新建 `db-backup-core`/`db-backup`，通过 `BackupSource`、`SnapshotSession`、`RestoreTarget` adapter 包装当前连接和 plugin。

**原因**：避免 `DatabasePlugin` 继续承载格式、云、加密、DAG、增量等不属于数据库基础抽象的职责，并保持依赖方向清晰。

### D4：增量能力逐 adapter 声明

**选择**：native log、Snapshot + Diff、metadata delta 是不同 capability；不设计一个假装统一的“重放 SQL”接口。

**原因**：binlog、WAL、transaction log 和 SQLite journal 的授权、语义和恢复机制差异很大。统一结果模型可以相同，读取/重放实现不能伪统一。

### D5：恢复以 plan hash 和 phase DAG 驱动

**选择**：任何修改目标的恢复先 dry-run，生成 plan hash；执行时校验 backup/target/mapping 没有变化。

**原因**：对象依赖、冲突、权限和不可回滚操作都需要在执行前审计；简单按文件名执行 SQL 不安全，也无法可靠续作。

### D6：校验清单排除自身，签名 tree hash

**选择**：`checksums.json` 包含 Manifest/payload hash，但不包含自身；对 canonical entries 计算 tree hash，可选签名该上下文。

**原因**：消除 Manifest/checksum 自引用循环，并让无签名和有签名实现都能明确验证。

### D7：不可变备份 + 内容寻址

**选择**：发布后不原地修改；新增增量、重加密、修复或压缩都生成新版本/ref。

**原因**：便于审计、并发同步、去重、回滚和安全 GC。

## 30. 未决问题

以下问题在实现相应 Phase 前必须形成 ADR 或协议决议：

1. **名称与治理**：ODBF 名称是否与现有公开格式冲突；schema、规范和兼容 fixture 放在哪个独立仓库；
2. **JSON schema 发布**：使用 JSON Schema 2020-12 还是自定义 schema 版本；未知字段 round-trip 的具体保证；
3. **Canonicalization**：直接采用 RFC 8785/JCS，还是对大整数、timestamp、binary 引用增加 ODBF profile；
4. **Logical type**：完全采用 Arrow schema、Arrow extension type，还是维护更稳定的 ODBF type IR；
5. **Parquet baseline**：最低 Parquet format/version、默认 codec、row group/part 大小和 statistics 隐私 policy；
6. **LOB 阈值**：按列/对象/全局如何配置，blob 是否跨表/跨备份去重；
7. **加密 REQUIRED 算法**：AES-256-GCM 与 XChaCha20-Poly1305 的互操作基线；密码 KDF 最低参数；
8. **明文 content hash 隐私**：远端是否暴露跨备份相同内容关系；高隐私模式如何影响 dedup；
9. **签名信任**：个人 key、组织 CA、TOFU 或 KMS signing 的 UI/CLI trust model；
10. **安全对象**：users/roles/grants/owners 的公开 schema、默认过滤和跨实例映射；
11. **Opaque DDL**：允许执行的默认 trust policy、SQL parser 限制和审核 UI；
12. **跨数据库恢复**：是 ODBF 1.x 核心能力还是上层 migration profile；哪些转换可称为无损；
13. **SQL Server 日志**：选择官方 backup API、外部 sidecar 还是只支持 Snapshot + Diff；目标版本/edition/权限矩阵；
14. **PostgreSQL 增量**：优先 logical decoding 还是 raw WAL archive；replication slot 生命周期与生产安全默认值；
15. **MySQL binlog**：row image、DDL、GTID domain 和 statement event 的接受策略；
16. **SQLite diff**：page-level snapshot 与 object/row diff 哪个作为首个可维护实现；
17. **分布式数据库**：多 shard/replica snapshot、clock/transaction boundary 如何进入 consistency 模型；
18. **恢复 checkpoint**：不同数据库的 batch exactly-once receipt 如何统一，何时采用 staging table；
19. **对象 identity**：native id 不稳定或 clone/restore 后变化时，rename detection 的置信模型；
20. **GC/compact**：长增量链的 compaction 是否生成新 full、如何保留原签名和审计链；
21. **依赖与许可证**：Arrow/Parquet/crypto/KMS/S3 crate 的体积、feature、许可证和最低 Rust 版本影响；
22. **独立工具**：是否发布不依赖 NavOp UI 的 `odbf` CLI/SDK，作为开放格式可恢复性的硬性验收工具。

## 31. 附录：恢复报告最小结构

机器可读 `RestoreReport` 可以使用以下结构；报告不必写回原备份目录：

```json
{
  "task_id": "restore-task-id",
  "backup_id": "backup-id",
  "backup_tree_hash": "sha256:...",
  "target_fingerprint": "sha256:...",
  "plan_hash": "sha256:...",
  "started_at": "2026-07-26T12:00:00Z",
  "finished_at": "2026-07-26T12:10:00Z",
  "status": "completed_verified",
  "restore_point": {"kind": "snapshot", "value": "snapshot-id"},
  "phases": [
    {
      "phase": "data",
      "status": "completed",
      "objects_total": 12,
      "objects_completed": 12,
      "rows_written": 100000,
      "bytes_read": 8388608
    }
  ],
  "objects": [
    {
      "object_id": "object-id",
      "target_name": "public.users",
      "action": "create_and_load",
      "status": "completed",
      "verification": "passed",
      "warnings": []
    }
  ],
  "warnings": [],
  "verification": {
    "schema": "passed",
    "row_counts": "passed",
    "deep_checksums": "not_requested"
  }
}
```

报告状态必须区分：

- `completed_verified`；
- `completed_unverified`；
- `completed_with_warnings`；
- `restored_with_verification_errors`；
- `partial`；
- `cancelled`；
- `failed`。

## 32. 结论

NavOp 备份功能应围绕“**公开格式 + 对象化 artifact + 依赖 DAG + 显式 capability + 可验证恢复**”建设：

- ODBF 目录和 Manifest 是开放、跨平台的长期合同；
- Parquet 是默认表数据格式，Arrow IPC 是传输/流式 fallback，CSV/SQL 只承担兼容角色；
- `db-backup-core` 和 `db-backup` 隔离格式、编排与现有连接抽象；
- full backup 先解决一致性、类型保真、取消和恢复验证，再逐数据库增加原生日志；
- 增量链遇到 gap、无稳定 row identity 或权限不足时必须诚实降级；
- 校验、签名、加密、不可变发布和内容寻址为云同步与长期版本管理提供统一基础；
- 第三方无需 NavOp 私有协议，就能根据 Manifest 和对象文件实现 verifier 或恢复工具。

该方案的首要成功标准不是“生成了若干文件”，而是：在明确的一致性边界上产生可独立验证的备份，并能通过计划化、可审计、可续作的流程恢复到目标数据库。
