//! Matrix rooms module - handles room-related endpoints

use hmac::{Hmac, Mac, NewMac};
use poem::{
    error::InternalServerError,
    handler,
    http::StatusCode,
    web::{Data, Json},
    Body, Request, Result,
};
use serde_json::{json, Value};
use sha2::Sha256;
use vodozemac::olm::{Message, OlmMessage, PreKeyMessage, Session as OlmSession};

use crate::api::message::{
    send_message, ChatMessageContent, ChatMessagePayload, MessageDetail, MessageTarget,
};
use crate::api::DateTime;
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

/// Unified handler for all room-related endpoints
/// Handles send, typing, and joined_members endpoints by parsing path manually
#[handler]
pub async fn rooms_handler(state: Data<&State>, body: Body, req: &Request) -> Result<Json<Value>> {
    // Validate access token and get uid
    let uid = super::auth::validate_access_token(&state, req).await?;

    let path = req.original_uri().path();

    tracing::debug!("rooms_handler, path={}, method={}", path, req.method());

    // Check if it's a room endpoint
    if !path.starts_with("/_matrix/client/v3/rooms/") {
        return Err(poem::error::Error::from_string(
            "Invalid path",
            StatusCode::BAD_REQUEST,
        ));
    }

    let path = path
        .strip_prefix("/_matrix/client/v3/rooms/")
        .ok_or_else(|| poem::error::Error::from_string("Invalid path", StatusCode::BAD_REQUEST))?;

    let parts: Vec<&str> = path.split('/').collect();

    // Validate room_id
    let room_id = super::auth::decode_path_segment(parts[0])?;

    // Check if this is the members, joined_members or read_markers endpoint
    if parts.len() == 2 && parts[1] == "members" {
        tracing::info!("Room endpoint handled: members, room_id={}", room_id);
        return handle_members(&state, &room_id, uid).await;
    } else if parts.len() == 2 && parts[1] == "joined_members" {
        tracing::info!("Room endpoint handled: joined_members, room_id={}", room_id);
        return handle_joined_members(&state, &room_id, uid).await;
    } else if parts.len() == 2 && parts[1] == "read_markers" {
        tracing::info!("Room endpoint handled: read_markers, room_id={}", room_id);
        return handle_read_markers(&state, &room_id, uid, req, body).await;
    }

    // Handle state endpoints
    // GET /_matrix/client/v3/rooms/{room_id}/state/{event_type}
    // PUT /_matrix/client/v3/rooms/{room_id}/state/{event_type}
    if parts.len() >= 2 && parts[1] == "state" {
        let method = req.method();

        tracing::debug!("state endpoint: method={}, parts={:?}", method, parts);

        // Handle PUT for setting room encryption
        if method == poem::http::Method::PUT && parts.len() >= 3 {
            let event_type = super::auth::decode_path_segment(parts[2])?;
            if event_type == "m.room.encryption" {
                tracing::info!("PUT room encryption state: room_id={}", room_id);
                return handle_set_room_encryption(&state, body, &room_id, uid).await;
            }
        }

        return handle_room_state(&state, &room_id, &parts, uid).await;
    }

    // Handle other endpoints that need event_type parsing
    if parts.len() < 2 {
        tracing::warn!(
            "Invalid path format, part split invalid, full path {}",
            req.original_uri().path()
        );
        return Err(poem::error::Error::from_string(
            "Invalid path format, parts inavlid",
            StatusCode::BAD_REQUEST,
        ));
    }

    let event_type = super::auth::decode_path_segment(parts[1])?;

    // Handle send and typing endpoints
    if event_type == "typing" {
        tracing::info!("Room endpoint handled: event_type=typing");
        return Ok(Json(serde_json::json!({})));
    } else if event_type == "send" {
        // Parse send_type from parts[2] (e.g. m.room.message, m.reaction)
        let send_type = parts
            .get(2)
            .map(|s| super::auth::decode_path_segment(s).ok());

        match send_type {
            Some(Some(ref st)) if st == "m.room.message" => {
                tracing::info!("Room endpoint handled: event_type=send, send_type=m.room.message");
                return handle_send_message(&state, body, &room_id, uid).await;
            }
            Some(Some(ref st)) if st == "m.reaction" => {
                tracing::info!("Room endpoint handled: event_type=send, send_type=m.reaction");
                return Ok(Json(json!({})));
            }
            Some(Some(ref st)) if st == "m.room.encrypted" => {
                // Handle encrypted messages - parse and store the encrypted payload
                tracing::info!(
                    "Room endpoint handled: event_type=send, send_type=m.room.encrypted, room_id={}, uid={}",
                    room_id,
                    uid
                );
                return handle_send_encrypted_message(&state, body, &room_id, uid).await;
            }
            Some(Some(st)) => {
                tracing::warn!(
                    "Room endpoint: event_type=send, unsupported send_type={}",
                    st
                );
                return Err(poem::error::Error::from_string(
                    format!("Unsupported send_type: {}", st),
                    StatusCode::BAD_REQUEST,
                ));
            }
            Some(None) => {
                return Err(poem::error::Error::from_string(
                    "Invalid send_type encoding",
                    StatusCode::BAD_REQUEST,
                ));
            }
            None => {
                tracing::warn!("Room endpoint: event_type=send, missing send_type");
                return Err(poem::error::Error::from_string(
                    "Missing send_type in path",
                    StatusCode::BAD_REQUEST,
                ));
            }
        }
    } else if event_type == "redact" {
        tracing::info!("Room endpoint handled: event_type=redact");
        return handle_redact_message(&state, body, &room_id, uid).await;
    } else if event_type == "receipt" {
        // Handle receipt endpoint: /rooms/{room_id}/receipt/m.read/{event_id}
        // parts[2] = receipt_type (e.g., "m.read")
        // parts[3] = event_id
        let receipt_type = parts
            .get(2)
            .map(|s| super::auth::decode_path_segment(s).ok());
        let event_id = parts
            .get(3)
            .map(|s| super::auth::decode_path_segment(s).ok());

        match (receipt_type, event_id) {
            (Some(Some(ref rt)), Some(Some(ref eid))) if rt == "m.read" => {
                tracing::info!(
                    "Room endpoint handled: receipt_type=m.read, event_id={}, room_id={}, uid={}",
                    eid,
                    room_id,
                    uid
                );
                return handle_read_receipt(&state, body, &room_id, uid, eid).await;
            }
            (Some(Some(ref rt)), Some(Some(_))) => {
                tracing::warn!(
                    "Room endpoint: event_type=receipt, unsupported receipt_type={}",
                    rt
                );
                return Err(poem::error::Error::from_string(
                    format!("Unsupported receipt_type: {}", rt),
                    StatusCode::BAD_REQUEST,
                ));
            }
            _ => {
                return Err(poem::error::Error::from_string(
                    "Invalid receipt path format",
                    StatusCode::BAD_REQUEST,
                ));
            }
        }
    }

    tracing::warn!(
        "Invalid path format, full path: {}",
        req.original_uri().path()
    );

    Err(poem::error::Error::from_string(
        "Invalid path format.",
        StatusCode::BAD_REQUEST,
    ))
}

/// Handle Matrix room state endpoints
async fn handle_room_state(
    state: &State,
    room_id: &str,
    parts: &[&str],
    _uid: i64,
) -> Result<Json<Value>> {
    if parts.len() < 3 {
        return Err(poem::error::Error::from_string(
            "Missing event_type in state path",
            StatusCode::BAD_REQUEST,
        ));
    }

    let event_type = super::auth::decode_path_segment(parts[2])?;

    // Handle m.room.member event type
    if event_type == "m.room.member" {
        if parts.len() < 4 {
            return Err(poem::error::Error::from_string(
                "Missing user_id in m.room.member state path",
                StatusCode::BAD_REQUEST,
            ));
        }

        let user_id = super::auth::decode_path_segment(parts[3])?;
        return handle_room_member_state(state, room_id, &user_id).await;
    }

    // Handle m.room.encryption event type
    if event_type == "m.room.encryption" {
        tracing::info!(
            "Room encryption state requested: room_id={}, checking database...",
            room_id
        );
        match state
            .room_encryption_manager
            .get_room_encryption(room_id)
            .await
        {
            Ok(Some(encryption)) => {
                tracing::info!(
                    "Room encryption state found: room_id={}, algorithm={}",
                    room_id,
                    encryption.algorithm
                );
                return Ok(Json(json!({
                    "algorithm": encryption.algorithm,
                    "rotation_period_msgs": encryption.rotation_period_msgs,
                    "rotation_period_ms": encryption.rotation_period_ms
                })));
            }
            Ok(None) => {
                tracing::info!(
                    "Room encryption state not found: room_id={}, room is not encrypted",
                    room_id
                );
                return Err(super::auth::matrix_error(
                    StatusCode::NOT_FOUND,
                    "M_NOT_FOUND",
                    "No encryption event found - room is not encrypted",
                ));
            }
            Err(e) => {
                tracing::error!("Failed to get room encryption state: {}", e);
                return Err(InternalServerError(e));
            }
        }
    }

    // Unknown event type
    tracing::warn!("Unknown state event type: {}", event_type);
    Err(super::auth::matrix_error(
        StatusCode::NOT_FOUND,
        "M_NOT_FOUND",
        "Unknown state event type",
    ))
}

/// Handle setting room encryption state
async fn handle_set_room_encryption(
    state: &State,
    body: Body,
    room_id: &str,
    _uid: i64,
) -> Result<Json<Value>> {
    let body_bytes = body.into_bytes().await.map_err(|e| {
        poem::error::Error::from_string(
            format!("Failed to read body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    let encryption_event: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        poem::error::Error::from_string(
            format!("Invalid JSON body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    let algorithm = encryption_event
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("m.megolm.v1.aes-sha2");

    // Validate algorithm
    if algorithm != "m.megolm.v1.aes-sha2" {
        return Err(super::auth::matrix_error(
            StatusCode::BAD_REQUEST,
            "M_UNSUPPORTED_ENCRYPTION_ALGORITHM",
            "Unsupported encryption algorithm. Only m.megolm.v1.aes-sha2 is supported.",
        ));
    }

    // Only DM rooms can be encrypted (1:1 chat)
    // Group rooms are not supported for encryption yet
    if !room_id.starts_with("!dm_") {
        return Err(super::auth::matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "Only DM rooms can be encrypted at this time. Group room encryption is not supported.",
        ));
    }

    // Extract optional rotation settings
    let rotation_period_msgs = encryption_event
        .get("rotation_period_msgs")
        .and_then(|v| v.as_i64());
    let rotation_period_ms = encryption_event
        .get("rotation_period_ms")
        .and_then(|v| v.as_i64());

    // Enable encryption for the room
    state
        .room_encryption_manager
        .enable_room_encryption(room_id, algorithm, rotation_period_msgs, rotation_period_ms)
        .await
        .map_err(|e| {
            tracing::error!("Failed to enable room encryption: {}", e);
            InternalServerError(e)
        })?;

    tracing::info!(
        "Room encryption enabled: room_id={}, algorithm={}",
        room_id,
        algorithm
    );

    // Return event_id per Matrix spec
    Ok(Json(json!({
        "event_id": format!("$enc_{}", DateTime::now().timestamp_millis())
    })))
}

/// Handle Matrix room member state request
async fn handle_room_member_state(
    state: &State,
    _room_id: &str,
    user_id: &str,
) -> Result<Json<Value>> {
    let matrix_domain = super::auth::get_matrix_domain(state);

    // Parse user_id to get username (format: @username:domain)
    let username = super::auth::parse_and_validate_matrix_user_id(user_id, &matrix_domain)?;

    // Get user info from cache to find displayname
    let cache = state.cache.read().await;
    let displayname = cache
        .users
        .values()
        .find(|user| user.name.eq_ignore_ascii_case(&username))
        .map(|user| user.name.clone())
        .unwrap_or_else(|| username.clone());
    drop(cache);

    // Return member state with membership=join, displayname, and empty avatar_url
    Ok(Json(json!({
        "membership": "join",
        "displayname": displayname,
        "avatar_url": null
    })))
}

/// Handle Matrix members request
/// Returns room member events in Matrix event format
async fn handle_members(state: &State, room_id: &str, _uid: i64) -> Result<Json<Value>> {
    let matrix_domain = super::auth::get_matrix_domain(state);
    let domain_suffix = format!(":{}", matrix_domain);

    let mut chunk = Vec::new();
    let now_ts = DateTime::now().timestamp_millis();

    // Parse room_id to determine room type
    if room_id.starts_with("!dm_") {
        // DM room: !dm_{uid1}_{uid2}:{matrix_domain}
        let room_part = room_id
            .strip_prefix("!dm_")
            .and_then(|s| s.strip_suffix(&domain_suffix))
            .ok_or_else(|| {
                poem::error::Error::from_string(
                    "Invalid DM room_id format",
                    StatusCode::BAD_REQUEST,
                )
            })?;

        let parts: Vec<&str> = room_part.split('_').collect();
        if parts.len() != 2 {
            return Err(poem::error::Error::from_string(
                "Invalid DM room_id format",
                StatusCode::BAD_REQUEST,
            ));
        }

        let uid1: i64 = parts[0].parse().map_err(|_| {
            poem::error::Error::from_string("Invalid uid in room_id", StatusCode::BAD_REQUEST)
        })?;
        let uid2: i64 = parts[1].parse().map_err(|_| {
            poem::error::Error::from_string("Invalid uid in room_id", StatusCode::BAD_REQUEST)
        })?;

        // Get user info from cache
        let cache = state.cache.read().await;

        for (idx, uid) in [uid1, uid2].iter().enumerate() {
            if let Some(user) = cache.users.get(uid) {
                let user_id = format!("@{}:{}", user.name, matrix_domain);
                let avatar_url = format!("mxc://{}/avatar/{}", matrix_domain, uid);

                // Create member event
                let event = json!({
                    "content": {
                        "membership": "join",
                        "displayname": user.name,
                        "avatar_url": avatar_url
                    },
                    "room_id": room_id,
                    "sender": user_id,
                    "state_key": user_id,
                    "type": "m.room.member",
                    "event_id": format!("${}_{}", room_id.replace(':', "_").replace('!', ""), idx),
                    "origin_server_ts": now_ts
                });
                chunk.push(event);
            }
        }
    } else if room_id.starts_with("!group_") {
        // Group room: !group_{gid}:{matrix_domain}
        let gid_str = room_id
            .strip_prefix("!group_")
            .and_then(|s| s.strip_suffix(&domain_suffix))
            .ok_or_else(|| {
                poem::error::Error::from_string(
                    "Invalid group room_id format",
                    StatusCode::BAD_REQUEST,
                )
            })?;

        let gid: i64 = gid_str.parse().map_err(|_| {
            poem::error::Error::from_string("Invalid gid in room_id", StatusCode::BAD_REQUEST)
        })?;

        // Get group members from cache
        let cache = state.cache.read().await;

        if let Some(group) = cache.groups.get(&gid) {
            // Get all members
            for (idx, member_uid) in group.members.iter().enumerate() {
                if let Some(user) = cache.users.get(member_uid) {
                    let user_id = format!("@{}:{}", user.name, matrix_domain);
                    let avatar_url = format!("mxc://{}/avatar/{}", matrix_domain, member_uid);

                    // Create member event
                    let event = json!({
                        "content": {
                            "membership": "join",
                            "displayname": user.name,
                            "avatar_url": avatar_url
                        },
                        "room_id": room_id,
                        "sender": user_id,
                        "state_key": user_id,
                        "type": "m.room.member",
                        "event_id": format!("${}_{}", room_id.replace(':', "_").replace('!', ""), idx),
                        "origin_server_ts": now_ts
                    });
                    chunk.push(event);
                }
            }
        } else {
            return Err(super::auth::matrix_error(
                StatusCode::NOT_FOUND,
                "M_NOT_FOUND",
                "Room not found",
            ));
        }
    } else {
        return Err(poem::error::Error::from_string(
            "Unknown room_id format",
            StatusCode::BAD_REQUEST,
        ));
    }

    Ok(Json(json!({
        "chunk": chunk
    })))
}

/// Handle Matrix joined_members request
async fn handle_joined_members(state: &State, room_id: &str, _uid: i64) -> Result<Json<Value>> {
    let matrix_domain = super::auth::get_matrix_domain(state);
    let domain_suffix = format!(":{}", matrix_domain);

    let mut members_map = serde_json::Map::new();

    // Parse room_id to determine room type
    if room_id.starts_with("!dm_") {
        // DM room: !dm_{uid1}_{uid2}:{matrix_domain}
        let room_part = room_id
            .strip_prefix("!dm_")
            .and_then(|s| s.strip_suffix(&domain_suffix))
            .ok_or_else(|| {
                poem::error::Error::from_string(
                    "Invalid DM room_id format",
                    StatusCode::BAD_REQUEST,
                )
            })?;

        let parts: Vec<&str> = room_part.split('_').collect();
        if parts.len() != 2 {
            return Err(poem::error::Error::from_string(
                "Invalid DM room_id format",
                StatusCode::BAD_REQUEST,
            ));
        }

        let uid1: i64 = parts[0].parse().map_err(|_| {
            poem::error::Error::from_string("Invalid uid in room_id", StatusCode::BAD_REQUEST)
        })?;
        let uid2: i64 = parts[1].parse().map_err(|_| {
            poem::error::Error::from_string("Invalid uid in room_id", StatusCode::BAD_REQUEST)
        })?;

        // Get user info from cache
        let cache = state.cache.read().await;

        for uid in [uid1, uid2] {
            if let Some(user) = cache.users.get(&uid) {
                let user_id = format!("@{}:{}", user.name, matrix_domain);

                let mut member_info = serde_json::Map::new();
                member_info.insert("display_name".to_string(), json!(user.name));

                // Add avatar_url if available
                let avatar_url = format!("mxc://{}/avatar/{}", matrix_domain, uid);
                member_info.insert("avatar_url".to_string(), json!(avatar_url));

                members_map.insert(user_id, json!(member_info));
            }
        }
    } else if room_id.starts_with("!group_") {
        // Group room: !group_{gid}:{matrix_domain}
        let gid_str = room_id
            .strip_prefix("!group_")
            .and_then(|s| s.strip_suffix(&domain_suffix))
            .ok_or_else(|| {
                poem::error::Error::from_string(
                    "Invalid group room_id format",
                    StatusCode::BAD_REQUEST,
                )
            })?;

        let gid: i64 = gid_str.parse().map_err(|_| {
            poem::error::Error::from_string("Invalid gid in room_id", StatusCode::BAD_REQUEST)
        })?;

        // Get group members from cache
        let cache = state.cache.read().await;

        if let Some(group) = cache.groups.get(&gid) {
            // Get all members
            for member_uid in &group.members {
                if let Some(user) = cache.users.get(member_uid) {
                    let user_id = format!("@{}:{}", user.name, matrix_domain);

                    let mut member_info = serde_json::Map::new();
                    member_info.insert("display_name".to_string(), json!(user.name));

                    // Add avatar_url
                    let avatar_url = format!("mxc://{}/avatar/{}", matrix_domain, member_uid);
                    member_info.insert("avatar_url".to_string(), json!(avatar_url));

                    members_map.insert(user_id, json!(member_info));
                }
            }
        } else {
            return Err(super::auth::matrix_error(
                StatusCode::NOT_FOUND,
                "M_NOT_FOUND",
                "Room not found",
            ));
        }
    } else {
        return Err(poem::error::Error::from_string(
            "Unknown room_id format",
            StatusCode::BAD_REQUEST,
        ));
    }

    Ok(Json(json!({
        "joined": members_map
    })))
}

/// Handle Matrix read_markers request
async fn handle_read_markers(
    state: &State,
    room_id: &str,
    uid: i64,
    req: &Request,
    body: Body,
) -> Result<Json<Value>> {
    let method = req.method();

    if method == poem::http::Method::GET {
        // Find the latest event_id in this room for the bot user
        let latest_event_id = get_latest_event_id_for_room(state, room_id).await;

        Ok(Json(json!({
            "m.fully_read": latest_event_id,
            "m.read": latest_event_id
        })))
    } else if method == poem::http::Method::POST {
        // Read the body to log it (client sends m.fully_read and m.read event_ids)
        let body_bytes = body.into_bytes().await.map_err(|e| {
            poem::error::Error::from_string(
                format!("Failed to read body: {}", e),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let body_str = String::from_utf8_lossy(&body_bytes);
        tracing::info!(
            "read_markers POST: uid={}, room_id={}, body={}",
            uid,
            room_id,
            body_str
        );

        // Successfully marked as read, return empty JSON per Matrix spec
        Ok(Json(json!({})))
    } else {
        Err(super::auth::matrix_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "M_UNRECOGNIZED",
            "Method not allowed",
        ))
    }
}

/// Get the latest event_id for a room by finding the most recent message
async fn get_latest_event_id_for_room(state: &State, room_id: &str) -> String {
    let bot_uids = super::sync::get_bot_uids().await;

    // Try to find a recent message for this bot user
    for bot_uid in &bot_uids {
        if let Ok(msgs) = state
            .msg_db
            .messages()
            .fetch_user_messages_after(*bot_uid, None, 1)
        {
            if let Some((mid, msg_bytes)) = msgs.first() {
                if let Ok(payload) = serde_json::from_slice::<ChatMessagePayload>(msg_bytes) {
                    let matrix_domain = super::auth::get_matrix_domain(state);
                    let msg_room_id = get_room_id(&payload, &matrix_domain);
                    if msg_room_id == room_id {
                        return format!("${}", mid);
                    }
                }
            }
        }
    }

    // Fallback: return a default event_id
    "$0".to_string()
}

/// Get room id from message payload
fn get_room_id(payload: &ChatMessagePayload, matrix_domain: &str) -> String {
    match &payload.target {
        MessageTarget::User(target_user) => {
            let sender_uid = payload.from_uid;
            let bot_uid = target_user.uid;
            format!("!dm_{}_{}:{}", sender_uid, bot_uid, matrix_domain)
        }
        MessageTarget::Group(group) => {
            format!("!group_{}:{}", group.gid, matrix_domain)
        }
    }
}

/// Parse room_id to extract target and sender info
/// Returns (target, sender_uid)
fn parse_room_target(
    room_id: &str,
    uid: i64,
    matrix_domain: &str,
) -> Result<(MessageTarget, i64), poem::Error> {
    let domain_suffix = format!(":{}", matrix_domain);

    if room_id.starts_with("!dm_") {
        let room_part = room_id
            .strip_prefix("!dm_")
            .and_then(|s| s.strip_suffix(&domain_suffix))
            .ok_or_else(|| {
                poem::error::Error::from_string(
                    "Invalid DM room_id format",
                    StatusCode::BAD_REQUEST,
                )
            })?;

        let parts: Vec<&str> = room_part.split('_').collect();
        if parts.len() != 2 {
            return Err(poem::error::Error::from_string(
                "Invalid DM room_id format",
                StatusCode::BAD_REQUEST,
            ));
        }

        let sender_uid: i64 = parts[0].parse().map_err(|_| {
            poem::error::Error::from_string(
                "Invalid sender uid in room_id",
                StatusCode::BAD_REQUEST,
            )
        })?;
        let bot_uid: i64 = parts[1].parse().map_err(|_| {
            poem::error::Error::from_string("Invalid bot uid in room_id", StatusCode::BAD_REQUEST)
        })?;

        if uid != bot_uid {
            return Err(poem::error::Error::from_string(
                "Unauthorized: token does not match bot_uid in room_id",
                StatusCode::FORBIDDEN,
            ));
        }

        Ok((MessageTarget::user(sender_uid), bot_uid))
    } else if room_id.starts_with("!group_") {
        let gid_str = room_id
            .strip_prefix("!group_")
            .and_then(|s| s.strip_suffix(&domain_suffix))
            .ok_or_else(|| {
                poem::error::Error::from_string(
                    "Invalid group room_id format",
                    StatusCode::BAD_REQUEST,
                )
            })?;
        let gid: i64 = gid_str.parse().map_err(|_| {
            poem::error::Error::from_string("Invalid gid in room_id", StatusCode::BAD_REQUEST)
        })?;

        Ok((MessageTarget::group(gid), uid))
    } else {
        Err(poem::error::Error::from_string(
            "Unknown room_id format",
            StatusCode::BAD_REQUEST,
        ))
    }
}

/// Handle Matrix send message request
async fn handle_send_message(
    state: &State,
    body: Body,
    room_id: &str,
    uid: i64,
) -> Result<Json<Value>> {
    let matrix_domain = super::auth::get_matrix_domain(state);

    // Parse room_id to determine target
    let (target, sender_uid) = parse_room_target(room_id, uid, &matrix_domain)?;

    // Read and print the request body
    let body_bytes = body.into_bytes().await.map_err(|e| {
        poem::error::Error::from_string(
            format!("Failed to read body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;
    let body_str = String::from_utf8_lossy(&body_bytes);
    tracing::info!("Matrix send message body: {}", body_str);

    // Parse Matrix message format to extract message content
    let matrix_msg: Value = serde_json::from_str(&body_str).map_err(|e| {
        poem::error::Error::from_string(
            format!("Invalid JSON body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    let msg_body = matrix_msg
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Check if message has format field (indicates formatted content like markdown)
    // Matrix spec: if "format" is present (e.g., "org.matrix.custom.html"), use text/markdown
    let content_type = if matrix_msg.get("format").is_some() {
        "text/markdown"
    } else {
        "text/plain"
    };

    tracing::info!("Received message from {}: {}", sender_uid, msg_body);

    // Echo back the message
    let reply_payload = ChatMessagePayload {
        from_uid: sender_uid,
        target,
        detail: MessageDetail::Normal(crate::api::message::MessageNormal {
            content: ChatMessageContent {
                content_type: content_type.to_string(),
                content: format!("{}", msg_body),
                properties: None,
            },
            expires_in: None,
        }),
        created_at: DateTime::now(),
    };

    match send_message(&state, reply_payload).await {
        Ok(mid) => {
            tracing::info!("Replied with message id: {}", mid);
            Ok(Json(serde_json::json!({
                "event_id": format!("${}", mid)
            })))
        }
        Err(e) => {
            tracing::error!("Failed to send reply: {}", e);
            Err(e)
        }
    }
}

/// Handle Matrix encrypted message request
async fn handle_send_encrypted_message(
    state: &State,
    body: Body,
    room_id: &str,
    uid: i64,
) -> Result<Json<Value>> {
    let matrix_domain = super::auth::get_matrix_domain(state);

    // Parse room_id to determine target
    let (target, _bot_uid) = parse_room_target(room_id, uid, &matrix_domain)?;

    // Extract sender_uid from target
    let sender_uid = match &target {
        MessageTarget::User(u) => u.uid,
        MessageTarget::Group(g) => g.gid, // For group, this is gid not uid
    };

    // Read and parse the encrypted message body
    let body_bytes = body.into_bytes().await.map_err(|e| {
        poem::error::Error::from_string(
            format!("Failed to read body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;
    tracing::info!(
        "Encrypted message received: room_id={}, uid={}",
        room_id,
        uid
    );

    let encrypted_msg: Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        poem::error::Error::from_string(
            format!("Invalid JSON body: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    // Parse encrypted content
    // Matrix encrypted message format (Olm):
    // {
    //   "algorithm": "m.olm.v1.curve25519-aes-sha2",
    //   "sender_key": "<curve25519 key>",
    //   "ciphertext": {
    //     "<our_curve25519_key>": {
    //       "type": 0 or 1,
    //       "body": "<base64 encoded message>"
    //     }
    //   }
    // }
    let content = encrypted_msg.get("content").unwrap_or(&encrypted_msg);
    let algorithm = content
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("m.olm.v1.curve25519-aes-sha2");
    let sender_key = content
        .get("sender_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    tracing::info!(
        "Encrypted message: algorithm={}, sender_key={}",
        algorithm,
        sender_key
    );

    // Branch by algorithm: Megolm vs Olm
    if algorithm == "m.megolm.v1.aes-sha2" {
        return handle_megolm_encrypted_message(&state, &encrypted_msg, room_id, uid).await;
    }

    // --- Olm handling (original code) ---
    let ciphertext_obj = content.get("ciphertext").and_then(|v| v.as_object());

    // Get bot's device keys to find our curve25519 key
    let bot_device_keys = state
        .device_keys_manager
        .get_user_device_keys(uid)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get bot device keys: {}", e);
            InternalServerError(e)
        })?;

    // Find the ciphertext for our key
    let mut our_ciphertext: Option<(u32, String)> = None;
    let mut our_curve25519_key: Option<String> = None;

    for device in &bot_device_keys {
        let key_str = format!("m.olm.v1.curve25519-aes-sha2:{}", device.curve25519_key);
        if let Some(ct) = ciphertext_obj.and_then(|c| c.get(&device.curve25519_key)) {
            let msg_type = ct.get("type").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let body = ct.get("body").and_then(|v| v.as_str()).unwrap_or("");
            our_ciphertext = Some((msg_type, body.to_string()));
            our_curve25519_key = Some(device.curve25519_key.clone());
            break;
        }
        // Also check with algorithm prefix
        if let Some(ct) = ciphertext_obj.and_then(|c| c.get(&key_str)) {
            let msg_type = ct.get("type").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let body = ct.get("body").and_then(|v| v.as_str()).unwrap_or("");
            our_ciphertext = Some((msg_type, body.to_string()));
            our_curve25519_key = Some(device.curve25519_key.clone());
            break;
        }
    }

    // If not found in device_keys_manager, try to get curve25519 key from device_keys
    if our_ciphertext.is_none() {
        for device in &bot_device_keys {
            our_curve25519_key = Some(device.curve25519_key.clone());
            break;
        }
    }

    let (msg_type, ciphertext_b64) = match our_ciphertext {
        Some(ct) => ct,
        None => {
            tracing::warn!(
                "No ciphertext found for bot's curve25519 key. Available keys: {:?}",
                bot_device_keys
                    .iter()
                    .map(|d| &d.curve25519_key)
                    .collect::<Vec<_>>()
            );
            return Err(poem::error::Error::from_string(
                "No ciphertext found for this device",
                StatusCode::BAD_REQUEST,
            ));
        }
    };

    tracing::info!(
        "Found ciphertext for our key: {}, msg_type={}",
        our_curve25519_key
            .as_ref()
            .unwrap_or(&"unknown".to_string()),
        msg_type
    );

    // Try to decrypt the message
    // First, try to find an existing session, or create one from pre-key message
    let decrypted = decrypt_olm_message(&state, uid, sender_key, msg_type, &ciphertext_b64).await?;

    // Convert decrypted bytes to string
    let plaintext = decrypted;
    tracing::info!("Decrypted message: {}", plaintext);

    // Parse the decrypted message to extract the actual content
    // Decrypted format: {"type":"m.room.message","content":{"msgtype":"m.text","body":"Hello"}}
    let decrypted_json: Value = serde_json::from_str(&plaintext).unwrap_or_else(|e| {
        tracing::warn!("Failed to parse decrypted JSON: {}", e);
        json!({
            "type": "m.room.message",
            "content": {
                "body": plaintext,
                "msgtype": "m.text"
            }
        })
    });

    let message_body = decrypted_json
        .get("content")
        .and_then(|c| c.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("[decrypted message]");
    let message_type = decrypted_json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("m.room.message");

    tracing::info!(
        "Decrypted message content: type={}, body={}",
        message_type,
        message_body
    );

    // Store the decrypted message
    let reply_payload = ChatMessagePayload {
        from_uid: sender_uid,
        target,
        detail: MessageDetail::Normal(crate::api::message::MessageNormal {
            content: ChatMessageContent {
                content_type: "text/plain".to_string(),
                content: message_body.to_string(),
                properties: None,
            },
            expires_in: None,
        }),
        created_at: DateTime::now(),
    };

    match send_message(&state, reply_payload).await {
        Ok(mid) => {
            tracing::info!("Decrypted message stored with id: {}", mid);
            Ok(Json(serde_json::json!({
                "event_id": format!("${}", mid)
            })))
        }
        Err(e) => {
            tracing::error!("Failed to store decrypted message: {}", e);
            Err(e)
        }
    }
}

/// Handle Megolm (m.megolm.v1.aes-sha2) encrypted message
/// Megolm is used for room-level encryption where all recipients share a single session
async fn handle_megolm_encrypted_message(
    state: &State,
    encrypted_msg: &Value,
    room_id: &str,
    uid: i64,
) -> Result<Json<Value>> {
    let matrix_domain = super::auth::get_matrix_domain(state);

    // Parse room_id to determine target and bot_uid
    // For DM room !dm_{sender_uid}_{bot_uid}:domain
    // target = User(sender_uid), bot_uid is the bot receiving the message
    let (target, bot_uid) = parse_room_target(room_id, uid, &matrix_domain)?;

    let content = encrypted_msg.get("content").unwrap_or(encrypted_msg);

    // Megolm message format:
    // {
    //   "algorithm": "m.megolm.v1.aes-sha2",
    //   "sender_key": "<curve25519 key>",
    //   "session_id": "<session id>",
    //   "ciphertext": "<base64 string>"  (NOT a JSON object like Olm)
    // }
    let session_id = content
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let ciphertext_str = content
        .get("ciphertext")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if session_id.is_empty() || ciphertext_str.is_empty() {
        tracing::warn!(
            "Megolm message missing session_id or ciphertext: session_id={}, ciphertext_len={}",
            session_id,
            ciphertext_str.len()
        );
        return Err(poem::error::Error::from_string(
            "Missing session_id or ciphertext in Megolm message",
            StatusCode::BAD_REQUEST,
        ));
    }

    tracing::info!(
        "Megolm encrypted message: session_id={}, ciphertext_len={}",
        session_id,
        ciphertext_str.len()
    );

    // Look up the inbound Megolm session by session_id
    let inbound_session_record = state
        .megolm_session_manager
        .get_inbound_session(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get Megolm inbound session: {}", e);
            InternalServerError(e)
        })?;

    if inbound_session_record.is_none() {
        tracing::warn!(
            "No inbound Megolm session found for session_id={}. The session key has not been received yet. Message will be accepted but cannot be decrypted.",
            session_id
        );
        // Return 200 OK to accept the message - the bridge will not retry
        // Once the room_key is received via sendToDevice, future messages will be decryptable
        return Ok(Json(json!({
            "event_id": format!("${}", 0)
        })));
    }

    let inbound_session_record = inbound_session_record.unwrap();

    // Get server key for pickle decryption
    let pickle_key = {
        let config = state.key_config.read().await;
        get_pickle_key(&config.server_key)
    };

    // Decrypt the session pickle
    let pickle_str = String::from_utf8_lossy(&inbound_session_record.session_data);
    let decrypted_pickle =
        vodozemac::megolm::InboundGroupSessionPickle::from_encrypted(&pickle_str, &pickle_key)
            .map_err(|e| {
                tracing::error!("Failed to decrypt Megolm session pickle: {}", e);
                poem::error::Error::from_string(
                    format!("Failed to decrypt session pickle: {}", e),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            })?;

    let mut inbound_session = vodozemac::megolm::InboundGroupSession::from_pickle(decrypted_pickle);

    // Parse the ciphertext as a MegolmMessage
    let megolm_message: vodozemac::megolm::MegolmMessage =
        ciphertext_str.try_into().map_err(|e| {
            tracing::error!("Failed to parse Megolm ciphertext: {}", e);
            poem::error::Error::from_string(
                format!("Invalid Megolm ciphertext: {}", e),
                StatusCode::BAD_REQUEST,
            )
        })?;

    // Decrypt the Megolm message
    let decrypted = inbound_session.decrypt(&megolm_message).map_err(|e| {
        tracing::error!("Failed to decrypt Megolm message: {}", e);
        poem::error::Error::from_string(
            format!("Failed to decrypt Megolm message: {}", e),
            StatusCode::BAD_REQUEST,
        )
    })?;

    // Update session last used
    let _ = state
        .megolm_session_manager
        .update_session_last_used(session_id)
        .await;

    // Convert decrypted bytes to string
    let plaintext = String::from_utf8_lossy(&decrypted.plaintext).to_string();
    tracing::info!("Decrypted Megolm message: {}", plaintext);

    // Parse the decrypted message to extract the actual content
    let decrypted_json: Value = serde_json::from_str(&plaintext).unwrap_or_else(|e| {
        tracing::warn!("Failed to parse decrypted Megolm JSON: {}", e);
        json!({
            "type": "m.room.message",
            "content": {
                "body": plaintext,
                "msgtype": "m.text"
            }
        })
    });

    let message_body = decrypted_json
        .get("content")
        .and_then(|c| c.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("[decrypted message]");
    let message_type = decrypted_json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("m.room.message");

    tracing::info!(
        "Decrypted Megolm message content: type={}, body={}",
        message_type,
        message_body
    );

    // Store the decrypted message
    // from_uid = bot_uid (the bot is the sender of the stored message)
    // target = User(sender_uid) (send to the user who sent the original message)
    let reply_payload = ChatMessagePayload {
        from_uid: bot_uid,
        target,
        detail: MessageDetail::Normal(crate::api::message::MessageNormal {
            content: ChatMessageContent {
                content_type: "text/plain".to_string(),
                content: message_body.to_string(),
                properties: None,
            },
            expires_in: None,
        }),
        created_at: DateTime::now(),
    };

    match send_message(&state, reply_payload).await {
        Ok(mid) => {
            tracing::info!("Decrypted Megolm message stored with id: {}", mid);
            Ok(Json(serde_json::json!({
                "event_id": format!("${}", mid)
            })))
        }
        Err(e) => {
            tracing::error!("Failed to store decrypted Megolm message: {}", e);
            Err(e)
        }
    }
}

/// Decrypt an Olm message
/// msg_type 0 = pre-key message, msg_type 1 = normal message
pub(super) async fn decrypt_olm_message(
    state: &State,
    local_uid: i64,
    sender_curve25519_key: &str,
    msg_type: u32,
    ciphertext_b64: &str,
) -> Result<String, poem::Error> {
    // Get sender_uid from sender_key by looking up in device_keys
    let sender_uid = get_uid_from_curve25519_key(&state, sender_curve25519_key)
        .await
        .unwrap_or(0);

    // Get existing sessions for this sender
    let sessions = state
        .olm_session_manager
        .get_inbound_sessions(local_uid, sender_uid)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get inbound sessions: {}", e);
            InternalServerError(e)
        })?;

    // Parse the Olm message from base64 based on message type
    let olm_msg = if msg_type == 0 {
        // Pre-key message
        let pre_key_msg = PreKeyMessage::from_base64(ciphertext_b64).map_err(|e| {
            poem::error::Error::from_string(
                format!("Invalid PreKey message base64: {}", e),
                StatusCode::BAD_REQUEST,
            )
        })?;
        OlmMessage::PreKey(pre_key_msg)
    } else {
        // Normal message
        let msg = Message::from_base64(ciphertext_b64).map_err(|e| {
            poem::error::Error::from_string(
                format!("Invalid Olm message base64: {}", e),
                StatusCode::BAD_REQUEST,
            )
        })?;
        OlmMessage::Normal(msg)
    };

    // Try to decrypt with each session
    for session_record in &sessions {
        // Deserialize session data - it's stored as JSON-pickled Session
        match serde_json::from_slice::<vodozemac::olm::SessionPickle>(&session_record.session_data)
        {
            Ok(pickle) => {
                let mut session = OlmSession::from_pickle(pickle);

                // Try to decrypt the message
                match session.decrypt(&olm_msg) {
                    Ok(plaintext) => {
                        // Update session last used
                        let _ = state
                            .olm_session_manager
                            .update_session_last_used(&session_record.session_id)
                            .await;

                        return Ok(String::from_utf8_lossy(&plaintext).to_string());
                    }
                    Err(e) => {
                        tracing::debug!(
                            "Decryption failed with session {}: {}",
                            session_record.session_id,
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to unpickle session: {}", e);
            }
        }
    }

    // No session found or decryption failed
    // For pre-key messages, we need to create a new session from OTK
    if msg_type == 0 {
        tracing::warn!(
            "Pre-key message received but no session found. Sender: {}",
            sender_curve25519_key
        );
        return Err(poem::error::Error::from_string(
            "No Olm session found for pre-key message - OTK handling not yet implemented",
            StatusCode::BAD_REQUEST,
        ));
    }

    Err(poem::error::Error::from_string(
        "No Olm session found for this sender or decryption failed",
        StatusCode::BAD_REQUEST,
    ))
}

/// Helper to extract sender uid from curve25519 key
pub(super) async fn get_uid_from_curve25519_key(
    state: &State,
    curve25519_key: &str,
) -> Option<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT uid FROM matrix_device_keys WHERE curve25519_key = ? LIMIT 1",
    )
    .bind(curve25519_key)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
}

/// Handle Matrix read receipt request
async fn handle_read_receipt(
    _state: &State,
    body: Body,
    room_id: &str,
    uid: i64,
    event_id: &str,
) -> Result<Json<Value>> {
    // Read the body to log it (client may send thread_id or other info)
    let body_bytes = body.into_bytes().await.ok();
    let body_str = body_bytes
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();

    tracing::info!(
        "Read receipt: uid={}, room_id={}, event_id={}, body={}",
        uid,
        room_id,
        event_id,
        body_str
    );

    // Return empty JSON per Matrix spec
    Ok(Json(json!({})))
}

/// Handle Matrix redact message request
async fn handle_redact_message(
    _state: &State,
    _body: Body,
    _room_id: &str,
    uid: i64,
) -> Result<Json<Value>> {
    Ok(Json(json!({
        "event_id": format!("${}_redact", uid)
    })))
}
