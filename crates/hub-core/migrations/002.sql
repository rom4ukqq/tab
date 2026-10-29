-- Remove TAB's persistent conversation copy. Configuration and security metadata stay.
PRAGMA secure_delete=ON;
BEGIN IMMEDIATE;
DROP TABLE IF EXISTS main.interactions;
DROP TABLE IF EXISTS main.events;
DROP TABLE IF EXISTS main.telegram_messages;
DROP TABLE IF EXISTS main.outbox;
DROP TABLE IF EXISTS main.routes;
DROP TABLE IF EXISTS main.assets;
DROP TABLE IF EXISTS main.turns;
DROP TABLE IF EXISTS main.sessions;
DELETE FROM settings WHERE key LIKE 'tg_text:%' OR key='tg_event_cursor';
DELETE FROM grants;
PRAGMA user_version=2;
COMMIT;
PRAGMA wal_checkpoint(TRUNCATE);
VACUUM;
PRAGMA wal_checkpoint(TRUNCATE);
