## Why

vachat-server 是一个面向私人使用的应用，用户规模小且彼此可信。当前联系人列表是"按需添加才可见"的模型：只有在 `contacts` 表中显式建立了关系行（add/block）的用户，才会出现在 `GET /contacts` 的返回中；没有任何记录的用户对调用者不可见。这与"私人应用，联系人默认都应可见"的预期不符，导致每个新用户都要手动添加才能看到。

同时，"屏蔽会话"与"删除联系人"此前被合并成了同一个动作：屏蔽（`status=2`）既隐藏用户又拦截私信，而 `remove` 在"默认全可见"模型下退化为无效操作（删行回到 default-visible），导致前端移除了"删除联系人"按钮。二者应拆为两个独立功能：屏蔽只管发消息（仍可见），删除只管列表可见性（从列表隐藏）。

## What Changes

- **BREAKING**：`GET /contacts`（`get_contacts`）返回逻辑由"仅返回显式添加/屏蔽的联系人"改为"返回所有未删除的用户（排除自己）"。
- **屏蔽（`status == 2`）语义变更**：屏蔽用户**仍出现在** `GET /contacts` 列表中（`status` 为 `"blocked"`），屏蔽只用于拦截私信，不再隐藏用户。
- **删除联系人（`status == 3`，新增）**：`POST /update_contact_status` 的 `remove` 动作由"删除行（回到默认可见）"改为"upsert 一条 `status = 3` 的记录"，使该用户从 `GET /contacts` 列表中隐藏。删除是唯一令用户从列表不可见的手段。
- 列表中每项的状态字段：无 `contacts` 记录 -> `"default"`；`status=1` -> `"added"`；`status=2` -> `"blocked"`；`status=3` -> 不出现在列表。
- `unblock` 语义保持：取消屏蔽后该用户回到"默认可见"状态（移除 `contacts` 记录行），而非置为 `added`。
- 私信屏蔽拦截逻辑（`message.rs` 中 `status == 2` 拒收）保持不变；删除（`status == 3`）**不**拦截私信。

## Capabilities

### New Capabilities
- `contacts`: 联系人可见性、屏蔽与删除管理--定义联系人在列表中的可见性规则、屏蔽/取消屏蔽/删除行为，以及私信屏蔽拦截。

### Modified Capabilities
<!-- openspec/specs/ 下此前无正式能力定义，故为新增能力而非修改 -->
- 无

## Impact

- **代码**：
  - `src/api/user.rs` - `get_contacts`（跳过 `status==3` 而非 `status==2`，`status==2` 映射为 `"blocked"`）、`update_contact_status`（`remove` 改为 upsert `status=3`）、`ContactInfo`（`status` 取值）、`UpdateContactStatusRequest` 注释
  - `src/api/message.rs` - 私信屏蔽拦截（行为不变，需回归验证）
  - `src/api/matrix/sync.rs` - bot 联系人 DM 房间生成（依赖 `user.contacts.keys()`，不调整；bot 不会删除联系人，不产生 `status=3`）
- **API**：`GET /contacts` 返回集合扩大（含全部未删除用户，含屏蔽用户）；`POST /update_contact_status` 的 `remove` 行为变更（upsert `status=3`）。属面向私有客户端的内部 API，影响可控。
- **数据**：无 schema 变更（沿用 `contacts` 表与 `status` 字段；新增取值 `3` 表示删除，"默认可见"由"无记录"表达）。
- **客户端**：联系人列表会展示全部未删除用户（含屏蔽者）；需重新挂回"删除联系人"入口（调用 `remove`），屏蔽/取消屏蔽入口与列表中屏蔽态展示需相应 UI 调整。
