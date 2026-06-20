//! Olm session management for DM encryption
//!
//! Olm is a double ratchet implementation used for 1:1 encrypted messages in Matrix.
//! https://matrix.org/docs/guides/e2ee#how-does-the-double-ratchet-protect-against-compromise
//!
//! This module provides session storage and retrieval.
//! Actual Olm encryption/decryption uses the vodozemac library.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

/// Olm session manager
pub struct OlmSessionManager {
    pool: SqlitePool,
}

/// Inbound Olm session for decryption
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct InboundOlmSession {
    pub session_id: String,
    pub local_uid: i64,
    pub local_device_id: String,
    pub sender_uid: i64,
    pub sender_device_id: String,
    pub sender_curve25519_key: String,
    pub session_data: Vec<u8>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: chrono::DateTime<chrono::Utc>,
}

impl OlmSessionManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Store an inbound Olm session
    pub async fn store_inbound_session(
        &self,
        local_uid: i64,
        local_device_id: &str,
        sender_uid: i64,
        sender_device_id: &str,
        sender_curve25519_key: &str,
        session_id: &str,
        session_data: &[u8],
    ) -> sqlx::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO matrix_olm_inbound_session
                (session_id, local_uid, local_device_id, sender_uid, sender_device_id,
                 sender_curve25519_key, session_data, created_at, last_used_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            ON CONFLICT(session_id) DO UPDATE SET
                session_data = excluded.session_data,
                last_used_at = datetime('now')
            "#,
        )
        .bind(session_id)
        .bind(local_uid)
        .bind(local_device_id)
        .bind(sender_uid)
        .bind(sender_device_id)
        .bind(sender_curve25519_key)
        .bind(session_data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get inbound sessions for a sender
    pub async fn get_inbound_sessions(
        &self,
        local_uid: i64,
        sender_uid: i64,
    ) -> sqlx::Result<Vec<InboundOlmSession>> {
        sqlx::query_as::<_, InboundOlmSession>(
            r#"
            SELECT session_id, local_uid, local_device_id, sender_uid, sender_device_id,
                   sender_curve25519_key, session_data, created_at, last_used_at
            FROM matrix_olm_inbound_session
            WHERE local_uid = ? AND sender_uid = ?
            ORDER BY last_used_at DESC
            "#,
        )
        .bind(local_uid)
        .bind(sender_uid)
        .fetch_all(&self.pool)
        .await
    }

    /// Get inbound session by session_id
    /// get_inbound_session_by_id 是 OlmSessionManager 上的一个按 session_id 查询入站 Olm session 的方法（返回
    /// Option<InboundOlmSession>），包含完整的 SQL 查询逻辑。当前代码库中没有调用方，原因是 Olm
    /// 消息解密的完整流程尚未接入——解密时需要通过 message 中的 session_id 查出对应的 session_data 才能用 vodozemac
    /// 解密。#[allow(dead_code)] 保留了该方法以便后续接入解密流程。
    #[allow(dead_code)]
    pub async fn get_inbound_session_by_id(
        &self,
        session_id: &str,
    ) -> sqlx::Result<Option<InboundOlmSession>> {
        sqlx::query_as::<_, InboundOlmSession>(
            r#"
            SELECT session_id, local_uid, local_device_id, sender_uid, sender_device_id,
                   sender_curve25519_key, session_data, created_at, last_used_at
            FROM matrix_olm_inbound_session
            WHERE session_id = ?
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// Update last_used_at for a session
    pub async fn update_session_last_used(&self, session_id: &str) -> sqlx::Result<()> {
        sqlx::query(
            r#"
            UPDATE matrix_olm_inbound_session
            SET last_used_at = datetime('now')
            WHERE session_id = ?
            "#,
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
