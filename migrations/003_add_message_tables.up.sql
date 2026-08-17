-- Message tables replacing the sled-based msgdb.
-- `message` stores the opaque serialized message body once; `user_message` is a
-- per-recipient inbox index (snapshot of recipients at send time);
-- `merged_message` stores aggregated message payloads (reactions).

CREATE TABLE "message" (
    "mid" INTEGER PRIMARY KEY AUTOINCREMENT,
    "kind" INTEGER NOT NULL DEFAULT 0, -- 0 = dm, 1 = group
    "from_uid" INTEGER NOT NULL,
    "to_uid" INTEGER,                  -- valid when kind = 0
    "gid" INTEGER,                     -- valid when kind = 1
    "content" BLOB NOT NULL,
    "created_at" DATETIME NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX "idx_message_dm" ON "message" ("from_uid", "to_uid", "mid");
CREATE INDEX "idx_message_group" ON "message" ("gid", "mid");

CREATE TABLE "user_message" (
    "uid" INTEGER NOT NULL,
    "mid" INTEGER NOT NULL,
    PRIMARY KEY ("uid", "mid")
);

CREATE INDEX "idx_user_message_mid" ON "user_message" ("mid");

CREATE TABLE "merged_message" (
    "mid" INTEGER PRIMARY KEY,
    "content" BLOB NOT NULL
);
