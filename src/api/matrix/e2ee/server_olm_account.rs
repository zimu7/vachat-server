//! Server-side Olm Account management for bot users
//!
//! Each bot user needs a virtual "server device" with its own Olm identity keys.
//! This allows external Matrix clients (like bridges) to:
//! 1. Discover the server device via keys/query
//! 2. Claim one-time keys via keys/claim
//! 3. Establish Olm sessions to send m.room_key events via sendToDevice
//!
//! The server Olm Account pickle is stored encrypted in the database,
//! using a key derived from the server's key_config.server_key via HMAC-SHA256.

use hmac::{Hmac, Mac, NewMac};
use sha2::Sha256;
use sqlx::SqlitePool;
use vodozemac::olm::Account;

/// Server device ID used for all bot virtual devices
pub const SERVER_DEVICE_ID: &str = "SERVERDEVICE";

/// Manager for server-side Olm Accounts
pub struct ServerOlmAccountManager {
    pool: SqlitePool,
}

impl ServerOlmAccountManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Get the encrypted pickle key derived from server_key using HMAC-SHA256
    fn get_pickle_key(server_key: &str) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"vachat-olm-pickle-salt").unwrap();
        mac.update(server_key.as_bytes());
        let result = mac.finalize().into_bytes();
        let mut key = [0u8; 32];
        key.copy_from_slice(&result[..32]);
        key
    }

    /// Initialize a server Olm Account for a bot user if it doesn't exist yet.
    /// This creates the Account, generates OTKs, and stores both the identity keys
    /// (as device_keys) and the signed OTKs (as device_otk) in the database.
    /// Returns the Account if it was newly created, or loads the existing one.
    pub async fn ensure_account(
        &self,
        uid: i64,
        user_id: &str,
        _matrix_domain: &str,
        server_key: &str,
    ) -> sqlx::Result<Account> {
        // Check if account already exists for SERVER_DEVICE_ID
        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM matrix_olm_account WHERE uid = ? AND device_id = ?",
        )
        .bind(uid)
        .bind(SERVER_DEVICE_ID)
        .fetch_one(&self.pool)
        .await?;

        if existing > 0 {
            return self.load_account(uid, server_key).await;
        }

        // Create new Account
        let mut account = Account::new();
        let identity_keys = account.identity_keys();

        // Generate 50 one-time keys
        let otk_result = account.generate_one_time_keys(50);
        account.mark_keys_as_published();

        // Pickle and encrypt the account for storage
        let pickle_key = Self::get_pickle_key(server_key);
        let pickle = account.pickle().encrypt(&pickle_key);

        // Store the encrypted account pickle
        sqlx::query(
            r#"
            INSERT INTO matrix_olm_account (uid, device_id, account_data, created_at, updated_at)
            VALUES (?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(uid)
        .bind(SERVER_DEVICE_ID)
        .bind(pickle.as_bytes())
        .execute(&self.pool)
        .await?;

        // Build and store device_keys for this server device
        let curve25519_key = identity_keys.curve25519.to_base64();
        let ed25519_key = identity_keys.ed25519.to_base64();

        // Build properly signed device_keys for this server device
        let signed_device_keys = build_signed_device_keys(
            &account,
            user_id,
            SERVER_DEVICE_ID,
            &curve25519_key,
            &ed25519_key,
        );
        let device_keys_str = serde_json::to_string(&signed_device_keys).unwrap_or_default();

        // Store device keys
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
        .bind(SERVER_DEVICE_ID)
        .bind(&curve25519_key)
        .bind(&ed25519_key)
        .bind(&device_keys_str)
        .execute(&self.pool)
        .await?;

        // Store signed one-time keys
        let otk_count = store_signed_otks(
            &self.pool,
            uid,
            SERVER_DEVICE_ID,
            &otk_result,
            &account,
            user_id,
        )
        .await?;

        tracing::info!(
            "Server Olm Account initialized for uid={}: curve25519={}, ed25519={}, otk_count={}",
            uid,
            &curve25519_key[..16],
            &ed25519_key[..16.min(ed25519_key.len())],
            otk_count
        );

        Ok(account)
    }

    /// Load an existing server Olm Account from the database
    pub async fn load_account(&self, uid: i64, server_key: &str) -> sqlx::Result<Account> {
        let account_data: Vec<u8> = sqlx::query_scalar(
            "SELECT account_data FROM matrix_olm_account WHERE uid = ? AND device_id = ?",
        )
        .bind(uid)
        .bind(SERVER_DEVICE_ID)
        .fetch_one(&self.pool)
        .await?;

        let pickle_key = Self::get_pickle_key(server_key);
        let pickle_str = String::from_utf8_lossy(&account_data);
        let decrypted_pickle =
            vodozemac::olm::AccountPickle::from_encrypted(&pickle_str, &pickle_key).map_err(
                |e| {
                    sqlx::Error::Protocol(format!(
                        "Failed to decrypt server Olm account pickle: {}",
                        e
                    ))
                },
            )?;

        Ok(Account::from_pickle(decrypted_pickle))
    }

    /// Create or load an Olm Account for a specific device
    /// This is used when a new device_id is created during login
    pub async fn create_device_account(
        &self,
        uid: i64,
        device_id: &str,
        user_id: &str,
        server_key: &str,
    ) -> sqlx::Result<Account> {
        // Check if account already exists for this device
        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM matrix_olm_account WHERE uid = ? AND device_id = ?",
        )
        .bind(uid)
        .bind(device_id)
        .fetch_one(&self.pool)
        .await?;

        if existing > 0 {
            return self.load_device_account(uid, device_id, server_key).await;
        }

        // Create new Account for this device
        let mut account = Account::new();
        let identity_keys = account.identity_keys();

        // Generate 50 one-time keys
        let otk_result = account.generate_one_time_keys(50);
        account.mark_keys_as_published();

        // Pickle and encrypt
        let pickle_key = Self::get_pickle_key(server_key);
        let pickle = account.pickle().encrypt(&pickle_key);

        // Store the account
        sqlx::query(
            r#"
            INSERT INTO matrix_olm_account (uid, device_id, account_data, created_at, updated_at)
            VALUES (?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(uid)
        .bind(device_id)
        .bind(pickle.as_bytes())
        .execute(&self.pool)
        .await?;

        // Build and store device_keys
        let curve25519_key = identity_keys.curve25519.to_base64();
        let ed25519_key = identity_keys.ed25519.to_base64();

        let signed_device_keys =
            build_signed_device_keys(&account, user_id, device_id, &curve25519_key, &ed25519_key);
        let device_keys_str = serde_json::to_string(&signed_device_keys).unwrap_or_default();

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
        .bind(&curve25519_key)
        .bind(&ed25519_key)
        .bind(&device_keys_str)
        .execute(&self.pool)
        .await?;

        // Store OTKs
        store_signed_otks(&self.pool, uid, device_id, &otk_result, &account, user_id).await?;

        tracing::info!(
            "Olm Account created for uid={}, device_id={}",
            uid,
            device_id
        );

        Ok(account)
    }

    /// Load an Olm Account for a specific device
    pub async fn load_device_account(
        &self,
        uid: i64,
        device_id: &str,
        server_key: &str,
    ) -> sqlx::Result<Account> {
        let account_data: Vec<u8> = sqlx::query_scalar(
            "SELECT account_data FROM matrix_olm_account WHERE uid = ? AND device_id = ?",
        )
        .bind(uid)
        .bind(device_id)
        .fetch_one(&self.pool)
        .await?;

        let pickle_key = Self::get_pickle_key(server_key);
        let pickle_str = String::from_utf8_lossy(&account_data);
        let decrypted_pickle =
            vodozemac::olm::AccountPickle::from_encrypted(&pickle_str, &pickle_key).map_err(
                |e| sqlx::Error::Protocol(format!("Failed to decrypt Olm account: {}", e)),
            )?;

        Ok(Account::from_pickle(decrypted_pickle))
    }

    /// Save the current state of an Account back to the database
    /// (needed after creating inbound sessions which consume OTKs)
    pub async fn save_account(
        &self,
        uid: i64,
        account: &Account,
        server_key: &str,
    ) -> sqlx::Result<()> {
        let pickle_key = Self::get_pickle_key(server_key);
        let pickle = account.pickle().encrypt(&pickle_key);

        sqlx::query(
            r#"
            UPDATE matrix_olm_account
            SET account_data = ?, updated_at = datetime('now')
            WHERE uid = ? AND device_id = ?
            "#,
        )
        .bind(pickle.as_bytes())
        .bind(uid)
        .bind(SERVER_DEVICE_ID)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Save the current state of a device Account back to the database
    /// save_device_account 是 save_account 的通用化版本，目前只需要保存服务端设备（SERVER_DEVICE_ID）的 account，所以只有
    /// save_account 被使用。save_device_account 是为后续多设备场景预留的，加 #[allow(dead_code)] 保留。
    #[allow(dead_code)]
    pub async fn save_device_account(
        &self,
        uid: i64,
        device_id: &str,
        account: &Account,
        server_key: &str,
    ) -> sqlx::Result<()> {
        let pickle_key = Self::get_pickle_key(server_key);
        let pickle = account.pickle().encrypt(&pickle_key);

        sqlx::query(
            r#"
            UPDATE matrix_olm_account
            SET account_data = ?, updated_at = datetime('now')
            WHERE uid = ? AND device_id = ?
            "#,
        )
        .bind(pickle.as_bytes())
        .bind(uid)
        .bind(device_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

/// Build properly signed device_keys structure for the server device
fn build_signed_device_keys(
    account: &Account,
    user_id: &str,
    device_id: &str,
    curve25519_key: &str,
    ed25519_key: &str,
) -> serde_json::Value {
    // Build the object that gets signed (per Matrix spec, signatures are over the
    // canonical JSON of the object WITHOUT the signatures field)
    let keys_obj = serde_json::json!({
        "user_id": user_id,
        "device_id": device_id,
        "algorithms": ["m.olm.v1.curve25519-aes-sha2", "m.megolm.v1.aes-sha2"],
        "keys": {
            format!("curve25519:{}", device_id): curve25519_key,
            format!("ed25519:{}", device_id): ed25519_key,
        }
    });

    // Create canonical JSON for signing (remove signatures if present)
    let canonical_json = canonical_json(&keys_obj);

    // Sign the canonical JSON
    let signature = account.sign(&canonical_json).to_base64();

    // Add signature to the final object
    let mut result = keys_obj.clone();
    result["signatures"] = serde_json::json!({
        user_id: {
            format!("ed25519:{}", device_id): signature
        }
    });

    result
}

/// Produce Matrix canonical JSON (sorted keys, no whitespace, no null values)
fn canonical_json(value: &serde_json::Value) -> String {
    // Simple canonical JSON: sort keys, compact format, skip null
    let mut buf = String::new();
    canonicalize_value(value, &mut buf);
    buf
}

fn canonicalize_value(value: &serde_json::Value, buf: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            buf.push('{');
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(k, _)| *k);
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push('"');
                buf.push_str(k);
                buf.push_str("\":");
                canonicalize_value(v, buf);
            }
            buf.push('}');
        }
        serde_json::Value::Array(arr) => {
            buf.push('[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                canonicalize_value(v, buf);
            }
            buf.push(']');
        }
        serde_json::Value::String(s) => {
            buf.push('"');
            // Escape special characters
            for c in s.chars() {
                match c {
                    '\\' => buf.push_str("\\\\"),
                    '"' => buf.push_str("\\\""),
                    '\n' => buf.push_str("\\n"),
                    '\r' => buf.push_str("\\r"),
                    '\t' => buf.push_str("\\t"),
                    c => buf.push(c),
                }
            }
            buf.push('"');
        }
        serde_json::Value::Number(n) => {
            buf.push_str(&n.to_string());
        }
        serde_json::Value::Bool(b) => {
            buf.push_str(if *b { "true" } else { "false" });
        }
        serde_json::Value::Null => {
            // Skip null values in canonical JSON (but this is simplified)
        }
    }
}

/// Store signed one-time keys generated by the server Account
async fn store_signed_otks(
    pool: &SqlitePool,
    uid: i64,
    device_id: &str,
    otk_result: &vodozemac::olm::OneTimeKeyGenerationResult,
    account: &Account,
    user_id: &str,
) -> sqlx::Result<usize> {
    let mut count = 0;

    // The generated keys are in otk_result.created as Curve25519PublicKey values
    // We need to match them with key IDs from account.one_time_keys()
    // and sign each one with the account's ed25519 key
    for public_key in &otk_result.created {
        let curve25519_key = public_key.to_base64();

        // Build the signed OTK JSON as per Matrix spec:
        // {"key": "<curve25519_key>", "signatures": {"@user:domain": {"ed25519:DEVICE": "<sig>"}}}
        // The signature is over the key JSON (just the "key" field)
        let key_json = format!("{{\"key\":\"{}\"}}", curve25519_key);
        let signature = account.sign(&key_json).to_base64();

        let key_id_str = format!("signed_curve25519:{}", curve25519_key);

        let signatures = serde_json::json!({
            user_id: {
                format!("ed25519:{}", device_id): signature
            }
        });
        let signatures_json = serde_json::to_string(&signatures).unwrap_or_default();

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
        .bind(&key_id_str)
        .bind(&curve25519_key)
        .bind(&signatures_json)
        .execute(pool)
        .await?;

        count += 1;
    }

    Ok(count)
}
