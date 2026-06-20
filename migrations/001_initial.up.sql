PRAGMA foreign_keys = false;

-- ----------------------------
-- Table structure for _sqlx_migrations
-- ----------------------------
DROP TABLE IF EXISTS "_sqlx_migrations";
CREATE TABLE "_sqlx_migrations" (
  "version" BIGINT,
  "description" TEXT NOT NULL,
  "installed_on" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "success" BOOLEAN NOT NULL,
  "checksum" BLOB NOT NULL,
  "execution_time" BIGINT NOT NULL,
  PRIMARY KEY ("version")
);

-- ----------------------------
-- Table structure for bot_command_list
-- ----------------------------
DROP TABLE IF EXISTS "bot_command_list";
CREATE TABLE "bot_command_list" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "command" text NOT NULL,
  "description" text,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for bot_key
-- ----------------------------
DROP TABLE IF EXISTS "bot_key";
CREATE TABLE "bot_key" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "name" TEXT NOT NULL,
  "key" TEXT NOT NULL,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "last_used" timestamp,
  "password" TEXT,
  "updated_at" timestamp,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for burn_after_reading
-- ----------------------------
DROP TABLE IF EXISTS "burn_after_reading";
CREATE TABLE "burn_after_reading" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "target_uid" integer,
  "target_gid" integer,
  "expires_in" integer NOT NULL,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("target_uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("target_gid") REFERENCES "group" ("gid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for config
-- ----------------------------
DROP TABLE IF EXISTS "config";
CREATE TABLE "config" (
  "name" text,
  "enabled" bool NOT NULL DEFAULT false,
  "value" text NOT NULL,
  PRIMARY KEY ("name")
);

-- ----------------------------
-- Table structure for contacts
-- ----------------------------
DROP TABLE IF EXISTS "contacts";
CREATE TABLE "contacts" (
  "uid" integer NOT NULL,
  "target_uid" integer NOT NULL,
  "status" integer NOT NULL,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  PRIMARY KEY ("uid", "target_uid"),
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("target_uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for device
-- ----------------------------
DROP TABLE IF EXISTS "device";
CREATE TABLE "device" (
  "uid" integer NOT NULL,
  "device" text NOT NULL,
  "device_token" text,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  PRIMARY KEY ("uid", "device"),
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for favorite_archive
-- ----------------------------
DROP TABLE IF EXISTS "favorite_archive";
CREATE TABLE "favorite_archive" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "archive_id" text NOT NULL,
  "created_at" timestamp NOT NULL,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for files
-- ----------------------------
DROP TABLE IF EXISTS "files";
CREATE TABLE "files" (
  "mid" integer NOT NULL,
  "from_uid" integer NOT NULL,
  "gid" integer NOT NULL,
  "ext" varchar NOT NULL,
  "content_type" varchar NOT NULL,
  "content" varchar NOT NULL,
  "properties" text NOT NULL,
  "created_at" timestamp NOT NULL,
  "expired" boolean NOT NULL,
  "confirmed" boolean NOT NULL,
  PRIMARY KEY ("mid")
);

-- ----------------------------
-- Table structure for group
-- ----------------------------
DROP TABLE IF EXISTS "group";
CREATE TABLE "group" (
  "gid" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" text NOT NULL,
  "owner" integer,
  "is_public" bool NOT NULL DEFAULT false,
  "description" text NOT NULL DEFAULT '',
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  "avatar_updated_at" timestamp NOT NULL DEFAULT '1970-01-01 00:00:00',
  "show_email" bool NOT NULL DEFAULT true,
  "dm_to_member" bool NOT NULL DEFAULT true,
  "add_friend" bool NOT NULL DEFAULT true,
  "only_owner_can_send_msg" bool NOT NULL DEFAULT false,
  "ext_setting" text
);

-- ----------------------------
-- Table structure for group_user
-- ----------------------------
DROP TABLE IF EXISTS "group_user";
CREATE TABLE "group_user" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "gid" integer NOT NULL,
  "uid" integer NOT NULL,
  FOREIGN KEY ("gid") REFERENCES "group" ("gid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for matrix_device_keys
-- ----------------------------
DROP TABLE IF EXISTS "matrix_device_keys";
CREATE TABLE "matrix_device_keys" (
  "uid" INTEGER NOT NULL,
  "device_id" TEXT NOT NULL,
  "curve25519_key" TEXT NOT NULL,
  "ed25519_key" TEXT NOT NULL,
  "keys_json" TEXT NOT NULL,
  "created_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("uid", "device_id"),
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for matrix_device_otk
-- ----------------------------
DROP TABLE IF EXISTS "matrix_device_otk";
CREATE TABLE "matrix_device_otk" (
  "uid" INTEGER NOT NULL,
  "device_id" TEXT NOT NULL,
  "key_id" TEXT NOT NULL,
  "curve25519_key" TEXT NOT NULL,
  "signature" TEXT,
  "used" BOOLEAN DEFAULT FALSE,
  "created_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("uid", "device_id", "key_id"),
  FOREIGN KEY ("uid", "device_id") REFERENCES "matrix_device_keys" ("uid", "device_id") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for matrix_megolm_inbound_session
-- ----------------------------
DROP TABLE IF EXISTS "matrix_megolm_inbound_session";
CREATE TABLE "matrix_megolm_inbound_session" (
  "session_id" TEXT NOT NULL,
  "room_id" TEXT NOT NULL,
  "sender_uid" INTEGER NOT NULL,
  "sender_device_id" TEXT NOT NULL,
  "sender_curve25519_key" TEXT NOT NULL,
  "session_data" BLOB NOT NULL,
  "created_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "last_used_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("session_id"),
  FOREIGN KEY ("sender_uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for matrix_olm_account
-- ----------------------------
DROP TABLE IF EXISTS "matrix_olm_account";
CREATE TABLE "matrix_olm_account" (
  "uid" INTEGER NOT NULL,
  "device_id" TEXT NOT NULL,
  "account_data" BLOB NOT NULL,
  "created_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("uid", "device_id"),
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for matrix_olm_inbound_session
-- ----------------------------
DROP TABLE IF EXISTS "matrix_olm_inbound_session";
CREATE TABLE "matrix_olm_inbound_session" (
  "session_id" TEXT NOT NULL,
  "local_uid" INTEGER NOT NULL,
  "local_device_id" TEXT NOT NULL,
  "sender_uid" INTEGER NOT NULL,
  "sender_device_id" TEXT NOT NULL,
  "sender_curve25519_key" TEXT NOT NULL,
  "session_data" BLOB NOT NULL,
  "created_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "last_used_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("session_id"),
  FOREIGN KEY ("local_uid", "local_device_id") REFERENCES "matrix_device_keys" ("uid", "device_id") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for matrix_olm_outbound_session
-- ----------------------------
DROP TABLE IF EXISTS "matrix_olm_outbound_session";
CREATE TABLE "matrix_olm_outbound_session" (
  "session_id" TEXT NOT NULL,
  "sender_uid" INTEGER NOT NULL,
  "sender_device_id" TEXT NOT NULL,
  "recipient_uid" INTEGER NOT NULL,
  "recipient_device_id" TEXT NOT NULL,
  "recipient_curve25519_key" TEXT NOT NULL,
  "session_data" BLOB NOT NULL,
  "created_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "last_used_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("session_id"),
  FOREIGN KEY ("sender_uid", "sender_device_id") REFERENCES "matrix_device_keys" ("uid", "device_id") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("recipient_uid", "recipient_device_id") REFERENCES "matrix_device_keys" ("uid", "device_id") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for matrix_room_encryption
-- ----------------------------
DROP TABLE IF EXISTS "matrix_room_encryption";
CREATE TABLE "matrix_room_encryption" (
  "room_id" TEXT NOT NULL,
  "algorithm" TEXT NOT NULL,
  "rotation_period_msgs" INTEGER DEFAULT 100,
  "rotation_period_ms" INTEGER DEFAULT 604800000,
  "created_at" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("room_id")
);

-- ----------------------------
-- Table structure for mute
-- ----------------------------
DROP TABLE IF EXISTS "mute";
CREATE TABLE "mute" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "mute_uid" integer,
  "mute_gid" integer,
  "expired_at" timestamp,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("mute_uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("mute_gid") REFERENCES "group" ("gid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for openid_connect
-- ----------------------------
DROP TABLE IF EXISTS "openid_connect";
CREATE TABLE "openid_connect" (
  "issuer" text NOT NULL,
  "subject" text NOT NULL,
  "uid" integer NOT NULL,
  PRIMARY KEY ("issuer", "subject"),
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for pinned_chat
-- ----------------------------
DROP TABLE IF EXISTS "pinned_chat";
CREATE TABLE "pinned_chat" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "target_uid" integer,
  "target_gid" integer,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("target_uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("target_gid") REFERENCES "group" ("gid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for pinned_message
-- ----------------------------
DROP TABLE IF EXISTS "pinned_message";
CREATE TABLE "pinned_message" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "gid" integer NOT NULL,
  "mid" integer NOT NULL,
  "created_by" integer NOT NULL,
  "created_at" timestamp NOT NULL,
  "msg_created_by" INTEGER,
  "msg_created_at" TIMESTAMP,
  FOREIGN KEY ("created_by") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("gid") REFERENCES "group" ("gid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for read_index
-- ----------------------------
DROP TABLE IF EXISTS "read_index";
CREATE TABLE "read_index" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "target_uid" integer,
  "target_gid" integer,
  "mid" integer NOT NULL,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("target_uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("target_gid") REFERENCES "group" ("gid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for refresh_token
-- ----------------------------
DROP TABLE IF EXISTS "refresh_token";
CREATE TABLE "refresh_token" (
  "uid" integer NOT NULL,
  "device" text NOT NULL,
  "token" text NOT NULL,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  PRIMARY KEY ("uid", "device"),
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- sqlite_sequence is an internal system table managed by SQLite.
-- It is automatically created when AUTOINCREMENT is used.
-- Only UPDATE the seq values; do not DROP or CREATE it.
-- ----------------------------

-- ----------------------------
-- Table structure for third_party_users
-- ----------------------------
DROP TABLE IF EXISTS "third_party_users";
CREATE TABLE "third_party_users" (
  "userid" text NOT NULL,
  "uid" integer NOT NULL,
  PRIMARY KEY ("userid"),
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for user
-- ----------------------------
DROP TABLE IF EXISTS "user";
CREATE TABLE "user" (
  "uid" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "name" text NOT NULL COLLATE nocase,
  "password" text,
  "email" text COLLATE nocase,
  "gender" integer NOT NULL,
  "language" text NOT NULL,
  "is_admin" boolean NOT NULL DEFAULT false,
  "create_by" text NOT NULL,
  "avatar_updated_at" timestamp NOT NULL DEFAULT '1970-01-01 00:00:00',
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  "status" integer NOT NULL DEFAULT 0,
  "is_guest" boolean NOT NULL DEFAULT false,
  "webhook_url" string,
  "is_bot" boolean NOT NULL DEFAULT false,
  "birthday" timestamp,
  "bot_secret" text,
  "widget_id" text,
  "msg_smtp_notify_enable" boolean NOT NULL DEFAULT false
);

-- ----------------------------
-- Table structure for user_log
-- ----------------------------
DROP TABLE IF EXISTS "user_log";
CREATE TABLE "user_log" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "action" integer NOT NULL,
  "email" text,
  "name" text,
  "gender" integer,
  "language" text,
  "is_admin" bool,
  "avatar_updated_at" timestamp,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "is_bot" boolean,
  "birthday" timestamp
);

-- ----------------------------
-- Table structure for user_remark
-- ----------------------------
DROP TABLE IF EXISTS "user_remark";
CREATE TABLE "user_remark" (
  "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
  "uid" integer NOT NULL,
  "contact_uid" integer NOT NULL,
  "remark" text NOT NULL,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  FOREIGN KEY ("uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("contact_uid") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Table structure for announcement
-- ----------------------------
DROP TABLE IF EXISTS "announcement";
CREATE TABLE "announcement" (
  "gid" integer NOT NULL PRIMARY KEY,
  "content" text NOT NULL DEFAULT '',
  "created_by" integer NOT NULL,
  "created_at" timestamp NOT NULL DEFAULT current_timestamp,
  "updated_at" timestamp NOT NULL DEFAULT current_timestamp,
  FOREIGN KEY ("gid") REFERENCES "group" ("gid") ON DELETE CASCADE ON UPDATE NO ACTION,
  FOREIGN KEY ("created_by") REFERENCES "user" ("uid") ON DELETE CASCADE ON UPDATE NO ACTION
);

-- ----------------------------
-- Auto increment value for bot_key
-- ----------------------------
UPDATE "sqlite_sequence" SET seq = 6 WHERE name = 'bot_key';

-- ----------------------------
-- Indexes structure for table bot_key
-- ----------------------------
CREATE INDEX "bot_key_uid"
ON "bot_key" (
  "uid" ASC
);
CREATE UNIQUE INDEX "bot_key_uid_name"
ON "bot_key" (
  "uid" ASC,
  "name" ASC
);

-- ----------------------------
-- Indexes structure for table burn_after_reading
-- ----------------------------
CREATE INDEX "burn_after_reading_uid"
ON "burn_after_reading" (
  "uid" ASC
);
CREATE UNIQUE INDEX "burn_after_reading_uid_gid"
ON "burn_after_reading" (
  "uid" ASC,
  "target_gid" ASC
);
CREATE UNIQUE INDEX "burn_after_reading_uid_uid"
ON "burn_after_reading" (
  "uid" ASC,
  "target_uid" ASC
);

-- ----------------------------
-- Indexes structure for table contacts
-- ----------------------------
CREATE INDEX "contacts_uid"
ON "contacts" (
  "uid" ASC
);
CREATE UNIQUE INDEX "contacts_uid_uid"
ON "contacts" (
  "uid" ASC,
  "target_uid" ASC
);

-- ----------------------------
-- Indexes structure for table favorite_archive
-- ----------------------------
CREATE UNIQUE INDEX "favorite_archive_uid_archive_id"
ON "favorite_archive" (
  "uid" ASC,
  "archive_id" ASC
);

-- ----------------------------
-- Indexes structure for table files
-- ----------------------------
CREATE INDEX "files_from_uid"
ON "files" (
  "from_uid" ASC
);
CREATE INDEX "files_from_uid_gid"
ON "files" (
  "from_uid" ASC,
  "gid" ASC
);

-- ----------------------------
-- Indexes structure for table group_user
-- ----------------------------
CREATE INDEX "group_user_gid"
ON "group_user" (
  "gid" ASC
);
CREATE UNIQUE INDEX "group_user_gid_uid"
ON "group_user" (
  "gid" ASC,
  "uid" ASC
);

-- ----------------------------
-- Indexes structure for table matrix_device_otk
-- ----------------------------
CREATE INDEX "matrix_device_otk_uid_device_unused"
ON "matrix_device_otk" (
  "uid" ASC,
  "device_id" ASC,
  "used" ASC
);

-- ----------------------------
-- Indexes structure for table matrix_megolm_inbound_session
-- ----------------------------
CREATE INDEX "matrix_megolm_inbound_room_sender"
ON "matrix_megolm_inbound_session" (
  "room_id" ASC,
  "sender_uid" ASC
);

-- ----------------------------
-- Indexes structure for table matrix_olm_inbound_session
-- ----------------------------
CREATE INDEX "matrix_olm_inbound_local"
ON "matrix_olm_inbound_session" (
  "local_uid" ASC,
  "sender_uid" ASC
);

-- ----------------------------
-- Indexes structure for table matrix_olm_outbound_session
-- ----------------------------
CREATE INDEX "matrix_olm_outbound_sender"
ON "matrix_olm_outbound_session" (
  "sender_uid" ASC,
  "recipient_uid" ASC
);

-- ----------------------------
-- Indexes structure for table mute
-- ----------------------------
CREATE INDEX "mute_expired_at"
ON "mute" (
  "expired_at" ASC
);
CREATE INDEX "mute_uid"
ON "mute" (
  "uid" ASC
);
CREATE UNIQUE INDEX "mute_uid_gid"
ON "mute" (
  "uid" ASC,
  "mute_gid" ASC
);
CREATE UNIQUE INDEX "mute_uid_uid"
ON "mute" (
  "uid" ASC,
  "mute_uid" ASC
);

-- ----------------------------
-- Indexes structure for table openid_connect
-- ----------------------------
CREATE UNIQUE INDEX "openid_connect_uid"
ON "openid_connect" (
  "uid" ASC
);

-- ----------------------------
-- Indexes structure for table pinned_chat
-- ----------------------------
CREATE INDEX "pinned_chat_uid"
ON "pinned_chat" (
  "uid" ASC
);
CREATE UNIQUE INDEX "pinned_chat_uid_gid"
ON "pinned_chat" (
  "uid" ASC,
  "target_gid" ASC
);
CREATE UNIQUE INDEX "pinned_chat_uid_uid"
ON "pinned_chat" (
  "uid" ASC,
  "target_uid" ASC
);

-- ----------------------------
-- Indexes structure for table pinned_message
-- ----------------------------
CREATE INDEX "pinned_message_gid"
ON "pinned_message" (
  "gid" ASC
);
CREATE INDEX "pinned_message_gid_mid"
ON "pinned_message" (
  "gid" ASC,
  "mid" ASC
);

-- ----------------------------
-- Auto increment value for read_index
-- ----------------------------
UPDATE "sqlite_sequence" SET seq = 2 WHERE name = 'read_index';

-- ----------------------------
-- Indexes structure for table read_index
-- ----------------------------
CREATE INDEX "read_index_uid"
ON "read_index" (
  "uid" ASC
);
CREATE UNIQUE INDEX "read_index_uid_gid"
ON "read_index" (
  "uid" ASC,
  "target_gid" ASC
);
CREATE UNIQUE INDEX "read_index_uid_uid"
ON "read_index" (
  "uid" ASC,
  "target_uid" ASC
);

-- ----------------------------
-- Auto increment value for user
-- ----------------------------
UPDATE "sqlite_sequence" SET seq = 3 WHERE name = 'user';

-- ----------------------------
-- Indexes structure for table user
-- ----------------------------
CREATE UNIQUE INDEX "user_email"
ON "user" (
  "email" ASC
);
CREATE UNIQUE INDEX "user_name"
ON "user" (
  "name" ASC
);

-- ----------------------------
-- Auto increment value for user_log
-- ----------------------------
UPDATE "sqlite_sequence" SET seq = 3 WHERE name = 'user_log';

-- ----------------------------
-- Indexes structure for table user_remark
-- ----------------------------
CREATE UNIQUE INDEX "uid_contact_uid_key"
ON "user_remark" (
  "uid" ASC,
  "contact_uid" ASC
);

PRAGMA foreign_keys = true;
