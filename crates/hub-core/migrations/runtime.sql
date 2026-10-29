-- A transient bridge context, never written to the application database or TEMP files.
PRAGMA temp_store=MEMORY;
CREATE TEMP TABLE sessions(id TEXT PRIMARY KEY,body TEXT NOT NULL);
CREATE TEMP TABLE turns(id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES sessions(id),state TEXT NOT NULL,body TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE INDEX temp.turns_session ON turns(session_id,created_at);
CREATE TEMP TABLE events(seq INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,turn_id TEXT NOT NULL,body TEXT NOT NULL);
CREATE TEMP TABLE interactions(id TEXT PRIMARY KEY,turn_id TEXT NOT NULL,state TEXT NOT NULL,body TEXT NOT NULL);
CREATE TEMP TABLE routes(chat_id INTEGER PRIMARY KEY,provider TEXT,project_id TEXT,session_id TEXT);
CREATE TEMP TABLE outbox(id TEXT PRIMARY KEY,method TEXT NOT NULL,body TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'queued',next_at INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,route_key TEXT);
CREATE INDEX temp.outbox_due ON outbox(state,next_at);
CREATE TEMP TABLE telegram_messages(turn_id TEXT PRIMARY KEY,chat_id INTEGER NOT NULL,message_id INTEGER NOT NULL);
CREATE TEMP TABLE assets(id TEXT PRIMARY KEY,body TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE TEMP TABLE runtime_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TEMP TABLE closed_sessions(id TEXT PRIMARY KEY);
