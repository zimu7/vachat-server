## Context

v0.2.0 在引入新能力前积累了若干遗留代码：third_party（第三方）登录、OpenID Connect / OIDC 认证流程，以及散落的配置项与证书目录。这些代码已不再被使用，但仍在编译路径与迁移脚本中，造成认知负担和潜在误用。openspec 本次为首次接入，此前该变更仅以 `docs/spec.md` 一行文字记录，现将其整理为正式归档。

当前状态（归档时）：
- 认证仅保留本地账号 / 管理员登录，third_party 与 oidc 路径已从 `src/api/token.rs` 等文件中删除。
- 证书位于 `config/cert/`，TLS 配置路径已更新。
- `migrations/001_initial.up.sql` 中 oidc 相关字段已移除。

## Goals / Non-Goals

**Goals:**
- 移除不再使用的 third_party 与 openid/oidc 认证代码、迁移与配置
- 将证书目录规整至 `config/` 下，统一配置入口
- 以 openspec 归档形式固化本次清理的事实记录，便于后续追溯

**Non-Goals:**
- 不重新设计认证体系（openid/oidc 后续如需再单独立项）
- 不调整对外 API 契约（仅删除已废弃的内部路径）
- 不修改任何业务逻辑代码

## Decisions

- **直接删除而非保留兼容层**：third_party / oidc 代码已无引用，保留只会增加维护面，故彻底删除。后续如需认证能力重新实现，而非回退。
- **证书迁入 `config/cert/`**：与 `config.toml` 同目录，统一配置资源入口，避免顶层散落 `cert/` 目录。
- **配置项清理**：移除从未使用的 `webclient_url`；`domain` 由注释态改为启用，使配置示例可直接可用。
- **归档而非新建 spec 能力**：本次为纯删除/重构，openspec/specs/ 下此前无正式能力定义，故不新增 spec delta，仅以 proposal + design + tasks 记录事实（archive 时 `--skip-specs`）。

## Risks / Trade-offs

- [openid/oidc 能力不可平滑回退] -> 如后续确需，须重新实现；在 proposal 与 tasks 中明确标注"后续如需再添加"。
- [历史迁移被修改] -> `migrations/001_initial.up.sql` 被直接删减 oidc 字段；该变更已合入历史，属既成事实，归档仅作记录。
- [证书路径变更可能影响部署] -> 部署脚本若仍引用旧 `cert/` 路径需同步更新；已在提交 `6c6d5b9` 中完成目录迁移。

## Migration Plan

已执行（历史提交）：
1. 删除 third_party 认证代码（提交 `034f5ff`）
2. 删除 openid/oidc 代码与迁移字段（提交 `06f0966`）
3. 清理无用配置、更新 TLS 路径（提交 `80268b2`）
4. 证书目录迁至 `config/cert/`（提交 `6c6d5b9`）

回滚策略：无。本变更为既成历史事实的归档，不部署、不回滚。

## Open Questions

- 无（工作已完成，本次仅做归档整理）。
