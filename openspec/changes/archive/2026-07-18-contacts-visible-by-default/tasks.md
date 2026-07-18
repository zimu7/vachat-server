## 1. 重写联系人列表可见性

- [x] 1.1 修改 `src/api/user.rs::get_contacts`：遍历 `cache.users`，排除调用者自身与 `status==3`（删除）用户，构造 `ContactResponse`
- [x] 1.2 状态映射：`status==1` -> `"added"`、`status==2` -> `"blocked"`（可见）、无记录 -> `"default"`；`status==3` 不进入列表
- [x] 1.3 默认用户 `ContactInfo.created_at/updated_at` 用目标用户自身时间戳填充；保持按 `updated_at` 倒序排序

## 2. 拆分屏蔽与删除语义

- [x] 2.1 `update_contact_status` 的 `remove` 分支由"删除行"改为 upsert `status=3`（并入 `add | block` 的 upsert 分支）
- [x] 2.2 对应缓存逻辑改为插入 `CacheContactInfo { status: 3, .. }`
- [x] 2.3 `unblock` 保持删行（回到默认可见），`add`/`block` 保持不变（`block` 现为"可见、不可发消息"）
- [x] 2.4 更新 `UpdateContactStatusRequest.action` 注释

## 3. 回归与验证

- [x] 3.1 验证 `src/api/message.rs` 私信屏蔽拦截（`status==2` -> 403）行为不变；`status==3` 不拦截
- [x] 3.2 确认 `src/api/matrix/sync.rs` bot DM 房间生成仍基于 `user.contacts.keys()`（本次不改动，记录决策）
- [x] 3.3 补充/更新单元测试：默认可见、屏蔽仍可见且 `status="blocked"`、删除不可见、删除不拦截私信、取消屏蔽回到 `default`、私信屏蔽拦截

## 4. 文档与收尾

- [x] 4.1 更新接口说明（`GET /contacts` 返回集合与 `status` 取值、`remove` 语义）
- [ ] 4.2 本地运行 `cargo build` 与 `cargo test` 通过
