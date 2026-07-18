## 1. 移除 third_party 认证代码

- [x] 1.1 删除 `src/api/token.rs` 中 third_party token 交换流程（提交 `034f5ff`）
- [x] 1.2 删除 `src/api/admin_login.rs`、`admin_system.rs`、`resource.rs` 中 third_party 相关分支（提交 `034f5ff`）
- [x] 1.3 移除 `src/config.rs`、`create_user.rs`、`server.rs`、`state.rs`、`test_harness.rs` 中 third_party 配置与依赖（提交 `034f5ff`）

## 2. 移除 openid / oidc 认证代码

- [x] 2.1 删除 `src/api/token.rs` 中 oidc 认证流程（提交 `06f0966`）
- [x] 2.2 删除 `src/api/admin_login.rs`、`create_user.rs` 中 oidc 相关逻辑（提交 `06f0966`）
- [x] 2.3 移除 `migrations/001_initial.up.sql` 中 oidc 相关字段（提交 `06f0966`）
- [x] 2.4 清理 `src/state.rs`、`server.rs`、`test_harness.rs` 中 oidc 状态与依赖（提交 `06f0966`）

## 3. 清理无用配置

- [x] 3.1 移除 `config/config.toml` 中未使用的 `webclient_url`（提交 `80268b2`）
- [x] 3.2 启用 `domain` 配置项，更新 TLS 证书路径为 `./config/cert/`（提交 `80268b2`）

## 4. 规整证书目录

- [x] 4.1 将 `cert/` 目录迁移至 `config/cert/`（提交 `6c6d5b9`）
- [x] 4.2 同步 `config/config.toml` 中 TLS 配置的证书路径（提交 `80268b2`）

## 5. 归档整理

- [x] 5.1 将 `docs/spec.md` 的记录整理为 openspec 变更（proposal / design / tasks）
- [x] 5.2 归档至 `openspec/changes/archive/` 作为已完成的历史变更
