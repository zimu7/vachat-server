-- Add agent_type column to user table.
-- Used to identify which agent (qwenpaw / hermes / cc-connect / ...) a bot is, so
-- the Matrix bridge can dispatch to the correct content-type converter. NULL for
-- existing rows (treated as unknown -> messages fall through to prose).
ALTER TABLE "user" ADD COLUMN "agent_type" TEXT;
