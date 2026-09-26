-- Per-customer Telegram bot columns (mika#1454).
-- Each customer can bring their own Telegram bot (via @BotFather).
-- All columns are nullable: NULL means "use single-bot mode fallback."
ALTER TABLE customers ADD COLUMN bot_token TEXT;
ALTER TABLE customers ADD COLUMN bot_username TEXT;
ALTER TABLE customers ADD COLUMN webhook_secret TEXT;
