//! Device keys management for E2EE
use super::types::{DeviceKeys, OneTimeKey};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

/// Device keys entry stored in database
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DeviceKeysEntry {
    pub uid: i64,
    pub device_id: String,
    pub curve25519_key: String,
    pub ed25519_key: String,
    pub keys_json: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One-time key entry stored in database
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct OneTimeKeyEntry {
    pub uid: i64,
    pub device_id: String,
    pub key_id: String,
    pub curve25519_key: String,
    pub signature: Option<String>,
    pub used: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Device keys manager for E2EE
pub struct DeviceKeysManager {
    pool: SqlitePool,
}

impl DeviceKeysManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Store or update device keys for a user's device
    pub async fn store_device_keys(
        &self,
        uid: i64,
        device_id: &str,
        device_keys: &DeviceKeys,
        curve25519_key: &str,
        ed25519_key: &str,
    ) -> sqlx::Result<()> {
        let keys_json = serde_json::to_string(device_keys).map_err(|e| {
            sqlx::Error::Protocol(format!("Failed to serialize device keys: {}", e))
        })?;

        sqlx::query(
            r#"
            INSERT INTO matrix_device_keys (uid, device_id, curve25519_key, ed25519_key, keys_json, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            ON CONFLICT(uid, device_id) DO UPDATE SET
                curve25519_key = excluded.curve25519_key,
                ed25519_key = excluded.ed25519_key,
                keys_json = excluded.keys_json,
                updated_at = datetime('now')
            "#,
        )
        .bind(uid)
        .bind(device_id)
        .bind(curve25519_key)
        .bind(ed25519_key)
        .bind(keys_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all device keys for a user
    pub async fn get_user_device_keys(&self, uid: i64) -> sqlx::Result<Vec<DeviceKeysEntry>> {
        sqlx::query_as::<_, DeviceKeysEntry>(
            r#"
            SELECT uid, device_id, curve25519_key, ed25519_key, keys_json, created_at, updated_at
            FROM matrix_device_keys
            WHERE uid = ?
            "#,
        )
        .bind(uid)
        .fetch_all(&self.pool)
        .await
    }

    /// Get all uids that have device_keys stored
    pub async fn get_all_users_with_device_keys(&self) -> sqlx::Result<Vec<i64>> {
        let results: Vec<(i64,)> = sqlx::query_as(
            "SELECT DISTINCT uid FROM matrix_device_keys",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(results.into_iter().map(|(uid,)| uid).collect())
    }

    /// Store one-time keys for a device
    pub async fn store_one_time_keys(
        &self,
        uid: i64,
        device_id: &str,
        keys: &std::collections::HashMap<String, OneTimeKey>,
    ) -> sqlx::Result<usize> {
        let mut count = 0;
        let mut tx = self.pool.begin().await?;

        // First, clean up used OTKs to make room for new ones
        sqlx::query(
            r#"
            DELETE FROM matrix_device_otk WHERE uid = ? AND device_id = ? AND used = TRUE
            "#,
        )
        .bind(uid)
        .bind(device_id)
        .execute(&mut *tx)
        .await?;

        for (key_id_str, otk) in keys {
            // Serialize signatures to JSON string for storage
            let signatures_json = otk
                .signatures
                .as_ref()
                .and_then(|sigs| serde_json::to_string(sigs).ok());

            tracing::info!(
                "Device keys uploaded for user uid={}, device_id={}, key_id={}",
                uid,
                device_id,
                key_id_str
            );

            // Insert or replace (in case of re-upload)
            sqlx::query(
                r#"
                INSERT INTO matrix_device_otk (uid, device_id, key_id, curve25519_key, signature, used, created_at)
                VALUES (?, ?, ?, ?, ?, FALSE, datetime('now'))
                ON CONFLICT(uid, device_id, key_id) DO UPDATE SET
                    curve25519_key = excluded.curve25519_key,
                    signature = excluded.signature,
                    used = FALSE,
                    created_at = datetime('now')
                "#,
            )
            .bind(uid)
            .bind(device_id)
            .bind(key_id_str)
            .bind(&otk.key)
            .bind(&signatures_json)
            .execute(&mut *tx)
            .await?;

            count += 1;
        }

        tx.commit().await?;
        Ok(count)
    }

    /// Get count of unused one-time keys for a device
    pub async fn get_one_time_key_count(&self, uid: i64, device_id: &str) -> sqlx::Result<i64> {
        let result: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM matrix_device_otk WHERE uid = ? AND device_id = ? AND used = FALSE
            "#,
        )
        .bind(uid)
        .bind(device_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.0)
    }

    /// Get unused one-time keys for a device (for keys/claim endpoint)
    pub async fn get_unused_one_time_keys(
        &self,
        uid: i64,
        device_id: &str,
        limit: usize,
    ) -> sqlx::Result<Vec<OneTimeKeyEntry>> {
        sqlx::query_as::<_, OneTimeKeyEntry>(
            r#"
            SELECT uid, device_id, key_id, curve25519_key, signature, used, created_at
            FROM matrix_device_otk
            WHERE uid = ? AND device_id = ? AND used = FALSE
            LIMIT ?
            "#,
        )
        .bind(uid)
        .bind(device_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
    }

    /// Mark one-time keys as used
    pub async fn mark_one_time_keys_used(
        &self,
        uid: i64,
        device_id: &str,
        key_ids: &[String],
    ) -> sqlx::Result<()> {
        if key_ids.is_empty() {
            return Ok(());
        }

        let placeholders = key_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let query = format!(
            "UPDATE matrix_device_otk SET used = TRUE WHERE uid = ? AND device_id = ? AND key_id IN ({})",
            placeholders
        );

        let mut sqlx_query = sqlx::query(&query).bind(uid).bind(device_id);
        for key_id in key_ids {
            sqlx_query = sqlx_query.bind(key_id);
        }
        sqlx_query.execute(&self.pool).await?;

        Ok(())
    }
}
