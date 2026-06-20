//! Matrix sync module - handles long polling sync for messages

use once_cell::sync::Lazy;
use poem::{
    handler,
    web::{Data, Json, Query},
    Result,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::RwLock;

use crate::api::message::{ChatMessagePayload, MessageDetail, MessageTarget};
use crate::api::DateTime;
use crate::state::{Cache, CacheGroup, State};

/// Global cache for bot uids and max mid
pub(super) static BOT_CACHE: Lazy<RwLock<BotCache>> = Lazy::new(|| RwLock::new(BotCache::new()));

/// Maximum wait time for long polling
const MAX_WAIT_MS: u64 = 3000;

/// Check interval while waiting
const CHECK_INTERVAL_MS: u64 = 3000;

/// Maximum message age in seconds for sync response (5 minutes)
const MAX_MESSAGE_AGE_SECONDS: i64 = 300;

pub(super) struct BotCache {
    pub(super) bot_uids: Vec<i64>,
    max_mid: i64,
    /// Cache of room_ids for each bot user, keyed by bot uid
    pub(super) room_cache: HashMap<i64, Vec<String>>,
    /// Last refresh timestamp (millis). 0 means never initialized.
    last_refreshed: i64,
}

/// Build list of users whose device keys have changed (for device_lists.changed in sync)
/// This includes all users that have device_keys stored in the database
async fn build_changed_users(state: &State, matrix_domain: &str) -> Vec<String> {
    let cache = state.cache.read().await;

    // Get all users that have device_keys entries
    let users_with_devices = state
        .device_keys_manager
        .get_all_users_with_device_keys()
        .await
        .unwrap_or_default();

    users_with_devices
        .iter()
        .filter_map(|uid| {
            cache
                .users
                .get(uid)
                .map(|user| format!("@{}:{}", user.name, matrix_domain))
        })
        .collect()
}

/// Get bot UIDs from cache
pub(crate) async fn get_bot_uids() -> Vec<i64> {
    let cache = BOT_CACHE.read().await;
    cache.bot_uids.clone()
}

/// Get bot room cache for a specific uid
pub(crate) async fn get_bot_rooms(uid: i64) -> Vec<String> {
    let cache = BOT_CACHE.read().await;
    cache.room_cache.get(&uid).cloned().unwrap_or_default()
}

impl BotCache {
    fn new() -> Self {
        Self {
            bot_uids: Vec::new(),
            max_mid: 0,
            room_cache: HashMap::new(),
            last_refreshed: 0,
        }
    }
}

/// Message event for Matrix sync
#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageEvent {
    pub mid: i64,
    pub from_uid: i64,
    pub room_id: String,
    pub sender: String,
    pub content: Value,
    pub timestamp: i64,
}

/// Sync query parameters
#[derive(Debug, Deserialize)]
pub struct SyncQuery {
    /// Since token in format "s{mid}"
    pub since: Option<String>,
    /// Timeout in milliseconds
    pub timeout: Option<i64>,
}

/// Invalidate bot cache so it gets rebuilt on next access
pub async fn invalidate_bot_cache() {
    let mut cache = BOT_CACHE.write().await;
    cache.bot_uids.clear();
    cache.room_cache.clear();
    cache.last_refreshed = 0;
    // max_mid is only ever increasing, so we leave it as-is
}

/// Initialize bot cache if not already initialized
pub async fn init_bot_cache(state: &State) {
    let cache = BOT_CACHE.read().await;

    // already initialized
    if cache.last_refreshed != 0 {
        return;
    }

    drop(cache);
    let mut cache = BOT_CACHE.write().await;

    let matrix_domain = super::auth::get_matrix_domain(state);

    // Get all bot user IDs and build room_cache from contacts
    let (bot_uids, room_cache): (Vec<i64>, HashMap<i64, Vec<String>>) = {
        let state_cache = state.cache.read().await;
        let bot_uids: Vec<i64> = state_cache
            .users
            .iter()
            .filter(|(_, user)| user.is_bot)
            .map(|(uid, _)| *uid)
            .collect();

        let mut room_cache: HashMap<i64, Vec<String>> = HashMap::new();
        let mut dm_rooms: Vec<String> = Vec::new();

        // Build room_cache from contacts:
        // If uid is a bot, generate DM room with each of its contacts (target_uids)
        // If target_uid is a bot, generate DM room with uid
        for (uid, user) in &state_cache.users {
            for target_uid in user.contacts.keys() {
                // uid is a bot: add room for uid's contact with target_uid
                if user.is_bot {
                    let (uid1, uid2) = if uid < target_uid {
                        (*uid, *target_uid)
                    } else {
                        (*target_uid, *uid)
                    };
                    let room_id = format!("!dm_{}_{}:{}", uid1, uid2, matrix_domain);
                    room_cache.entry(*uid).or_default().push(room_id.clone());
                    dm_rooms.push(room_id);
                }
                // target_uid is a bot: add room for target_uid's contact with uid
                if let Some(target_user) = state_cache.users.get(target_uid) {
                    if target_user.is_bot {
                        let (uid1, uid2) = if uid < target_uid {
                            (*uid, *target_uid)
                        } else {
                            (*target_uid, *uid)
                        };
                        let room_id = format!("!dm_{}_{}:{}", uid1, uid2, matrix_domain);
                        room_cache
                            .entry(*target_uid)
                            .or_default()
                            .push(room_id.clone());
                        dm_rooms.push(room_id);
                    }
                }
            }
        }

        // Add group rooms for each bot that is in the group
        for (gid, group) in &state_cache.groups {
            for bot_uid in &bot_uids {
                if group.contains_user(*bot_uid) {
                    let room_id = format!("!group_{}:{}", gid, matrix_domain);
                    room_cache.entry(*bot_uid).or_default().push(room_id);
                }
            }
        }

        // Deduplicate room_ids for each bot
        for rooms in room_cache.values_mut() {
            rooms.sort();
            rooms.dedup();
        }

        // Deduplicate dm_rooms and enable encryption for each DM room
        dm_rooms.sort();
        dm_rooms.dedup();
        for room_id in &dm_rooms {
            if let Err(e) = state
                .room_encryption_manager
                .enable_room_encryption(room_id, "m.megolm.v1.aes-sha2", Some(100), Some(604800000))
                .await
            {
                tracing::warn!("Failed to enable encryption for DM room {}: {}", room_id, e);
            }
        }

        (bot_uids, room_cache)
    };

    // Get max mid from all bot users
    let mut max_mid: i64 = 0;
    for bot_uid in &bot_uids {
        if let Ok(msgs) = state
            .msg_db
            .messages()
            .fetch_user_messages_after(*bot_uid, None, 1)
        {
            if let Some((mid, _)) = msgs.first() {
                max_mid = max_mid.max(*mid);
            }
        }
    }

    cache.bot_uids = bot_uids;
    cache.max_mid = max_mid;
    cache.room_cache = room_cache;
    cache.last_refreshed = DateTime::now().timestamp_millis();

    tracing::debug!(
        "Refreshed bot cache: bot_uids={:?}, max_mid={}, room_cache={:?}",
        cache.bot_uids,
        cache.max_mid,
        cache.room_cache
    );
}

/// Matrix sync endpoint - implements long polling for Matrix sync
#[handler]
pub async fn sync(
    state: Data<&State>,
    Query(SyncQuery { since, timeout }): Query<SyncQuery>,
    req: &poem::Request,
) -> Result<Json<Value>> {
    // Validate access token
    let uid = super::auth::validate_access_token(&state, req).await?;

    // Mark bot as online when it connects via Matrix (reset 60s timeout)
    if state.bot_online_tx.send((uid, true)).is_ok() {
        // Successfully sent, bot online state will be handled by the background task
    }

    // Ensure bot cache is populated (rebuilds if invalidated by CRUD operations)
    init_bot_cache(&state).await;

    let timeout_ms = timeout.unwrap_or(MAX_WAIT_MS as i64);

    // Calculate deadline for long polling
    let deadline_ms = DateTime::now().timestamp_millis() + timeout_ms;

    // Get bot_uids, max_mid, and room_cache from cache
    let (bot_uids, cached_max_mid, room_cache) = {
        let cache = BOT_CACHE.read().await;
        (
            cache.bot_uids.clone(),
            cache.max_mid,
            cache.room_cache.clone(),
        )
    };

    // Parse since token to get starting message id
    // If since is empty or since_mid > cached_max_mid, use cached max_mid as since_mid
    let since_mid = if let Some(s) = since {
        s.strip_prefix('s')
            .and_then(|s| s.parse::<i64>().ok())
            .map(|mid| mid.min(cached_max_mid))
            .unwrap_or(cached_max_mid)
    } else {
        cached_max_mid
    };

    let mut next_batch = since_mid;
    tracing::debug!(
        "sync: uid={:?}, since_mid={:?}, cached_max_mid={:?}, timeout={}",
        uid,
        since_mid,
        cached_max_mid,
        timeout_ms
    );

    let matrix_domain = super::auth::get_matrix_domain(&state);

    // Build device_lists.changed: list all users that have device keys
    // so mautrix knows to query their keys via keys_query and keys_claim
    let changed_users = build_changed_users(&state, &matrix_domain).await;

    // Get OTK count for device_one_time_keys_count in sync response
    let otk_count = {
        let cache = state.cache.read().await;
        let user = cache.users.get(&uid);
        let token = super::auth::get_token_from_request(req);
        let device_id = user
            .and_then(|u| {
                u.bot_keys
                    .values()
                    .find(|bk| Some(&bk.key) == token.as_ref())
            })
            .map(|bk| bk.name.clone())
            .unwrap_or_else(|| "BOTDEVICE".to_string());
        state
            .device_keys_manager
            .get_one_time_key_count(uid, &device_id)
            .await
            .unwrap_or(0)
    };

    if bot_uids.is_empty() {
        tokio::time::sleep(std::time::Duration::from_millis(timeout_ms as u64)).await;
        tracing::debug!("No bot users found");
        let rooms_join = build_joined_rooms(uid, &room_cache);
        return Ok(Json(empty_sync_response(
            next_batch,
            rooms_join,
            changed_users,
            otk_count,
        )));
    }

    // Long polling loop - wait for messages up to timeout
    loop {
        // Check if we've exceeded the deadline
        let current_time = DateTime::now().timestamp_millis();
        if current_time >= deadline_ms {
            break;
        }

        // Quick check: get max message ID from database and compare with cached_max_mid
        // If max_msg_id <= cached_max_mid, there are no new messages
        let max_msg_id = state.msg_db.get_max_msg_id().unwrap_or(None).unwrap_or(0);
        let remaining_time = deadline_ms - DateTime::now().timestamp_millis();
        let sleep_duration = if remaining_time > 0 {
            (CHECK_INTERVAL_MS as i64).min(remaining_time) as u64
        } else {
            0
        };

        if max_msg_id <= cached_max_mid {
            tokio::time::sleep(std::time::Duration::from_millis(sleep_duration)).await;
            continue;
        }

        // Fetch messages
        let messages = fetch_bot_messages(&state, &bot_uids, since_mid, current_time).await;
        if messages.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(sleep_duration)).await;
            continue;
        }

        // If we have messages, return immediately

        next_batch = messages.last().map(|m| m.mid).unwrap_or(cached_max_mid);

        // Update bot cache max_mid
        {
            let mut cache = BOT_CACHE.write().await;
            cache.max_mid = cache.max_mid.max(next_batch);
        }

        let resp = build_sync_response(messages, next_batch, changed_users, otk_count);
        tracing::debug!(
            "sync response: {}",
            serde_json::to_string_pretty(&resp).unwrap_or_default()
        );
        return Ok(Json(resp));
    }

    let rooms_join = build_joined_rooms(uid, &room_cache);

    // Timeout reached, return empty response
    let resp = empty_sync_response(next_batch, rooms_join, changed_users, otk_count);
    tracing::trace!(
        "sync response (timeout): {}",
        serde_json::to_string_pretty(&resp).unwrap_or_default()
    );
    Ok(Json(resp))
}

/// Fetch all bot messages after a given message id
/// This fetches messages sent TO the bot (by other users), not messages sent BY the bot.
async fn fetch_bot_messages(
    state: &State,
    bot_uids: &[i64],
    since_mid: i64,
    now_millis: i64,
) -> Vec<MessageEvent> {
    let matrix_domain = super::auth::get_matrix_domain(state);

    // If since_mid is 0 (no since token), only include messages from the last 30 seconds
    // Otherwise, use 5 minutes to allow syncing from last checkpoint
    let max_age_seconds = if since_mid == 0 {
        30 // 30 seconds for initial sync
    } else {
        MAX_MESSAGE_AGE_SECONDS
    };

    // Fetch all messages after since_mid in a single call
    let all_msgs = match state
        .msg_db
        .messages()
        .fetch_messages_after(since_mid, 1000)
    {
        Ok(msgs) => msgs,
        Err(e) => {
            tracing::warn!("Failed to fetch messages: {}", e);
            return Vec::new();
        }
    };

    if all_msgs.is_empty() {
        tracing::debug!(
            "fetch_bot_messages: no messages after mid={}, bot_uids={:?}, return empty.",
            since_mid,
            bot_uids
        );
        return Vec::new();
    }

    let all_msg_cnt = all_msgs.len();
    let cache = state.cache.read().await;
    let mut messages = Vec::new();

    // Filter messages for all bots in a single pass
    for (mid, msg_bytes) in all_msgs {
        for bot_uid in bot_uids {
            if let Some(msg_event) = process_message(
                mid,
                &msg_bytes,
                *bot_uid,
                &cache,
                now_millis,
                max_age_seconds,
                &matrix_domain,
            ) {
                messages.push(msg_event);
                break; // Message matched one bot, no need to check others
            }
        }
    }

    tracing::debug!(
        "fetch_bot_messages: fetched {} messages after mid={}, bot_uids={:?}, returning {} messages",
        all_msg_cnt,
        since_mid,
        bot_uids,
        messages.len()
    );
    messages.sort_by_key(|m| m.mid);
    messages.dedup_by_key(|m| m.mid);
    messages
}

/// Process a single message and return a MessageEvent if it should be included
fn process_message(
    mid: i64,
    msg_bytes: &[u8],
    bot_uid: i64,
    cache: &tokio::sync::RwLockReadGuard<'_, Cache>,
    now_millis: i64,
    max_age_seconds: i64,
    matrix_domain: &str,
) -> Option<MessageEvent> {
    let payload = serde_json::from_slice::<ChatMessagePayload>(msg_bytes).ok()?;

    // Check if this message is sent TO the bot (not BY the bot)
    let is_message_to_bot = match &payload.target {
        // DM to bot: target user is the bot
        MessageTarget::User(target_user) => {
            target_user.uid == bot_uid && payload.from_uid != bot_uid
        }
        // Group message: bot is in the group and message is not from bot
        MessageTarget::Group(group) => {
            let bot_in_group: bool = cache
                .groups
                .get(&group.gid)
                .map(|g: &CacheGroup| g.contains_user(bot_uid))
                .unwrap_or(false);
            payload.from_uid != bot_uid && bot_in_group
        }
    };

    tracing::trace!(
        "Message {}: from_uid={}, target={:?}, is_message_to_bot={}",
        mid,
        payload.from_uid,
        payload.target,
        is_message_to_bot
    );

    if !is_message_to_bot {
        return None;
    }

    // Filter by age
    let message_age = now_millis - payload.created_at.timestamp_millis();
    tracing::debug!(
        "Message {}: message_age={}ms, max_age={}ms",
        mid,
        message_age,
        max_age_seconds * 1000
    );

    if message_age > (max_age_seconds * 1000) {
        tracing::debug!("Message {} filtered by age", mid);
        return None;
    }

    let content = extract_message_content(&payload, cache, matrix_domain)?;
    let sender_name = cache
        .users
        .get(&payload.from_uid)
        .map(|u| u.name.as_str())
        .unwrap_or("unknown");

    let room_id = get_room_id(&payload, matrix_domain);

    tracing::debug!("Adding message {} from {}", mid, sender_name);

    Some(MessageEvent {
        mid,
        from_uid: payload.from_uid,
        room_id,
        sender: format!("@{}:{}", sender_name, matrix_domain),
        content,
        timestamp: payload.created_at.timestamp_millis(),
    })
}

/// Extract message content from ChatMessagePayload
fn extract_message_content(
    payload: &ChatMessagePayload,
    cache: &tokio::sync::RwLockReadGuard<'_, Cache>,
    matrix_domain: &str,
) -> Option<Value> {
    let (content, properties) = match &payload.detail {
        MessageDetail::Normal(ref d) => (&d.content, &d.content.properties),
        MessageDetail::Reply(ref d) => (&d.content, &d.content.properties),
        _ => return None,
    };

    // Extract mentions from properties
    let mention_uids: Vec<i64> = properties
        .as_ref()
        .and_then(|p| p.get("mentions"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_default();

    // Build mention info: uid -> (matrix_id, display_name)
    let mention_info: Vec<(i64, String, String)> = mention_uids
        .iter()
        .filter_map(|uid| {
            cache.users.get(uid).map(|user| {
                let matrix_id = format!("@{}:{}", user.name, matrix_domain);
                (*uid, matrix_id, user.name.clone())
            })
        })
        .collect();

    // Replace @uid with @displayName in body text
    let mut body = content.content.clone();
    for (uid, _, display_name) in &mention_info {
        body = body.replace(&format!("@{}", uid), &format!("@{}", display_name));
    }

    // Build m.mentions user_ids list
    let mention_user_ids: Vec<String> = mention_info
        .iter()
        .map(|(_, matrix_id, _)| matrix_id.clone())
        .collect();

    // Build formatted_body with HTML mention links for mentioned users
    let formatted_body = if !mention_info.is_empty() {
        let mut fb = content.content.clone();
        for (uid, matrix_id, display_name) in &mention_info {
            fb = fb.replace(
                &format!("@{}", uid),
                &format!(
                    "<a href=\"https://matrix.to/#/{}\">{}</a>",
                    matrix_id, display_name
                ),
            );
        }
        Some(fb)
    } else {
        None
    };

    match content.content_type.as_str() {
        "text/plain" => {
            let mut content_json = json!({
                "msgtype": "m.text",
                "body": body
            });
            if !mention_user_ids.is_empty() {
                content_json
                    .as_object_mut()
                    .unwrap()
                    .insert("m.mentions".to_string(), json!({ "user_ids": mention_user_ids }));
            }
            if let Some(fb) = formatted_body {
                content_json.as_object_mut().unwrap().insert(
                    "format".to_string(),
                    json!("org.matrix.custom.html"),
                );
                content_json
                    .as_object_mut()
                    .unwrap()
                    .insert("formatted_body".to_string(), json!(fb));
            }
            Some(content_json)
        }
        "text/markdown" => {
            let mut content_json = json!({
                "msgtype": "m.text",
                "body": body,
                "format": "org.matrix.custom.html",
                "formatted_body": formatted_body.unwrap_or(content.content.clone())
            });
            if !mention_user_ids.is_empty() {
                content_json
                    .as_object_mut()
                    .unwrap()
                    .insert("m.mentions".to_string(), json!({ "user_ids": mention_user_ids }));
            }
            Some(content_json)
        }
        "vachat/file" | "vachat/archive" => {
            // Use mxc:// URL format instead of HTTPS
            // The file_path is used as the media_id in the mxc URL
            let file_path = &content.content;
            // Replace '/' with '_' to create a valid media_id (mxc URLs don't allow '/' in media_id)
            let media_id = file_path.replace('/', "_");
            let mxc_url = format!("mxc://{}/{}", matrix_domain, media_id);

            let mime_type = content.properties
                .as_ref()
                .and_then(|p| p.get("content_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("application/octet-stream");

            let filename = content.properties
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or(file_path);

            let msgtype = if mime_type.starts_with("image/") {
                "m.image"
            } else {
                "m.file"
            };

            Some(json!({
                "msgtype": msgtype,
                "body": filename,
                "url": mxc_url,
                "info": {
                    "mimetype": mime_type,
                    "size": content.properties
                        .as_ref()
                        .and_then(|p| p.get("size"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                }
            }))
        }
        _ => Some(json!({
            "msgtype": "m.text",
            "body": body
        })),
    }
}

/// Get room id from message payload
fn get_room_id(payload: &ChatMessagePayload, matrix_domain: &str) -> String {
    match &payload.target {
        MessageTarget::User(target_user) => {
            // For DM, create room id based on the two users
            let sender_uid = payload.from_uid;
            let bot_uid = target_user.uid;
            format!("!dm_{}_{}:{}", sender_uid, bot_uid, matrix_domain)
        }
        MessageTarget::Group(group) => {
            format!("!group_{}:{}", group.gid, matrix_domain)
        }
    }
}

/// Build joined rooms map for a bot user from room_cache
fn build_joined_rooms(uid: i64, room_cache: &HashMap<i64, Vec<String>>) -> HashMap<String, Value> {
    let mut rooms_join: HashMap<String, Value> = HashMap::new();

    // Get rooms for this specific bot user from room_cache
    if let Some(rooms) = room_cache.get(&uid) {
        for room_id in rooms {
            rooms_join.insert(
                room_id.clone(),
                json!({
                    "timeline": { "events": [], "limited": false },
                    "state": { "events": [] }
                }),
            );
        }
    }

    rooms_join
}

/// Create empty Matrix sync response (with joined rooms)
fn empty_sync_response(
    next_batch: i64,
    rooms_join: HashMap<String, Value>,
    changed_users: Vec<String>,
    otk_count: i64,
) -> Value {
    json!({
        "next_batch": format!("s{}", next_batch),
        "rooms": {
            "join": rooms_join
        },
        "device_lists": {
            "changed": changed_users,
            "left": []
        },
        "device_one_time_keys_count": {
            "signed_curve25519": otk_count
        }
    })
}

/// Build Matrix sync response
/// Detects consecutive media + text messages from the same sender in the same room,
/// and adds m.relates_to to link text messages to their corresponding media messages.
fn build_sync_response(
    messages: Vec<MessageEvent>,
    next_batch: i64,
    changed_users: Vec<String>,
    otk_count: i64,
) -> Value {
    let mut rooms_join: HashMap<String, Value> = HashMap::new();

    // Track the last media message event_id for each (room_id, sender) pair
    // Value is (media_mid, media_timestamp)
    let mut last_media_event: HashMap<(String, String), (i64, i64)> = HashMap::new();

    for msg in &messages {
        let room_id = msg.room_id.clone();
        let sender = msg.sender.clone();
        let key = (room_id.clone(), sender.clone());

        // Check if this is a media message (has "url" field in content)
        let is_media = msg.content.get("url").is_some();

        if is_media {
            // Update the last media event for this (room, sender) pair
            last_media_event.insert(key.clone(), (msg.mid, msg.timestamp));
        }

        // Check if this text message should be linked to a preceding media message
        // Conditions:
        // 1. This is a text message (no "url" field)
        // 2. There was a media message from the same sender in the same room
        // 3. The time difference is within 60 seconds (likely sent together)
        let relates_to = if !is_media {
            if let Some((media_mid, media_ts)) = last_media_event.get(&key) {
                let time_diff = (msg.timestamp - media_ts).abs();
                // Only link if within 60 seconds
                if time_diff < 60000 {
                    Some(json!({
                        "rel_type": "m.reference",
                        "event_id": format!("${}", media_mid)
                    }))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Build the event content, adding relates_to if present
        let content = if let Some(rel) = relates_to {
            let mut content = msg.content.clone();
            if let Some(obj) = content.as_object_mut() {
                obj.insert("m.relates_to".to_string(), rel);
            }
            content
        } else {
            msg.content.clone()
        };

        let event = json!({
            "content": content,
            "event_id": format!("${}", msg.mid),
            "origin_server_ts": msg.timestamp,
            "sender": msg.sender,
            "type": "m.room.message",
            "unsigned": { "age": 0 }
        });

        rooms_join
            .entry(room_id)
            .and_modify(|room: &mut Value| {
                if let Some(events) = room["timeline"]["events"].as_array_mut() {
                    events.push(event.clone());
                }
            })
            .or_insert_with(|| {
                json!({
                    "timeline": {
                        "events": vec![event],
                        "limited": false,
                        "prev_batch": format!("s{}", next_batch)
                    },
                    "state": { "events": [] }
                })
            });
    }

    json!({
        "next_batch": format!("s{}", next_batch),
        "rooms": {
            "join": rooms_join
        },
        "device_lists": {
            "changed": changed_users,
            "left": []
        },
        "device_one_time_keys_count": {
            "signed_curve25519": otk_count
        }
    })
}
