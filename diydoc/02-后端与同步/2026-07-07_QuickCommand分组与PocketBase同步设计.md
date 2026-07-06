# QuickCommand 分组与同步设计

更新时间：2026-07-07

状态：设计确认中，按最小实现方案推进

## 1. 目标

在现有快捷命令能力基础上，新增“分组”功能，并确保：

- 本地 `SQLite` 可保存分组信息
- `PocketBase` 同步后分组信息不会丢失
- 置顶命令仍然永远显示
- 普通命令区可按分组过滤
- 新增/编辑快捷命令时可直接指定分组

本轮只做最小实现，不做树形文件夹、多级分组、分组管理面板。

## 2. 交互目标

### 2.1 总体布局

快捷命令面板保持三层结构：

1. 搜索条
2. 分组切换入口
3. 命令内容区

其中命令内容区再分两块：

- 置顶命令区：永远显示，不受分组切换影响
- 普通命令区：按当前选中分组过滤

### 2.2 分组入口位置

最终采用“右侧竖栏分组切换”方案，而不是放在主列表横向条中。

原因：

- 分组名可能较长，横向条容易挤占主列表空间
- 右侧竖栏垂直空间更充足，更适合做模式切换
- 不会遮挡置顶命令区

### 2.3 右侧竖栏结构

右侧竖栏一分为二：

- 上半部分：保留现有工具按钮
- 下半部分：显示快捷命令分组切换按钮

分组按钮行为：

- 默认有 `全部`
- 默认有 `未分组`
- 其他按钮来自用户创建过的分组
- 点击某个分组后，仅切换普通命令区
- 置顶命令区保持不变

### 2.4 分组按钮显示

每个分组按钮包含：

- 分组颜色
- 分组名称（必要时允许缩写）
- hover tooltip 显示完整分组名

当前选中分组高亮显示。

### 2.5 新增/编辑弹窗

在快捷命令新增/编辑弹窗中增加：

- `分组`
- `分组颜色`

字段顺序建议：

1. 名称
2. 描述
3. 分组
4. 分组颜色
5. 作用域
6. 命令

默认值：

- 分组：空，表示不分组
- 分组颜色：空，表示使用默认颜色

## 3. 数据结构设计

### 3.1 本地 QuickCommand 模型

在 `QuickCommand` 上新增：

- `group_name: Option<String>`
- `group_color: Option<String>`

约束：

- `None` 或空串表示“不分组”
- `group_color` 为空表示使用默认颜色

### 3.2 SQLite

在 `quick_commands` 表新增两列：

- `group_name TEXT NULL`
- `group_color TEXT NULL`

兼容策略：

- 老数据默认 `NULL`
- 读取时空值视为未分组
- 不需要额外分组表

## 4. PocketBase / 同步边界

### 4.1 当前同步方式确认

当前快捷命令不是单独同步到 `PocketBase` 某个独立 collection 字段模型，
而是统一走：

- `sync_data`
- `encrypted_data`

也就是快捷命令内容先序列化为 `QuickCommandPlainData`，
再整体加密后写入 `sync_data.encrypted_data`。

### 4.2 结论

本轮“快捷命令分组”同步：

- **不需要新增 PocketBase 表字段**
- **不需要改单独 collection schema**
- **只需要扩展加密明文 payload**

也就是说要改的是：

- `QuickCommandPlainData`
- 上传时的序列化内容
- 下载时的反序列化内容

### 4.3 需要补齐的同步字段

在 `QuickCommandPlainData` 上新增：

- `group_name: Option<String>`
- `group_color: Option<String>`

上传：

- 本地 `QuickCommand` -> `QuickCommandPlainData` 时带上两个字段

下载：

- `QuickCommandPlainData` -> 本地 `QuickCommand` 时恢复两个字段

### 4.4 为什么不改 PocketBase schema

因为当前 `PocketBase` 只需要存：

- `data_type`
- `encrypted_data`
- `checksum`
- `version`

快捷命令的业务字段都封装在 `encrypted_data` 里。

所以新增分组信息只会改变：

- 明文 JSON 结构
- 校验和
- 加密后的 payload

不会要求在线库额外加列。

## 5. 列表过滤与显示规则

### 5.1 默认状态

默认选中 `全部`：

- 置顶命令区：显示全部置顶命令
- 普通命令区：显示全部非置顶命令

### 5.2 选择某个分组

例如选中 `Docker`：

- 置顶命令区：仍显示全部置顶命令
- 普通命令区：只显示 `group_name == Docker` 的非置顶命令

### 5.3 未分组

选中 `未分组` 时：

- 置顶命令区：仍显示全部置顶命令
- 普通命令区：只显示 `group_name` 为空的非置顶命令

### 5.4 置顶与分组优先级

优先级规则固定为：

1. 置顶优先显示
2. 分组只作用于普通命令区

不做“置顶区也随分组隐藏”的逻辑。

## 6. 排序兼容

现有排序逻辑保持不变。

普通命令区内继续沿用：

- `pinned DESC`
- `use_count DESC`
- `last_used_at DESC`
- `sort_order ASC`
- `created_at DESC`

由于普通命令区本身已经过滤掉置顶项，实际主要保留：

- `use_count DESC`
- `last_used_at DESC`
- `sort_order ASC`
- `created_at DESC`

## 7. 最小实现范围

第一期只做：

1. `QuickCommand` 增加 `group_name/group_color`
2. SQLite migration 增加对应字段
3. `QuickCommandPlainData` 增加对应字段
4. 上传/下载同步链支持这两个字段
5. 新增/编辑弹窗增加“分组/分组颜色”
6. 右侧竖栏增加分组切换区
7. 普通命令区按当前分组过滤

## 8. 暂不做

以下能力不在第一期范围内：

- 多级分组
- 分组折叠
- 拖拽重排分组
- 分组重命名面板
- 分组删除确认链
- 分组统计持久化
- 分组权限/团队级共享规则

## 9. 实施顺序

建议按下面顺序落地：

1. 文档确认
2. 本地 `QuickCommand` 结构与 migration
3. 同步 payload (`QuickCommandPlainData`)
4. 上传/下载恢复链路
5. 快捷命令新增/编辑弹窗
6. 右侧分组切换 UI
7. 普通命令区过滤显示

## 10. 当前结论

快捷命令分组是一个“本地模型 + 加密同步 payload + UI 过滤显示”的组合能力。

当前架构下：

- SQLite 需要新增字段
- `QuickCommandPlainData` 需要新增字段
- `PocketBase` 不需要额外改 schema
- UI 上优先采用“右侧竖栏分组切换 + 置顶永远显示”的方案
