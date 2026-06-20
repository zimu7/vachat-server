//! Megolm inbound group session management for room-level encryption
//!
//! Megolm is a group ratchet used for encrypting messages in Matrix rooms.
//! Unlike Olm which is 1:1, Megolm uses a single session for all recipients
//! in a room. The session key is distributed via to-device m.room_key events.
//!
//! https://matrix.org/docs/guides/e2ee#how-does-megolm-protect-against-compromise

use sqlx::{FromRow, SqlitePool};
use serde::{Deserialize, Serialize};

/// Megolm inbound session record stored in database
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct MegolmInboundSession {
    pub session_id: String,
    pub room_id: String,
    pub sender_uid: i64,
    pub sender_device_id: String,
    pub sender_curve25519_key: String,
    pub session_data: Vec<u8>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: chrono::DateTime<chrono::Utc>,
}

/// Megolm session manager for storing and retrieving inbound group sessions
pub struct MegolmSessionManager {
    pool: SqlitePool,
}

impl MegolmSessionManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Store an inbound Megolm group session
    pub async fn store_inbound_session(
        &self,
        session_id: &str,
        room_id: &str,
        sender_uid: i64,
        sender_device_id: &str,
        sender_curve25519_key: &str,
        session_data: &[u8],
    ) -> sqlx::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO matrix_megolm_inbound_session
                (session_id, room_id, sender_uid, sender_device_id, sender_curve25519_key,
                 session_data, created_at, last_used_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            ON CONFLICT(session_id) DO UPDATE SET
                session_data = excluded.session_data,
                last_used_at = datetime('now')
            "#,
        )
        .bind(session_id)
        .bind(room_id)
        .bind(sender_uid)
        .bind(sender_device_id)
        .bind(sender_curve25519_key)
        .bind(session_data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get inbound Megolm session by session_id
    pub async fn get_inbound_session(
        &self,
        session_id: &str,
    ) -> sqlx::Result<Option<MegolmInboundSession>> {
        sqlx::query_as::<_, MegolmInboundSession>(
            r#"
            SELECT session_id, room_id, sender_uid, sender_device_id, sender_curve25519_key,
                   session_data, created_at, last_used_at
            FROM matrix_megolm_inbound_session
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
            UPDATE matrix_megolm_inbound_session
            SET last_used_at = datetime('now')
            WHERE session_id = ?
            "#,
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all inbound sessions for a room
    #[allow(dead_code)]
    pub async fn get_room_sessions(
        &self,
        room_id: &str,
    ) -> sqlx::Result<Vec<MegolmInboundSession>> {
        sqlx::query_as::<_, MegolmInboundSession>(
            r#"
            SELECT session_id, room_id, sender_uid, sender_device_id, sender_curve25519_key,
                   session_data, created_at, last_used_at
            FROM matrix_megolm_inbound_session
            WHERE room_id = ?
            ORDER BY last_used_at DESC
            "#,
        )
        .bind(room_id)
        .fetch_all(&self.pool)
        .await
    }
}