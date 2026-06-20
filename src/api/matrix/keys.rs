//! Matrix keys module - handles E2EE keys query/upload/claim endpoints

use poem::{
    error::InternalServerError,
    handler,
    http::StatusCode,
    web::{Data, Json},
    Body, Request, Result,
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::api::matrix::e2ee::{
    DeviceKeys as E2EEDeviceKeys, KeysUploadRequest as E2EEKeysUploadRequest, OneTimeKey,
};
use serde_json::{json, Value};

use crate::state::State;

/// Matrix keys query request
#[derive(Debug, Deserialize)]
pub struct KeysQueryRequest {
    /// Map of user ID to list of device IDs
    #[serde(default)]
    pub device_keys: HashMap<String, Vec<String>>,

    /// Optional timeout (currently unused)
    #[serde(default)]
    #[allow(dead_code)]
    pub timeout: Option<i64>,

    /// Optional token (currently unused)
    #[serde(default)]
    #[allow(dead_code)]
    pub token: Option<String>,
}

/// Matrix keys claim request
#[derive(Debug, Deserialize)]
pub struct KeysClaimRequest {
    /// Map of user ID to device ID to algorithm
    #[serde(default)]
    pub one_time_keys: HashMap<String, HashMap<String, String>>,

    /// Optional timeout (currently unused)
    #[serde(default)]
    #[allow(dead_code)]
    pub timeout: Option<i64>,
}

/// Matrix keys claim endpoint
/// Claims and marks one-time keys as used
#[handler]
pub async fn keys_claim(
    state: Data<&State>,
    Json(req): Json<KeysClaimRequest>,
    req_http: &Request,
) -> Result<Json<Value>> {
    // Validate access token
    let _uid = super::auth::validate_access_token(&state, req_http).await?;

    let mut one_time_keys: HashMap<String, HashMap<String, HashMap<String, Value>>> =
        HashMap::new();
    let mut failures: HashMap<String, Value> = HashMap::new();

    for (user_id, device_map) in &req.one_time_keys {
        let mut user_otks: HashMap<String, HashMap<String, Value>> = HashMap::new();

        // Resolve user_id to uid
        // user_id format: @username:domain
        let username = user_id.strip_prefix('@').unwrap_or(user_id);
        let username = username.split(':').next().unwrap_or(username);

        // Find the target user's uid
        let target_uid = {
            let cache = state.cache.read().await;
            cache
                .users
                .iter()
                .find(|(_, u)| u.name == username)
                .map(|(uid, _)| *uid)
        };

        if let Some(target_uid) = target_uid {
            for (device_id, algorithm) in device_map {
                tracing::info!(
                    "keys_claim: user_id={}, device_id={}, algorithm={}",
                    user_id,
                    device_id,
                    algorithm
                );

                // Get one unused OTK for this device
                if let Ok(entries) = state
                    .device_keys_manager
                    .get_unused_one_time_keys(target_uid, device_id, 1)
                    .await
                {
                    tracing::info!(
                        "keys_claim: found {} unused OTKs for user {} device {}",
                        entries.len(),
                        user_id,
                        device_id
                    );

                    if let Some(entry) = entries.first() {
                        // Build the OTK response per Matrix spec:
                        // device_id -> { "signed_curve25519:key_id": { "key": "...", "signatures": {...} } }
                        let mut key_data = HashMap::new();
                        key_data.insert("key".to_string(), json!(entry.curve25519_key));

                        // Parse and add signatures if present
                        if let Some(ref sig_json) = entry.signature {
                            if let Ok(sig_value) = serde_json::from_str::<Value>(sig_json) {
                                key_data.insert("signatures".to_string(), sig_value);
                            }
                        }

                        // Only respond if algorithm matches signed_curve25519
                        if algorithm == "signed_curve25519" {
                            // The key_id in the database already includes the algorithm prefix
                            // e.g. "signed_curve25519:AAAA/A"
                            let mut device_otks: HashMap<String, Value> = HashMap::new();
                            device_otks.insert(entry.key_id.clone(), json!(key_data));
                            user_otks.insert(device_id.clone(), device_otks);

                            // Mark the key as used
                            if let Err(e) = state
                                .device_keys_manager
                                .mark_one_time_keys_used(
                                    target_uid,
                                    device_id,
                                    &[entry.key_id.clone()],
                                )
                                .await
                            {
                                tracing::error!("Failed to mark OTK as used: {}", e);
                            }
                        }
                    } else {
                        // No OTK available for this device
                        tracing::warn!(
                            "No OTK available for user {} device {}",
                            user_id,
                            device_id
                        );
                    }
                }
            }
        } else {
            // User not found
            tracing::warn!("User {} not found for keys claim", user_id);
            failures.insert(
                user_id.clone(),
                json!({
                    "status": "NOT_FOUND",
                    "reason": "User not found"
                }),
            );
        }

        one_time_keys.insert(user_id.clone(), user_otks);
    }

    Ok(Json(json!({
        "one_time_keys": one_time_keys,
        "failures": failures
    })))
}

/// Matrix keys query endpoint
#[handler]
pub async fn keys_query(
    state: Data<&State>,
    Json(req): Json<KeysQueryRequest>,
    req_http: &Request,
) -> Result<Json<Value>> {
    // Validate access token
    let _uid = super::auth::validate_access_token(&state, req_http).await?;

    let mut device_keys_response: HashMap<String, HashMap<String, E2EEDeviceKeys>> = HashMap::new();
    let cache = state.cache.read().await;

    // Process each user in the request
    for (user_id, device_ids) in &req.device_keys {
        let mut user_devices: HashMap<String, E2EEDeviceKeys> = HashMap::new();

        // Try to resolve user_id to uid
        // user_id format: @username:domain
        let username = user_id.strip_prefix('@').unwrap_or(user_id);
        let username = username.split(':').next().unwrap_or(username);
        tracing::info!(
            "keys_query: user_id={}, parsed username={}, requested device_ids={:?}",
            user_id,
            username,
            device_ids
        );

        // Find user by name in cache
        if let Some((uid, _user)) = cache.users.iter().find(|(_, u)| u.name == username) {
            tracing::info!("keys_query: found user uid={}", uid);
            // Get device keys from database
            if let Ok(entries) = state.device_keys_manager.get_user_device_keys(*uid).await {
                tracing::info!("keys_query: found {} device_keys entries", entries.len());
                for entry in entries {
                    tracing::info!(
                        "keys_query: entry device_id={}, requested device_ids={:?}",
                        entry.device_id,
                        device_ids
                    );
                    // Filter by requested device_ids if specified
                    if !device_ids.is_empty() && !device_ids.contains(&entry.device_id) {
                        tracing::info!(
                            "keys_query: skipping device_id={} (not in requested list)",
                            entry.device_id
                        );
                        continue;
                    }

                    // Parse stored keys_json back to DeviceKeys
                    if let Ok(device_keys) =
                        serde_json::from_str::<E2EEDeviceKeys>(&entry.keys_json)
                    {
                        user_devices.insert(entry.device_id, device_keys);
                    }
                }
            } else {
                tracing::warn!(
                    "keys_query: failed to get device_keys from db for uid={}",
                    uid
                );
            }
        } else {
            tracing::warn!(
                "keys_query: user '{}' not found in cache. Available users: {:?}",
                username,
                cache
                    .users
                    .values()
                    .map(|u| u.name.as_str())
                    .collect::<Vec<_>>()
            );
        }

        device_keys_response.insert(user_id.clone(), user_devices);
    }

    let response = json!({
        "device_keys": device_keys_response,
        "failures": {}
    });
    tracing::info!("keys_query response: {:?}", response);

    Ok(Json(response))
}

/// Matrix keys upload endpoint
#[handler]
pub async fn keys_upload(state: Data<&State>, body: Body, req: &Request) -> Result<Json<Value>> {
    // Validate access token and get uid
    let uid = super::auth::validate_access_token(&state, req).await?;

    // Read request body
    let body_bytes = body.into_bytes().await.map_err(|e| {
        poem::error::Error::from_string(
            format!("Failed to read body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    // Log raw request body for debugging
    let body_str = String::from_utf8_lossy(&body_bytes);
    tracing::info!("keys_upload raw body: {}", body_str);

    let upload_req: E2EEKeysUploadRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        poem::error::Error::from_string(
            format!("Invalid JSON body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    // Get token from Authorization header to find matching bot_key
    let auth_header = req.headers().get("Authorization").ok_or_else(|| {
        super::auth::matrix_error(
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            "Missing Authorization header",
        )
    })?;
    let auth_str = auth_header.to_str().map_err(|_| {
        super::auth::matrix_error(
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            "Invalid Authorization header",
        )
    })?;
    let token = auth_str.strip_prefix("Bearer ").ok_or_else(|| {
        super::auth::matrix_error(
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            "Invalid Authorization header format",
        )
    })?;

    // Get device_id from the bot_key associated with the access token
    // This must match what login/whoami returns, otherwise keys_query and keys_claim
    // won't find the stored keys
    let cache = state.cache.read().await;
    let user = cache.users.get(&uid).ok_or_else(|| {
        super::auth::matrix_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "M_UNKNOWN",
            "User not found in cache",
        )
    })?;
    let device_id = user
        .bot_keys
        .values()
        .find(|bot_key| bot_key.key == token)
        .map(|bot_key| bot_key.name.clone())
        .unwrap_or_else(|| "BOTDEVICE".to_string());
    drop(cache);

    tracing::info!(
        "keys_upload: uid={}, device_id={}, has_device_keys={}, has_otk={}",
        uid,
        device_id,
        upload_req.device_keys.is_some(),
        upload_req.one_time_keys.is_some()
    );

    // Store device keys if provided
    if let Some(device_keys) = &upload_req.device_keys {
        // Extract Curve25519 and Ed25519 keys from the keys map
        let curve25519_key_id = format!("curve25519:{}", device_keys.device_id);
        let ed25519_key_id = format!("ed25519:{}", device_keys.device_id);

        let curve25519_key = device_keys
            .keys
            .get(&curve25519_key_id)
            .cloned()
            .unwrap_or_default();
        let ed25519_key = device_keys
            .keys
            .get(&ed25519_key_id)
            .cloned()
            .unwrap_or_default();

        if curve25519_key.is_empty() || ed25519_key.is_empty() {
            tracing::warn!(
                "Missing curve25519 or ed25519 key. device_id={}, available keys: {:?}",
                device_keys.device_id,
                device_keys.keys.keys().collect::<Vec<_>>()
            );
            return Err(super::auth::matrix_error(
                StatusCode::BAD_REQUEST,
                "M_MISSING_PARAM",
                "Missing curve25519 or ed25519 key",
            ));
        }

        state
            .device_keys_manager
            .store_device_keys(uid, &device_id, device_keys, &curve25519_key, &ed25519_key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to store device keys: {}", e);
                InternalServerError(e)
            })?;

        tracing::info!(
            "Device keys stored for user uid={}, device_id={}, curve25519={}, ed25519={}",
            uid,
            device_id,
            &curve25519_key[..16],
            &ed25519_key[..16.min(ed25519_key.len())]
        );
    }

    // Store one-time keys if provided
    // Handle both formats:
    // 1. Nested: {"signed_curve25519": {"key_id": {...}}}
    // 2. Flat: {"signed_curve25519:key_id": {...}}
    if let Some(ref one_time_keys) = &upload_req.one_time_keys {
        // Convert Value to OneTimeKey
        let mut nested_keys: std::collections::HashMap<String, OneTimeKey> =
            std::collections::HashMap::new();

        for (key, value) in one_time_keys {
            match serde_json::from_value::<OneTimeKey>(value.clone()) {
                Ok(otk) => {
                    nested_keys.insert(key.clone(), otk);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse one-time key {}: {}", key, e);
                }
            }
        }

        tracing::info!(
            "keys_upload: processing {} one_time_keys",
            nested_keys.len()
        );

        let one_time_key_count = state
            .device_keys_manager
            .store_one_time_keys(uid, &device_id, &nested_keys)
            .await
            .map_err(|e| {
                tracing::error!("Failed to store one-time keys: {}", e);
                InternalServerError(e)
            })?;

        tracing::info!(
            "One-time keys stored for user uid={}, device_id={}, count={}",
            uid,
            device_id,
            one_time_key_count
        );
    }

    // Return the count of unused one-time keys
    let otk_count = state
        .device_keys_manager
        .get_one_time_key_count(uid, &device_id)
        .await
        .unwrap_or(0);

    let response = json!({
        "one_time_key_counts": {
            "signed_curve25519": otk_count
        }
    });
    tracing::info!(
        "keys_upload response: {}",
        serde_json::to_string(&response).unwrap_or_default()
    );

    Ok(Json(response))
}
