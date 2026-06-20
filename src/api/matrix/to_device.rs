//! Matrix to-device endpoint - handles m.room.encrypted to-device events
//!
//! This endpoint receives to-device messages, primarily used for distributing
//! Megolm session keys via Olm-encrypted m.room_key events.

use hmac::{Hmac, Mac, NewMac};
use poem::{
    handler,
    http::StatusCode,
    web::{Data, Json},
    Body, Request, Result,
};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::state::State;

/// Get pickle key using HMAC-SHA256
fn get_pickle_key(server_key: &str) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"vachat-olm-pickle-salt").unwrap();
    mac.update(server_key.as_bytes());
    let result = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result[..32]);
    key
}

/// Matrix sendToDevice endpoint
/// Receives to-device messages like m.room.encrypted containing m.room_key
#[handler]
pub async fn send_to_device(
    state: Data<&State>,
    body: Body,
    req: &Request,
) -> Result<Json<Value>> {
    // Validate access token
    let uid = super::auth::validate_access_token(&state, req).await?;

    // Read request body
    let body_bytes = body.into_bytes().await.map_err(|e| {
        poem::error::Error::from_string(
            format!("Failed to read body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    let messages: Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        poem::error::Error::from_string(
            format!("Invalid JSON body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    // Format: { "messages": { "@user:domain": { "device_id": { ...encrypted content... } } } }
    let messages_map = messages
        .get("messages")
        .and_then(|v| v.as_object());

    if messages_map.is_none() {
        tracing::warn!("sendToDevice: no messages field in body");
        return Ok(Json(json!({})));
    }

    let messages_map = messages_map.unwrap();

    for (user_id, device_map) in messages_map {
        if let Some(devices) = device_map.as_object() {
            for (device_id, encrypted_content) in devices {
                tracing::info!(
                    "sendToDevice: uid={}, user_id={}, device_id={}",
                    uid,
                    user_id,
                    device_id
                );

                // Process m.room.encrypted to-device events
                // These contain Olm-encrypted m.room_key events
                if let Some(content) = encrypted_content.as_object() {
                    let algorithm = content
                        .get("algorithm")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if algorithm == "m.olm.v1.curve25519-aes-sha2" {
                        // This is an Olm-encrypted to-device message
                        // Try to decrypt it and extract the m.room_key
                        if let Err(e) = process_olm_to_device_message(&state, uid, content).await {
                            tracing::warn!(
                                "Failed to process Olm to-device message for uid={}: {}",
                                uid,
                                e
                            );
                        }
                    } else {
                        tracing::debug!(
                            "sendToDevice: unknown algorithm {} for uid={}",
                            algorithm,
                            uid
                        );
                    }
                }
            }
        }
    }

    Ok(Json(json!({})))
}

/// Process an Olm-encrypted to-device message
/// Decrypts the Olm payload and handles the inner event (e.g. m.room_key)
///
/// This uses the server's Olm Account to create an inbound session from the
/// pre-key message, which is the proper Matrix way to establish an Olm session.
async fn process_olm_to_device_message(
    state: &State,
    local_uid: i64,
    content: &serde_json::Map<String, Value>,
) -> Result<()> {
    let sender_key = content
        .get("sender_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let ciphertext_obj = content.get("ciphertext").and_then(|v| v.as_object());

    if ciphertext_obj.is_none() {
        tracing::warn!("Olm to-device message has no ciphertext object");
        return Ok(());
    }

    let ciphertext_obj = ciphertext_obj.unwrap();

    // Get our server device's curve25519 key
    let bot_device_keys = state
        .device_keys_manager
        .get_user_device_keys(local_uid)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get bot device keys: {}", e);
            poem::error::Error::from_string(
                format!("Internal error: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

    // Find the ciphertext for our server device
    let mut our_ciphertext: Option<(u32, String)> = None;

    for device in &bot_device_keys {
        if let Some(ct) = ciphertext_obj.get(&device.curve25519_key) {
            let msg_type = ct.get("type").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let body = ct.get("body").and_then(|v| v.as_str()).unwrap_or("");
            our_ciphertext = Some((msg_type, body.to_string()));
            break;
        }
    }

    let (msg_type, ciphertext_b64) = match our_ciphertext {
        Some(ct) => ct,
        None => {
            tracing::warn!(
                "No ciphertext found for our key in to-device message. Available keys: {:?}",
                ciphertext_obj.keys().collect::<Vec<_>>()
            );
            return Ok(());
        }
    };

    // msg_type 0 = pre-key message (first message in a new Olm session)
    // msg_type 1 = normal message (continuation of an existing session)
    if msg_type != 0 {
        tracing::warn!(
            "Non-pre-key Olm message in to-device (msg_type={}), attempting session-based decrypt",
            msg_type
        );
        // Try decrypting with existing sessions (for session continuation)
        let decrypted = super::rooms::decrypt_olm_message(
            state,
            local_uid,
            sender_key,
            msg_type,
            &ciphertext_b64,
        )
        .await?;

        handle_decrypted_to_device(state, local_uid, sender_key, &decrypted).await?;
        return Ok(());
    }

    // For pre-key messages, use the server Olm Account to create a new inbound session
    let server_key = {
        let config = state.key_config.read().await;
        config.server_key.clone()
    };

    // Load the server Olm Account
    let mut account = state
        .server_olm_account_manager
        .load_account(local_uid, &server_key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to load server Olm account for uid={}: {}", local_uid, e);
            poem::error::Error::from_string(
                format!("Internal error: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

    // Parse the sender's curve25519 identity key
    let sender_identity_key = vodozemac::Curve25519PublicKey::from_base64(sender_key).map_err(|e| {
        tracing::error!("Invalid sender curve25519 key: {}", e);
        poem::error::Error::from_string(
            format!("Invalid sender key: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    // Parse the pre-key message
    let pre_key_msg = vodozemac::olm::PreKeyMessage::from_base64(&ciphertext_b64).map_err(|e| {
        tracing::error!("Invalid PreKey message base64: {}", e);
        poem::error::Error::from_string(
            format!("Invalid PreKey message: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    // Create inbound session from the pre-key message
    let inbound_result = account
        .create_inbound_session(sender_identity_key, &pre_key_msg)
        .map_err(|e| {
            tracing::error!("Failed to create inbound Olm session: {}", e);
            poem::error::Error::from_string(
                format!("Failed to create inbound session: {}", e),
                StatusCode::BAD_REQUEST,
            )
        })?;

    // Get the plaintext
    let plaintext = String::from_utf8_lossy(&inbound_result.plaintext).to_string();
    tracing::info!("Decrypted to-device message (new session): {}", plaintext);

    // Save the updated Account (OTK was consumed) and the new inbound session
    state
        .server_olm_account_manager
        .save_account(local_uid, &account, &server_key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to save server Olm account: {}", e);
            poem::error::Error::from_string(
                format!("Internal error: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

    // Store the inbound session for future use (session continuation)
    state
        .olm_session_manager
        .store_inbound_session(
            local_uid,
            crate::api::matrix::e2ee::SERVER_DEVICE_ID,
            0, // sender_uid - unknown at this point
            "", // sender_device_id - unknown
            sender_key,
            &inbound_result.session.session_id(),
            &serde_json::to_vec(&inbound_result.session.pickle()).unwrap_or_default(),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to store inbound Olm session: {}", e);
            poem::error::Error::from_string(
                format!("Internal error: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

    // Handle the decrypted content
    handle_decrypted_to_device(state, local_uid, sender_key, &plaintext).await?;

    Ok(())
}

/// Handle a decrypted to-device message content
async fn handle_decrypted_to_device(
    state: &State,
    local_uid: i64,
    sender_key: &str,
    plaintext: &str,
) -> Result<()> {
    let decrypted_json: Value = serde_json::from_str(plaintext).unwrap_or_else(|e| {
        tracing::warn!("Failed to parse decrypted to-device JSON: {}", e);
        json!({})
    });

    let event_type = decrypted_json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if event_type == "m.room_key" {
        handle_room_key_event(state, local_uid, sender_key, &decrypted_json).await?;
    } else {
        tracing::debug!(
            "Ignoring to-device event of type: {}",
            event_type
        );
    }

    Ok(())
}

/// Handle a m.room_key event - store the inbound Megolm session
async fn handle_room_key_event(
    state: &State,
    _local_uid: i64,
    sender_key: &str,
    room_key_json: &Value,
) -> Result<()> {
    let content = room_key_json.get("content").unwrap_or(room_key_json);

    let algorithm = content
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if algorithm != "m.megolm.v1.aes-sha2" {
        tracing::debug!("Ignoring room key with algorithm: {}", algorithm);
        return Ok(());
    }

    let session_id = content
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let session_key_b64 = content
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let room_id = content
        .get("room_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if session_id.is_empty() || session_key_b64.is_empty() {
        tracing::warn!("m.room_key missing session_id or session_key");
        return Ok(());
    }

    tracing::info!(
        "Received m.room_key: room_id={}, session_id={}, algorithm={}",
        room_id,
        session_id,
        algorithm
    );

    // Find sender_uid from sender_key
    let sender_uid = super::rooms::get_uid_from_curve25519_key(state, sender_key)
        .await
        .unwrap_or(0);

    // Create InboundGroupSession from the session key
    let session_key = vodozemac::megolm::SessionKey::from_base64(session_key_b64).map_err(|e| {
        tracing::error!("Failed to parse Megolm session key: {}", e);
        poem::error::Error::from_string(
            format!("Invalid session key: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    let inbound_session = vodozemac::megolm::InboundGroupSession::new(
        &session_key,
        vodozemac::megolm::SessionConfig::version_1(),
    );

    // Pickle and encrypt the session for storage
    let pickle_key = {
        let config = state.key_config.read().await;
        get_pickle_key(&config.server_key)
    };

    let pickle = inbound_session.pickle().encrypt(&pickle_key);

    // Store the inbound session
    state
        .megolm_session_manager
        .store_inbound_session(
            session_id,
            room_id,
            sender_uid,
            "",
            sender_key,
            pickle.as_bytes(),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to store Megolm inbound session: {}", e);
            poem::error::Error::from_string(
                format!("Failed to store session: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

    tracing::info!(
        "Stored Megolm inbound session: session_id={}, room_id={}, sender_uid={}",
        session_id,
        room_id,
        sender_uid
    );

    Ok(())
}
