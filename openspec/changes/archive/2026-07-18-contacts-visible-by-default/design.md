## Context

当前联系人可见性是"显式添加才可见"的模型：

- `contacts` 表 `(uid, target_uid, status)`：`status=1`（added）、`status=2`（blocked）。无记录 = 不出现在列表。
- `GET /contacts`（`src/api/user.rs::get_contacts`）只遍历 `current_user.contacts`（HashMap），即仅返回有记录的用户；`status` 映射为 `"added"` / `"blocked"`。
- `POST /update_contact_status`：`add`->1、`block`->2、`remove`->删除行、`unblock`->`status` 2->1。
- 私信拦截（`src/api/message.rs`）：目标用户把发送者 `status==2` 时返回 403。
- 另有 `GET /`（`get_all_users`）已返回全部用户（`UserInfo`），但不含联系人关系信息。

私有应用用户规模小、彼此可信，期望"默认全可见"。此外，屏蔽与删除此前被合并：屏蔽既隐藏又拦截，`remove` 在可见性翻转后退化。本次把二者拆开：屏蔽只管发消息，删除只管列表可见性。

## Goals / Non-Goals

**Goals:**
- `GET /contacts` 默认返回所有未删除用户（排除自己）
- 屏蔽（`status=2`）仍可见、仅拦截私信
- 删除（`status=3`，新）是唯一令用户从列表不可见的手段；`remove` 动作 upsert `status=3`
- `unblock` 回到"默认可见"，不遗留 `added` 状态
- 私信屏蔽拦截行为保持不变并回归验证；删除不拦截私信

**Non-Goals:**
- 不引入分页（私有应用用户量小，暂不需要）
- 不变更 `contacts` 表 schema（沿用现有 `status` 字段，新增取值 `3`，"默认可见"由"无记录"表达）
- 不调整 `contact_verification_enable` 系统配置语义
- 不重构 bot 在 `sync.rs` 的 DM 房间生成逻辑（见 Decisions）
- 不提供"已删除联系人"恢复入口的独立端点（恢复走 `add` 覆盖，见 Risks）

## Decisions

- **状态语义**：`status=1` added（可见）、`status=2` blocked（可见、不可发消息）、`status=3` deleted（不可见）、无记录 default（可见）。屏蔽与删除正交于两个维度（可见性 / 发消息），但共享单一 `status` 列，故互斥：对同一 target，后发的动作覆盖前者（如对已屏蔽者删除 -> `status=3`，此时不再拦截私信，符合"删除不管发消息"）。
- **`get_contacts` 改为遍历 `cache.users`**：对每个非自身、非删除（`status==3`）用户构造 `ContactResponse`。状态映射：`status==1` -> `"added"`；`status==2` -> `"blocked"`；`status==3` -> 跳过（不可见）；无记录 -> `"default"`。
- **默认用户的时间戳来源**：`ContactInfo.created_at/updated_at` 为非空 `DateTime`。默认用户无联系人关系时间戳，采用目标用户自身的 `created_at/updated_at` 填充，保持结构非空、排序可用，且不引入 API schema 变更。
- **排序**：沿用按 `updated_at` 倒序；默认用户用目标用户 `updated_at` 参与排序，避免无序。
- **`unblock` 删除记录**：将 `unblock` 的 SQL 保持为 `delete from contacts where uid=? and target_uid=? and status=2`；缓存逻辑 `user.contacts.remove(&target_uid)`。取消屏蔽后回到"默认可见"，不遗留 `added`。
- **`remove` 改为 upsert `status=3`**：把 `remove` 并入 `add | block` 的 upsert 分支，`remove` 取 `status=3`；缓存同步插入 `CacheContactInfo { status: 3, .. }`。这样"删除联系人"在"默认全可见"模型下真正从列表移除（而非删行回到 default-visible）。`remove` 复用而非新增动作名：原意即"从列表移除"，此前因可见性翻转成无效操作，现在恢复其语义。
- **`add`/`block` 保持不变**：`add`->1（仍可用于备注/置顶等显式关系）、`block`->2（可见、不可发消息）。
- **私信拦截不动**：`message.rs` 中 `status==2` 拦截保留；`status==3` 不拦截（删除仅控可见性）。
- **bot DM 房间生成（`sync.rs`）不调整**：继续基于 `user.contacts.keys()`（显式 add/block/remove）。bot 不会删除联系人，实际不产生 `status=3`，与既有 blocked 仍生成 DM 房间的行为一致；如需 bot 与全部未删除用户通信，另立变更。

## Risks / Trade-offs

- [屏蔽用户仍在列表中可见] -> 符合"屏蔽只管发消息"的设计；屏蔽态可在列表项上展示（`status="blocked"`），并保留取消屏蔽入口。
- [删除后用户从列表消失，无法在列表中找到以恢复] -> 恢复走搜索用户后 `add`（upsert 覆盖 `status=3` -> `1`）；如需独立"已删除列表"端点，后续另立变更。
- [`GET /contacts` 返回集合扩大，性能随用户数线性增长] -> 私有应用用户量小，可接受；若后续规模增长再加排序/分页。
- [`unblock` 行为变更属 BREAKING] -> 客户端如依赖"取消屏蔽后变为 added"需适配；私有客户端影响可控。
- [默认用户用目标用户时间戳排序，可能不反映"最近联系"] -> 可接受；如需更精细排序，后续按最近消息时间排序。

## Migration Plan

无数据迁移（schema 不变，仅新增 `status` 取值 `3`）。部署步骤：
1. 修改 `get_contacts`（跳过 `status==3`、`status==2` 映射为 `"blocked"`）
2. 修改 `update_contact_status` 的 `remove` 分支为 upsert `status=3`，缓存同步插入
3. 更新 `ContactInfo` 状态取值与 `UpdateContactStatusRequest` 注释
4. 补充/更新单元测试（默认可见、屏蔽仍可见、删除不可见、删除不拦截私信、取消屏蔽回到 default、私信屏蔽拦截）
5. 回归 `GET /contacts`、`/update_contact_status` 四动作、私信发送链路

回滚：还原上述代码改动即可，数据无破坏性变更（遗留的 `status=3` 行在回滚后会因 `get_contacts` 旧逻辑被当作"有记录但非 2"处理，需注意；可在回滚前清理 `status=3` 行）。

## Open Questions

- 是否需要"已删除联系人"恢复端点（`GET /contacts/deleted`）？当前倾向否，恢复走搜索 + `add`；待确认。
- bot 是否需要随"默认可见"调整为与全部未删除用户生成 DM 房间？当前倾向否，维持现状。
