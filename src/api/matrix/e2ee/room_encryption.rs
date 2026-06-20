//! Room encryption state management

use sqlx::{SqlitePool, FromRow};
use serde::{Deserialize, Serialize};

/// Room encryption state stored in database
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct RoomEncryptionState {
    pub room_id: String,
    pub algorithm: String,
    pub rotation_period_msgs: i64,
    pub rotation_period_ms: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Room encryption manager
pub struct RoomEncryptionManager {
    pool: SqlitePool,
}

impl RoomEncryptionManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Enable encryption for a room
    /// Returns true if encryption was newly enabled, false if already enabled
    pub async fn enable_room_encryption(
        &self,
        room_id: &str,
        algorithm: &str,
        rotation_period_msgs: Option<i64>,
        rotation_period_ms: Option<i64>,
    ) -> sqlx::Result<bool> {
        // Check if already encrypted
        if let Some(existing) = self.get_room_encryption(room_id).await? {
            // Already encrypted, check if algorithm matches
            if existing.algorithm == algorithm {
                return Ok(false);
            }
            // Algorithm mismatch - in Matrix, this should be an error
            // but we'll allow re-negotiation for simplicity
        }

        let rotation_period_msgs = rotation_period_msgs.unwrap_or(100);
        let rotation_period_ms = rotation_period_ms.unwrap_or(604800000); // 1 week

        sqlx::query(
            r#"
            INSERT INTO matrix_room_encryption (room_id, algorithm, rotation_period_msgs, rotation_period_ms, created_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            ON CONFLICT(room_id) DO UPDATE SET
                algorithm = excluded.algorithm,
                rotation_period_msgs = excluded.rotation_period_msgs,
                rotation_period_ms = excluded.rotation_period_ms
            "#,
        )
        .bind(room_id)
        .bind(algorithm)
        .bind(rotation_period_msgs)
        .bind(rotation_period_ms)
        .execute(&self.pool)
        .await?;

        Ok(true)
    }

    /// Get encryption state for a room
    pub async fn get_room_encryption(&self, room_id: &str) -> sqlx::Result<Option<RoomEncryptionState>> {
        sqlx::query_as::<_, RoomEncryptionState>(
            r#"
            SELECT room_id, algorithm, rotation_period_msgs, rotation_period_ms, created_at
            FROM matrix_room_encryption
            WHERE room_id = ?
            "#,
        )
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await
    }
}
