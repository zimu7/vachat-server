# 智能体消息内容类型

VaChat 聊天消息的内容（`ChatMessageContent`）由三部分组成：

- `content_type`：内容类型（MIME 风格字符串），决定前端如何解析与渲染。
- `content`：主要内容载荷（字符串）。
- `properties`：结构化扩展属性（可选，JSON 对象）。

智能体（bot）通过发送消息接口（`POST /api/user/:uid/send`、`POST /api/group/:gid/send`，或 bot 专用的 `POST /bot/send_to_user/:uid`、`POST /bot/send_to_group/:gid`）发送消息时，按 HTTP `Content-Type` 头区分内容类型。前端据此区分智能体的最终回答与过程性内容（思考、工具调用、工具结果），实现差异化展示。

## 既有内容类型

| content_type | content | properties | 说明 |
|---|---|---|---|
| `text/plain` | 纯文本正文 | 可选（`X-Properties` 透传） | 普通文本消息 |
| `text/markdown` | Markdown 正文 | 可选（`X-Properties` 透传） | Markdown 消息 |
| `vachat/file` | 文件路径 | `name`、`content_type`、`size` | 文件消息 |
| `vachat/archive` | 归档 id | 可选 | 归档消息 |

## 智能体过程性内容类型

`vachat/agent/*` 命名空间下的内容类型用于承载智能体的过程性内容。这些内容**不触发推送通知**，前端可对思考折叠、工具调用/结果卡片化等差异化展示。它们沿用标准消息生命周期，支持 edit / reply / like / delete（如思考可经 `PUT /message/:mid/edit` 增量流式更新）。

### `vachat/agent/thinking`

智能体的思考/推理过程。

- `content`：思考文本（纯文本）。
- `properties`：可选，可含 `signature`（透传自 `X-Properties`）。

请求示例：

```http
POST /api/user/:uid/send
X-API-Key: <token>
Content-Type: vachat/agent/thinking

analyzing the request
```

### `vachat/agent/tool_use`

智能体发起的工具调用。

- `content`：工具名。
- `properties`：
  - `id`：工具调用标识，用于与对应的 `tool_result` 关联。
  - `input`：工具输入（JSON 对象）。

请求示例（body 为 JSON）：

```http
POST /api/user/:uid/send
X-API-Key: <token>
Content-Type: vachat/agent/tool_use

{"name":"search","id":"tu_1","input":{"query":"rust"}}
```

### `vachat/agent/tool_result`

工具执行结果。

- `content`：对应 `tool_use` 的 `id`。
- `properties`：
  - `result`：工具结果（字符串）。
  - `is_error`：是否为错误（布尔，可选，缺省视为未置位）。

请求示例（body 为 JSON）：

```http
POST /api/user/:uid/send
X-API-Key: <token>
Content-Type: vachat/agent/tool_result

{"tool_use_id":"tu_1","result":"found 3 hits","is_error":false}
```

## 命名空间可扩展性

`vachat/agent/*` 命名空间可扩展。未来新增过程类型（如 `vachat/agent/status`、`vachat/agent/citation`）只需新增 `content_type` 取值，结构化数据承载于 `properties`，无需变更消息结构或数据库 schema。未知的 `vachat/agent/*` 取值同样不触发推送通知。

## Matrix 智能体接入的内容转换

QwenPaw、Hermes、cc-connect 等智能体通过 Matrix 协议接入，其消息由 vachat-server 的 Matrix 桥接（`src/api/matrix/rooms.rs`）接收并统一转换成 `vachat/agent/*` 后再广播给前端。转换逻辑在 `src/api/matrix/agent_convert.rs`，分两层：

### 按 agent 类型派发

每种智能体的消息格式不同，转换规则也不同。给 bot 用户设置 `agent_type` 字段（管理端创建/更新 bot 时设置；目前已知取值 `qwenpaw`、`hermes`、`cc_connect`）后，桥接按该字段选择对应的推断器。未设置 `agent_type` 的 bot 不做推断，消息回落为散文。

### Layer 1：显式类型映射（推荐）

智能体在 Matrix 事件里主动告知类型时，直接按类型映射，任何配合的智能体通用。识别两种信号（任选其一）：

- 自定义 `msgtype`：`"msgtype": "vachat.agent.thinking"` / `vachat.agent.tool_use` / `vachat.agent.tool_result`（或任意 `vachat.agent.<type>`）。
- 自定义字段：`"vachat_content_type": "vachat/agent/<type>"`。

字段形态与 bot HTTP API 一致：

| 类型 | content | properties |
|---|---|---|
| `vachat/agent/thinking` | body（思考文本） | 可选 |
| `vachat/agent/tool_use` | `name` | `id`（可选）、`input`（可选，JSON） |
| `vachat/agent/tool_result` | `tool_use_id` | `result`（可选）、`is_error`（可选） |

### Layer 2：内容特征推断（按 agent_type）

智能体未带类型信号时，按 `agent_type` 对应的推断器从 `body` 推断。**QwenPaw**（`agent_type=qwenpaw`）的格式：

- `🔧 **<工具名>**` 后跟 ``` 代码块 -> `vachat/agent/tool_use`（`content`=工具名，`properties.input`=代码块内容，能解析成 JSON 用对象、否则用字符串）。
- `✅ **<工具名>**` 后跟 ``` 代码块 -> `vachat/agent/tool_result`（`content`=工具名，`properties.result`=代码块内容）。
- 其余散文 -> `text/markdown`（事件带 `format` 时）或 `text/plain`。

**cc-connect**（`agent_type=cc_connect`，Claude Code 等）的格式——每类消息以一个 emoji 开头：

- `💭 <思考文本>` -> `vachat/agent/thinking`（`content`=去掉 `💭 ` 前缀的思考文本，可跨行）。
- `🔧 **Tool #<序号>: <工具名>**` 一行，后跟 `---` 分隔行和工具输入 -> `vachat/agent/tool_use`（`content`=工具名，去掉 `Tool #<序号>: ` 前缀；`properties.input`=输入，优先取 ``` 代码块内容，其次取 `` `内联代码` ``，否则取 `---` 后的纯文本）。
- `🧾` 一行，后跟 `🟢 Status: <ok|error>`、`🔢 Exit: <码>` 和 ``` 代码块 -> `vachat/agent/tool_result`（`content`=空，cc-connect 的结果不带 id/工具名，前端按顺序与上一个 `tool_use` 关联；`properties.result`=代码块内容，`properties.is_error`=`Status` 不为 `ok` 时为 `true`）。
- `❌ Error: <文本>` 与无 emoji 前缀的最终回答 -> 散文（`text/markdown`/`text/plain`），仍触发推送。

cc-connect 的 `💭` 前缀是显式标记，因此其思考可与最终回答区分（区别于 QwenPaw/Hermes）。

Hermes 的推断规则待补（格式样本待采集）；在此之前其消息回落为散文。

### 限制

- **推断的 tool_use / tool_result 没有 `id`**，按工具名软关联（与上文"id 软关联、客户端自行保证"一致）。
- **thinking 是否与最终回答区分取决于智能体**：cc-connect 以 `💭` 显式标记思考，可区分；QwenPaw/Hermes 暂无标记，两者都按 `text/markdown` / `text/plain` 对待。等明确各自的标记后再细化（如序列降级）。
- 显式信号（Layer 1）优先于推断（Layer 2）。

