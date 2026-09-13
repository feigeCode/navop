# Native Resource Workbench：声明与覆盖契约草案

日期：2026-09-09。状态：设计示例，**当前 manifest parser 尚不支持**。配合 `2026-09-09-native-resource-workbench-design.md` 使用；不是可直接替换现有 extension.json 的完整安装包。

## 1. 首版 canonical 示例

以下 JSON 是向当前 ES manifest 的 contributes **合并新增**的贡献。保留原有 runtime、permissions、connections（含表单与 shellViewId）、shellViews 和版本字段；不得用这个片段覆盖整个 manifest。

```json
{
  "contributes": {
    "resourceWorkbenches": [
      {
        "schemaVersion": 1,
        "id": "elasticsearch",
        "title": "Elasticsearch",
        "connectionIds": ["elasticsearch9"],
        "runtimeId": "main",
        "resourceType": "elasticsearch",
        "defaultPage": "overview",
        "operations": {
          "clusterInfo": {
            "mode": "invoke",
            "method": "elasticsearch/cluster/info",
            "requires": ["elasticsearch/cluster/info"],
            "effect": "read",
            "params": {}
          },
          "listIndices": {
            "mode": "invoke",
            "method": "elasticsearch/index/list",
            "requires": ["elasticsearch/index/list"],
            "effect": "read",
            "params": {}
          },
          "indexInfo": {
            "mode": "invoke",
            "method": "elasticsearch/index/get",
            "requires": ["elasticsearch/index/get"],
            "effect": "read",
            "params": {
              "name": {"source": "route", "path": "/name", "type": "string"}
            }
          },
          "indexMapping": {
            "mode": "invoke",
            "method": "elasticsearch/index/mapping",
            "requires": ["elasticsearch/index/mapping"],
            "effect": "read",
            "params": {
              "name": {"source": "route", "path": "/name", "type": "string"}
            }
          },
          "search": {
            "mode": "job",
            "method": "elasticsearch/search/async",
            "requires": ["elasticsearch/search/async"],
            "effect": "read",
            "params": {
              "query": {"source": "input", "path": "/query", "type": "string"}
            }
          }
        },
        "navigation": [
          {"pageId": "overview"},
          {"pageId": "indices"},
          {"pageId": "search"},
          {"pageId": "tasks"}
        ],
        "tree": [
          {
            "id": "indices",
            "title": "Indices",
            "pageId": "indices",
            "children": {
              "operation": "listIndices",
              "itemsPath": "/indices",
              "keyPaths": ["/name"],
              "labelPath": "/name",
              "open": {
                "pageId": "index-detail",
                "route": {
                  "name": {"source": "selection", "path": "/name", "type": "string"}
                }
              }
            }
          }
        ],
        "pages": [
          {
            "id": "overview",
            "title": "Overview",
            "template": "json",
            "renderer": {"kind": "native"},
            "load": {"operation": "clusterInfo"}
          },
          {
            "id": "indices",
            "title": "Indices",
            "template": "collection",
            "renderer": {"kind": "native"},
            "load": {"operation": "listIndices"},
            "collection": {
              "itemsPath": "/indices",
              "keyPaths": ["/name"],
              "pagination": {"kind": "none"},
              "columns": [
                {"id": "name", "title": "Name", "path": "/name", "type": "string"},
                {"id": "health", "title": "Health", "path": "/health", "type": "string"},
                {"id": "docs", "title": "Documents", "path": "/docs", "type": "display"},
                {"id": "size", "title": "Size (bytes)", "path": "/size_bytes", "type": "display"}
              ],
              "open": {
                "pageId": "index-detail",
                "route": {
                  "name": {"source": "selection", "path": "/name", "type": "string"}
                }
              }
            }
          },
          {
            "id": "index-detail",
            "title": "Index",
            "template": "json",
            "renderer": {"kind": "native"},
            "route": {"name": {"type": "string", "required": true}},
            "load": {"operation": "indexInfo"},
            "links": [
              {
                "title": "Mapping",
                "pageId": "mapping",
                "route": {
                  "name": {"source": "route", "path": "/name", "type": "string"}
                }
              }
            ]
          },
          {
            "id": "mapping",
            "title": "Mapping",
            "template": "json",
            "renderer": {"kind": "native"},
            "route": {"name": {"type": "string", "required": true}},
            "load": {"operation": "indexMapping"}
          },
          {
            "id": "search",
            "title": "Search",
            "template": "query",
            "renderer": {"kind": "native"},
            "inputs": [
              {"id": "query", "type": "string", "editor": "text", "default": "*", "required": true}
            ],
            "execute": {"operation": "search"},
            "result": {"template": "json", "valuePath": "/raw"}
          },
          {
            "id": "tasks",
            "title": "Tasks",
            "template": "tasks",
            "renderer": {"kind": "native"},
            "scope": "session"
          }
        ]
      }
    ]
  }
}
```

这里 Overview 故意先用 JSON，不依赖未约定的指标字段。Tasks 读取当前宿主管理的任务，不发一个并不存在的 `elasticsearch/tasks/list`。分页 `none` 只表示现有索引接口没有声明服务器分页，不能据此允许无限载入。

## 2. 最小语义规则

- `id`、operation key、page id、tree id 使用稳定非本地化标识；title 后续接现有本地化体系。所有引用在安装期解析，不能由运行时数据选择跨扩展代码。
- `connectionIds` 是同扩展连接 contribution id，不是用户保存的连接记录 id。每条连接只能绑定一个 workbench；同一 workbench 可适配多条 runtimeId/resourceType 相同的连接贡献。
- `params` 的每个条目是绑定描述，不是已经求值的 provider 参数。`source` 的取值域限定为 literal/input/route/selection/paging；literal 使用 value，其余使用 path。绑定缺失或类型错误时不发 RPC。
- `path`/`itemsPath`/`valuePath`/`keyPaths` 使用单一 JSON Pointer 风格路径契约（根为 `""`，转义 `~0` 与 `~1`）；不支持拼接、脚本、过滤、递归搜索。对象 key 含 `/` 时必须正确转义。
- collection/tree 的 keyPaths 相对于单行对象，itemsPath 相对于解码后的结果 value。复合键编码为类型化 tuple，不能简单用分隔符拼接而产生碰撞。
- `display` 允许 JSON 标量和 null 的安全展示，不自动转换成数字；复杂对象展示受限摘要或 JSON 入口。数值排序必须有显式数值类型和转换契约。
- `load.operation` 必须引用 invoke + read 操作；加载页面不能自动执行写操作或启动用户任务。`execute.operation` 由显式用户动作触发，可以 invoke 或 job。
- Query 的 `result.valuePath` 应用于解码后的 Inline/Blob JSON value，不是 ResultRef 外壳；Tasks 则由宿主 session/task registry 提供数据。
- operation 的 requires 是需要满足的 capability 列表，不自动等同 method；示例采用当前 ES provider 已公布的方法名能力。后续 provider 可有不同能力命名。
- 新增写 operation 时 `effect` 使用 write/destructive/unknown，并定义确认与结果未知策略；其输入必须来自已校验的表单或明确用户动作，不来自任意插值文本。
- operation 只描述调用；toolbar/menu/keybinding 共用命名 action 引用该 operation。通用 refresh/cancel/reconnect/navigation 属于宿主 action，不伪装成 provider method。
- 模板的字段组合必须用 Rust enum/结构做闭合校验；template=json 不接受 collection 配置，Shell-only 页面不接受 native fallback，缺失 defaultPage/operation/viewId 直接报错。
- 新 schema 中的未知字段严格拒绝并标出声明路径；不能套用当前顶层“忽略未知贡献”行为去静默忽略页面内部拼写错误。

## 3. 页面级 Shell 覆盖

当新的 `search-editor` Shell contribution 与 borrowed-mount/workbench host module 已实现并声明匹配的宿主版本后，可以只替换上面 search 页的 renderer：

```json
{
  "kind": "shell",
  "viewId": "search-editor",
  "fallback": "native"
}
```

其余 native template、inputs、execute、result 保留，原生仍是可用退路。viewId 必须在同一扩展的 shellViews 中存在，且通过嵌入兼容性校验；当前 ES 的 `explorer` 不是这个新 view。

Shell-only 页面使用 `renderer: {kind: shell, viewId: ...}` 且不声明 template/native fallback；宿主仍提供页标题、连接状态、错误与禁用说明。这不是新增 `template: custom_shell`，避免把业务模板和渲染技术混成一个枚举。

新 Shell 页面不需要再声明一份 tree/header/table。需要时只消费宿主的输入、选择、操作与会话快照。工作台根的 renderer 和页面 renderer 不互相嵌套创建两套 session。

## 4. 工作台主体覆盖

后续阶段可在 workbench 根声明：

```json
{
  "bodyRenderer": {
    "kind": "shell",
    "viewId": "advanced-workspace",
    "fallback": "native"
  }
}
```

原生 pages/navigation/tree 仍必须有效；bodyRenderer 替代工作台主体，不替代连接 Tab/Header、安全操作、任务入口和切回原生入口。启用需要用户允许。该模式激活时不同时挂载原生 page renderer 或各页面的 Shell mount；切回时重新按原生导航恢复页面，避免重复订阅和请求。

## 5. 不在首版示例里伪造的能力

Server cursor pagination、动态嵌套树、参数表单、编辑/保存/冲突、event 开流配置、metrics 和双向终端仍需各自的 typed schema、provider fixture 和安全测试。schemaVersion=1 的首发支持集合应按实现阶段明确列出；未实现的字段/模板不对扩展作者宣称可用。

可以先发布只含 collection/json/query/tasks 的有限版本，后续对需要新模板的插件提升宿主要求。不让 schema 接受一个字段但 renderer 只显示空白，也不靠“默认退回 JSON”掩盖不支持的危险操作。
