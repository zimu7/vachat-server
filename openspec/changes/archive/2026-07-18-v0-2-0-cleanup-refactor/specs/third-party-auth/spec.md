## REMOVED Requirements

### Requirement: Third-party login and token exchange

The system SHALL NOT support third-party (third_party) login flows or
third-party token exchange endpoints. All third-party authentication code,
configuration entries, and supporting wiring have been removed from
`src/api/token.rs`, `admin_login.rs`, `admin_system.rs`, `resource.rs`,
`config.rs`, `create_user.rs`, `server.rs`, `state.rs`, and
`test_harness.rs`.

**Reason**: The third-party authentication flow was no longer used in v0.2.0
and retaining it increased maintenance surface and misuse risk. Removed in
commit `034f5ff` ("删掉third_party相关的代码。").

**Migration**: No replacement is provided. If third-party login is needed
again it must be re-implemented from scratch; this is a non-reversible
removal, not a deprecation with a drop-in alternative.
