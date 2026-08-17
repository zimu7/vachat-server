//! SQLite-backed message storage.
//!
//! Replaces the sled-based `rc-msgdb` crate. Message bodies are opaque
//! serialized bytes (`BLOB`) and are stored once in `message`, with a
//! per-recipient inbox index in `user_message` and aggregated payloads
//! (reactions) in `merged_message`.

use sqlx::SqlitePool;

/// `message.kind` values
const KIND_DM: i32 = 0;
const KIND_GROUP: i32 = 1;

/// Get the raw message body by message ID.
pub async fn get(pool: &SqlitePool, mid: i64) -> sqlx::Result<Option<Vec<u8>>> {
    sqlx::query_scalar::<_, Vec<u8>>("SELECT content FROM message WHERE mid = ?")
        .bind(mid)
        .fetch_optional(pool)
        .await
}

/// Send a message to a group. Inserts the message body and the recipient
/// snapshot (`to`) in a single transaction; returns the new message ID.
pub async fn send_to_group(
    pool: &SqlitePool,
    gid: i64,
    from_uid: i64,
    to: impl IntoIterator<Item = i64>,
    msg: &[u8],
) -> sqlx::Result<i64> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query("INSERT INTO message (kind, from_uid, gid, content) VALUES (?, ?, ?, ?)")
        .bind(KIND_GROUP)
        .bind(from_uid)
        .bind(gid)
        .bind(msg)
        .execute(&mut *tx)
        .await?;
    let mid = res.last_insert_rowid();
    for uid in to {
        sqlx::query("INSERT OR IGNORE INTO user_message (uid, mid) VALUES (?, ?)")
            .bind(uid)
            .bind(mid)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(mid)
}

/// Send a direct message between two users. Inserts the message body and
/// inbox entries for both users in a single transaction; returns the new
/// message ID.
pub async fn send_to_dm(
    pool: &SqlitePool,
    from_uid: i64,
    to_uid: i64,
    msg: &[u8],
) -> sqlx::Result<i64> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query("INSERT INTO message (kind, from_uid, to_uid, content) VALUES (?, ?, ?, ?)")
        .bind(KIND_DM)
        .bind(from_uid)
        .bind(to_uid)
        .bind(msg)
        .execute(&mut *tx)
        .await?;
    let mid = res.last_insert_rowid();
    for uid in [from_uid, to_uid] {
        sqlx::query("INSERT OR IGNORE INTO user_message (uid, mid) VALUES (?, ?)")
            .bind(uid)
            .bind(mid)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(mid)
}

/// Fetch the newest `limit` messages in the user's inbox with `mid > after`,
/// returned in ascending order. `after = None` is equivalent to `after = 0`.
pub async fn fetch_user_messages_after(
    pool: &SqlitePool,
    uid: i64,
    after: Option<i64>,
    limit: usize,
) -> sqlx::Result<Vec<(i64, Vec<u8>)>> {
    let mut msgs = sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT m.mid, m.content FROM user_message um \
         JOIN message m ON m.mid = um.mid \
         WHERE um.uid = ? AND um.mid > ? \
         ORDER BY um.mid DESC LIMIT ?",
    )
    .bind(uid)
    .bind(after.unwrap_or(-1))
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    msgs.reverse();
    Ok(msgs)
}

/// Fetch all messages with `mid > since_mid`, returned in ascending order.
pub async fn fetch_messages_after(
    pool: &SqlitePool,
    since_mid: i64,
    limit: usize,
) -> sqlx::Result<Vec<(i64, Vec<u8>)>> {
    sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT mid, content FROM message WHERE mid > ? ORDER BY mid ASC LIMIT ?",
    )
    .bind(since_mid)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
}

/// Fetch DM messages with `mid < before` (newest `limit` first, then reversed
/// to ascending order). `before = None` means no upper bound.
pub async fn fetch_dm_messages_before(
    pool: &SqlitePool,
    from_uid: i64,
    to_uid: i64,
    before: Option<i64>,
    limit: usize,
) -> sqlx::Result<Vec<(i64, Vec<u8>)>> {
    let mut msgs = sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT mid, content FROM message \
         WHERE kind = ? AND ((from_uid = ? AND to_uid = ?) OR (from_uid = ? AND to_uid = ?)) \
         AND mid < ? \
         ORDER BY mid DESC LIMIT ?",
    )
    .bind(KIND_DM)
    .bind(from_uid)
    .bind(to_uid)
    .bind(to_uid)
    .bind(from_uid)
    .bind(before.unwrap_or(i64::MAX))
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    msgs.reverse();
    Ok(msgs)
}

/// Fetch group messages with `mid < before` (newest `limit` first, then
/// reversed to ascending order). `before = None` means no upper bound.
pub async fn fetch_group_messages_before(
    pool: &SqlitePool,
    gid: i64,
    before: Option<i64>,
    limit: usize,
) -> sqlx::Result<Vec<(i64, Vec<u8>)>> {
    let mut msgs = sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT mid, content FROM message \
         WHERE kind = ? AND gid = ? AND mid < ? \
         ORDER BY mid DESC LIMIT ?",
    )
    .bind(KIND_GROUP)
    .bind(gid)
    .bind(before.unwrap_or(i64::MAX))
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    msgs.reverse();
    Ok(msgs)
}

/// Fetch all messages with `mid <= before` in descending order (newest first).
pub async fn fetch_messages_before_rev(
    pool: &SqlitePool,
    before: i64,
    limit: usize,
) -> sqlx::Result<Vec<(i64, Vec<u8>)>> {
    sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT mid, content FROM message WHERE mid <= ? ORDER BY mid DESC LIMIT ?",
    )
    .bind(before)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
}

/// Get the maximum message ID in the database.
pub async fn get_max_msg_id(pool: &SqlitePool) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(mid) FROM message")
        .fetch_one(pool)
        .await
}

/// Insert (or replace) the merged message payload for a message.
pub async fn insert_merged_msg(pool: &SqlitePool, mid: i64, msg: &[u8]) -> sqlx::Result<()> {
    sqlx::query("INSERT OR REPLACE INTO merged_message (mid, content) VALUES (?, ?)")
        .bind(mid)
        .bind(msg)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get the merged message payload for a message.
pub async fn get_merged_msg(pool: &SqlitePool, mid: i64) -> sqlx::Result<Option<Vec<u8>>> {
    sqlx::query_scalar::<_, Vec<u8>>("SELECT content FROM merged_message WHERE mid = ?")
        .bind(mid)
        .fetch_optional(pool)
        .await
}

/// Remove the merged message payload for a message. A no-op if no merged
/// message exists. After this, `get_merged_msg` returns `None`, which callers
/// interpret as "message deleted".
pub async fn remove_merged_msg(pool: &SqlitePool, mid: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM merged_message WHERE mid = ?")
        .bind(mid)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update the merged message payload with a read-modify-write inside a
/// transaction. If no merged message exists for `mid`, this is a no-op.
pub async fn update_merged_msg(
    pool: &SqlitePool,
    mid: i64,
    f: impl FnOnce(&[u8]) -> Vec<u8>,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    let existing = sqlx::query_scalar::<_, Vec<u8>>("SELECT content FROM merged_message WHERE mid = ?")
        .bind(mid)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(data) = existing {
        sqlx::query("UPDATE merged_message SET content = ? WHERE mid = ?")
            .bind(f(&data))
            .bind(mid)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Clear all messages for a group. Removes the message bodies, the inbox
/// indexes and the merged messages in a single transaction; returns the list
/// of message IDs that were removed.
pub async fn clear_group_messages(pool: &SqlitePool, gid: i64) -> sqlx::Result<Vec<i64>> {
    let mut tx = pool.begin().await?;
    let mids: Vec<i64> =
        sqlx::query_scalar("SELECT mid FROM message WHERE kind = ? AND gid = ? ORDER BY mid ASC")
            .bind(KIND_GROUP)
            .bind(gid)
            .fetch_all(&mut *tx)
            .await?;

    sqlx::query(
        "DELETE FROM user_message WHERE mid IN \
         (SELECT mid FROM message WHERE kind = ? AND gid = ?)",
    )
    .bind(KIND_GROUP)
    .bind(gid)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM merged_message WHERE mid IN \
         (SELECT mid FROM message WHERE kind = ? AND gid = ?)",
    )
    .bind(KIND_GROUP)
    .bind(gid)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM message WHERE kind = ? AND gid = ?")
        .bind(KIND_GROUP)
        .bind(gid)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(mids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::migrate::MigrateDatabase;

    /// Create an in-memory temp-dir SQLite pool with all migrations applied.
    async fn test_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::TempDir::new().unwrap();
        let dsn = format!("sqlite:{}", dir.path().join("db.sqlite").display());
        sqlx::Sqlite::create_database(&dsn).await.unwrap();
        let pool = SqlitePool::connect(&dsn).await.unwrap();
        crate::server::MIGRATOR.run(&pool).await.unwrap();
        (dir, pool)
    }

    fn msg(content: &str) -> Vec<u8> {
        content.as_bytes().to_vec()
    }

    fn mids(rows: &[(i64, Vec<u8>)]) -> Vec<i64> {
        rows.iter().map(|(mid, _)| *mid).collect()
    }

    #[tokio::test]
    async fn test_send_and_fetch_user_messages() {
        let (_dir, pool) = test_pool().await;

        // recipients snapshot: uid 1 sends to group 10 with members [2, 3]
        let mid1 = send_to_group(&pool, 10, 1, [2, 3], &msg("hello")).await.unwrap();
        let mid2 = send_to_group(&pool, 10, 1, [2, 3], &msg("world")).await.unwrap();
        // uid 4 joins later: must NOT see the messages (snapshot semantics)
        let mid3 = send_to_group(&pool, 10, 1, [2, 3, 4], &msg("joined")).await.unwrap();

        // ascending order, all three for uid 2
        let rows = fetch_user_messages_after(&pool, 2, None, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid1, mid2, mid3]);

        // after cursor: strictly greater than mid1
        let rows = fetch_user_messages_after(&pool, 2, Some(mid1), 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid2, mid3]);

        // uid 4 only sees the message sent after joining
        let rows = fetch_user_messages_after(&pool, 4, None, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid3]);

        // limit
        let rows = fetch_user_messages_after(&pool, 2, None, 2).await.unwrap();
        assert_eq!(mids(&rows), vec![mid2, mid3]);
    }

    #[tokio::test]
    async fn test_send_and_fetch_dm_messages() {
        let (_dir, pool) = test_pool().await;

        let mid1 = send_to_dm(&pool, 1, 2, &msg("a")).await.unwrap();
        let mid2 = send_to_dm(&pool, 2, 1, &msg("b")).await.unwrap();
        let mid3 = send_to_dm(&pool, 1, 2, &msg("c")).await.unwrap();

        // works in both uid orders, ascending, no upper bound
        let rows = fetch_dm_messages_before(&pool, 1, 2, None, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid1, mid2, mid3]);
        let rows = fetch_dm_messages_before(&pool, 2, 1, None, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid1, mid2, mid3]);

        // before bound is exclusive (mid < before)
        let rows = fetch_dm_messages_before(&pool, 1, 2, Some(mid3), 100)
            .await
            .unwrap();
        assert_eq!(mids(&rows), vec![mid1, mid2]);

        // limit takes the newest first
        let rows = fetch_dm_messages_before(&pool, 1, 2, None, 2).await.unwrap();
        assert_eq!(mids(&rows), vec![mid2, mid3]);
    }

    #[tokio::test]
    async fn test_self_dm_dedup() {
        let (_dir, pool) = test_pool().await;

        send_to_dm(&pool, 1, 1, &msg("self")).await.unwrap();
        let rows = fetch_user_messages_after(&pool, 1, None, 100).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn test_fetch_messages_after_and_before_rev() {
        let (_dir, pool) = test_pool().await;

        let mid1 = send_to_dm(&pool, 1, 2, &msg("a")).await.unwrap();
        let mid2 = send_to_dm(&pool, 1, 2, &msg("b")).await.unwrap();
        let mid3 = send_to_dm(&pool, 1, 2, &msg("c")).await.unwrap();

        // ascending, strictly greater than since
        let rows = fetch_messages_after(&pool, mid1, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid2, mid3]);

        // descending, inclusive of before
        let rows = fetch_messages_before_rev(&pool, mid3, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid3, mid2, mid1]);
        let rows = fetch_messages_before_rev(&pool, mid2, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid2, mid1]);
    }

    #[tokio::test]
    async fn test_get_and_max_mid() {
        let (_dir, pool) = test_pool().await;

        assert_eq!(get_max_msg_id(&pool).await.unwrap(), None);

        let mid1 = send_to_dm(&pool, 1, 2, &msg("a")).await.unwrap();
        let mid2 = send_to_dm(&pool, 1, 2, &msg("b")).await.unwrap();
        assert_eq!(get_max_msg_id(&pool).await.unwrap(), Some(mid2));
        assert_eq!(get(&pool, mid1).await.unwrap().unwrap(), msg("a"));
        assert!(get(&pool, mid2 + 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_group_messages_before() {
        let (_dir, pool) = test_pool().await;

        let mid1 = send_to_group(&pool, 10, 1, [1], &msg("a")).await.unwrap();
        let mid2 = send_to_group(&pool, 10, 1, [1], &msg("b")).await.unwrap();
        // message in another group must not leak
        let mid3 = send_to_group(&pool, 20, 1, [1], &msg("x")).await.unwrap();

        let rows = fetch_group_messages_before(&pool, 10, None, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid1, mid2]);
        let rows = fetch_group_messages_before(&pool, 10, Some(mid2), 100)
            .await
            .unwrap();
        assert_eq!(mids(&rows), vec![mid1]);
        assert!(mid3 > mid2);
    }

    #[tokio::test]
    async fn test_merged_msg_ops() {
        let (_dir, pool) = test_pool().await;

        insert_merged_msg(&pool, 1, &msg("v1")).await.unwrap();
        assert_eq!(get_merged_msg(&pool, 1).await.unwrap().unwrap(), msg("v1"));

        update_merged_msg(&pool, 1, |data| {
            let mut data = data.to_vec();
            data.extend_from_slice(b"-v2");
            data
        })
        .await
        .unwrap();
        assert_eq!(get_merged_msg(&pool, 1).await.unwrap().unwrap(), msg("v1-v2"));

        // other mids unaffected
        insert_merged_msg(&pool, 2, &msg("other")).await.unwrap();
        update_merged_msg(&pool, 1, |_| msg("v3")).await.unwrap();
        assert_eq!(get_merged_msg(&pool, 2).await.unwrap().unwrap(), msg("other"));

        // updating a missing mid is a no-op
        update_merged_msg(&pool, 99, |_| msg("nope")).await.unwrap();
        assert!(get_merged_msg(&pool, 99).await.unwrap().is_none());

        // removing an existing mid makes it read back as deleted
        remove_merged_msg(&pool, 1).await.unwrap();
        assert!(get_merged_msg(&pool, 1).await.unwrap().is_none());
        // other mids unaffected, removing a missing mid is a no-op
        assert_eq!(get_merged_msg(&pool, 2).await.unwrap().unwrap(), msg("other"));
        remove_merged_msg(&pool, 99).await.unwrap();
    }

    #[tokio::test]
    async fn test_clear_group_messages() {
        let (_dir, pool) = test_pool().await;

        let gid_a = 10;
        let mid1 = send_to_group(&pool, gid_a, 1, [2], &msg("a")).await.unwrap();
        let mid2 = send_to_group(&pool, gid_a, 1, [2], &msg("b")).await.unwrap();
        insert_merged_msg(&pool, mid1, &msg("merged-a")).await.unwrap();
        let mid_dm = send_to_dm(&pool, 1, 2, &msg("dm")).await.unwrap();
        // other group untouched
        let mid_other = send_to_group(&pool, 20, 1, [2], &msg("x")).await.unwrap();

        let removed = clear_group_messages(&pool, gid_a).await.unwrap();
        assert_eq!(removed, vec![mid1, mid2]);

        // group conversation is empty
        assert!(fetch_group_messages_before(&pool, gid_a, None, 100)
            .await
            .unwrap()
            .is_empty());
        // merged messages of removed mids are gone
        assert!(get_merged_msg(&pool, mid1).await.unwrap().is_none());
        // user inbox keeps dm + other group messages only
        let rows = fetch_user_messages_after(&pool, 2, None, 100).await.unwrap();
        assert_eq!(mids(&rows), vec![mid_dm, mid_other]);

        // AUTOINCREMENT: new mids stay above deleted ones
        let new_mid = send_to_group(&pool, gid_a, 1, [2], &msg("new")).await.unwrap();
        assert!(new_mid > mid2);
        assert!(new_mid > mid_other);
    }
}
