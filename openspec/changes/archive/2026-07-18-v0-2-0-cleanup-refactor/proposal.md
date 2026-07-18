## Why

v0.2.0 版本在继续演进前需要对遗留代码进行清理与重构。此前引入的 third_party（第三方）登录、OpenID Connect / OIDC 等认证流程已不再使用，但相关代码、配置和数据库迁移脚本仍残留在仓库中，增加了维护成本与误用风险。本次变更将这些无用代码与配置移除，并把证书目录规整到 `config/` 下，使 v0.2.0 的代码结构更清晰、配置更聚焦。

本变更归档自 `docs/spec.md`，原始记录为：

> # 1、v0.2.0 代码清理和重构
> 代码清理与重构。

对应的工作已在历史提交中完成（见 tasks.md 的提交引用）。

## What Changes

- **移除 third_party 认证代码**：删除第三方登录与 token 交换流程，涉及 `src/api/token.rs`、`admin_login.rs`、`admin_system.rs`、`resource.rs`、`config.rs`、`create_user.rs`、`server.rs`、`state.rs`、`test_harness.rs`。
- **移除 openid / oidc 认证代码**：删除 OpenID Connect 相关流程及数据库迁移字段，后续如需认证再重新实现。
- **清理无用配置**：移除未使用的 `webclient_url`，启用 `domain` 配置项。
- **规整证书目录**：将 `cert/` 迁移到 `config/cert/`，并更新 TLS 配置中的证书路径。

## Capabilities

### New Capabilities
<!-- 本次为清理与重构，不引入新能力 -->
- 无

### Modified Capabilities
<!-- 此前未在 openspec/specs/ 下正式记录任何能力，故无既有 spec 的行为变更 -->
- 无

## Impact

- **代码**：`src/api/token.rs`、`src/api/admin_login.rs`、`src/api/admin_system.rs`、`src/api/resource.rs`、`src/config.rs`、`src/create_user.rs`、`src/server.rs`、`src/state.rs`、`src/test_harness.rs`
- **数据库迁移**：`migrations/001_initial.up.sql`（移除 oidc 相关字段）
- **配置**：`config/config.toml`（清理无用项、更新证书路径）
- **文件布局**：`cert/` → `config/cert/`
- **注意**：openid/oidc 与 third_party 认证能力被移除，后续若需要须重新实现，非平滑回退。
