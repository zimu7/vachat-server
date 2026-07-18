## REMOVED Requirements

### Requirement: OpenID Connect / OIDC authentication

The system SHALL NOT support OpenID Connect (OIDC) authentication flows.
All OIDC code paths, database migration fields, and supporting state have
been removed from `src/api/token.rs`, `admin_login.rs`, `create_user.rs`,
`state.rs`, `server.rs`, `test_harness.rs`, and
`migrations/001_initial.up.sql`.

**Reason**: The OIDC authentication flow was not used in v0.2.0 and was
removed to reduce maintenance surface. Removed in commit `06f0966`
("去掉openid及oidc相关代码，后续需要再添加。"). It may be re-added as a
separate change if a concrete need arises.

**Migration**: No replacement is provided. If OIDC support is required again
it must be re-implemented, including the migration fields that were removed
from `migrations/001_initial.up.sql`. This is a non-reversible removal.
