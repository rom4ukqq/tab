use anyhow::{anyhow, Result};
use rusqlite::Connection;
use std::path::Path;
use tokio::sync::{mpsc, oneshot};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;
#[cfg(test)]
mod transient_tests {
    use super::*;
    #[tokio::test]
    async fn migration_removes_history_and_preserves_configuration() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("hub.db");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(include_str!("../migrations/001.sql")).unwrap();
        old.execute_batch("INSERT INTO settings VALUES('config','keep-config'); INSERT INTO settings VALUES('tg_text:old','private-response-marker'); INSERT INTO sessions VALUES('old','private-prompt-marker'); INSERT INTO attempts VALUES(1,5,0,99999,1);").unwrap();
        drop(old);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.setting("config").await.unwrap().as_deref(), Some("keep-config"));
        assert!(store.setting("tg_text:old").await.unwrap().is_none());
        store.call(|db| { db.execute("INSERT INTO sessions VALUES('new','runtime-only-marker')", [])?; Ok(()) }).await.unwrap();
        let disk = Connection::open(&path).unwrap();
        let count: i64 = disk.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('sessions','turns','events','outbox','interactions','assets','routes','telegram_messages')", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        let blocked: i64 = disk.query_row("SELECT blocked_until FROM attempts", [], |r| r.get(0)).unwrap(); assert_eq!(blocked, 99999);
        let bytes = std::fs::read(&path).unwrap();
        for marker in [b"private-response-marker".as_slice(), b"private-prompt-marker", b"runtime-only-marker"] { assert!(!bytes.windows(marker.len()).any(|s| s == marker)); }
    }
    #[tokio::test]
    async fn delivered_closed_context_is_discarded() {
        let dir = tempfile::tempdir().unwrap(); let store = Store::open(&dir.path().join("hub.db")).unwrap();
        store.call(|db| { db.execute_batch("INSERT INTO sessions VALUES('s','{}'); INSERT INTO turns VALUES('t','s','completed','{\"text\":\"prompt\",\"assets\":[]}',0); INSERT INTO events(session_id,turn_id,body) VALUES('s','t','{\"kind\":\"completed\"}'); INSERT INTO runtime_settings VALUES('tg_event_cursor','1'); INSERT INTO runtime_settings VALUES('tg_text:t','reply'); INSERT INTO closed_sessions VALUES('s');")?; Ok(()) }).await.unwrap();
        store.prune_closed_sessions().await.unwrap();
        let remaining: i64 = store.call(|db| Ok(db.query_row("SELECT (SELECT COUNT(*) FROM sessions)+(SELECT COUNT(*) FROM turns)+(SELECT COUNT(*) FROM events)+(SELECT COUNT(*) FROM runtime_settings WHERE key LIKE 'tg_text:%')", [], |r| r.get(0))?)).await.unwrap();
        assert_eq!(remaining, 0);
    }
    #[test]
    fn only_tab_generated_log_exports_are_removed() {
        let dir = tempfile::tempdir().unwrap(); let exports = dir.path().join("exports"); std::fs::create_dir(&exports).unwrap();
        let old = exports.join(format!("{}.jsonl", uuid::Uuid::new_v4())); std::fs::write(&old, "old history").unwrap();
        std::fs::write(exports.join("notes.jsonl"), "user file").unwrap();
        crate::security::purge_tab_log_exports(dir.path()).unwrap();
        assert!(!old.exists()); assert!(exports.join("notes.jsonl").exists()); assert!(exports.is_dir());
    }
}
#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<Job>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 2 {
            return Err(anyhow!(
                "База создана более новой версией TAB; downgrade запрещён"
            ));
        }
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;")?;
        if version == 0 { conn.execute_batch(include_str!("../migrations/001.sql"))?; }
        if version < 2 { conn.execute_batch(include_str!("../migrations/002.sql"))?; }
        conn.execute_batch(include_str!("../migrations/runtime.sql"))?;
        conn.execute("DELETE FROM grants", [])?;
        let (tx, mut rx) = mpsc::channel::<Job>(128);
        std::thread::Builder::new()
            .name("hub-sqlite".into())
            .spawn(move || {
                while let Some(job) = rx.blocking_recv() {
                    job(&mut conn);
                }
            })?;
        Ok(Self { tx })
    }
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Box::new(move |c| {
                let _ = tx.send(f(c));
            }))
            .await
            .map_err(|_| anyhow!("Storage worker stopped"))?;
        rx.await.map_err(|_| anyhow!("Storage worker stopped"))?
    }
    pub async fn setting(&self, key: &str) -> Result<Option<String>> {
        let k = key.to_owned();
        self.call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(
                c.query_row("SELECT value FROM settings WHERE key=?1", [k], |r| r.get(0))
                    .optional()?,
            )
        })
        .await
    }
    pub async fn put(&self, key: &str, value: String) -> Result<()> {
        let k = key.to_owned();
        self.call(move|c|{c.execute("INSERT INTO settings VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[k,value])?;Ok(())}).await
    }
    pub async fn audit(&self, action: &str, target: &str) -> Result<()> {
        let (a, t) = (action.to_owned(), target.to_owned());
        self.call(move |c| {
            c.execute(
                "INSERT INTO audit(at,action,target) VALUES(?1,?2,?3)",
                rusqlite::params![crate::domain::now(), a, t],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn prune_closed_sessions(&self) -> Result<()> {
        self.call(|db| {
            let tx = db.transaction()?;
            tx.execute("DELETE FROM outbox WHERE state NOT IN ('queued','sending')", [])?;
            let pending: i64 = tx.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?;
            if pending == 0 {
                tx.execute_batch("CREATE TEMP TABLE delivered_turns AS SELECT id FROM turns WHERE state NOT IN ('queued','dispatching','running') AND EXISTS(SELECT 1 FROM events WHERE turn_id=turns.id AND json_extract(body,'$.kind')='completed' AND seq<=COALESCE((SELECT CAST(value AS INTEGER) FROM runtime_settings WHERE key='tg_event_cursor'),0));
                    DELETE FROM runtime_settings WHERE key IN (SELECT 'tg_text:'||id FROM delivered_turns);
                    DELETE FROM interactions WHERE turn_id IN (SELECT id FROM delivered_turns);
                    DELETE FROM events WHERE turn_id IN (SELECT id FROM delivered_turns);
                    DELETE FROM telegram_messages WHERE turn_id IN (SELECT id FROM delivered_turns);
                    UPDATE turns SET body=json_set(body,'$.text','','$.assets',json('[]')) WHERE id IN (SELECT id FROM delivered_turns);
                    DROP TABLE delivered_turns;")?;
                // Only discard a closed context once cancellation and message projection finish.
                tx.execute_batch("CREATE TEMP TABLE discard_sessions AS SELECT id FROM closed_sessions WHERE NOT EXISTS(SELECT 1 FROM turns WHERE session_id=closed_sessions.id AND state IN ('queued','dispatching','running')) AND NOT EXISTS(SELECT 1 FROM events WHERE session_id=closed_sessions.id AND seq>COALESCE((SELECT CAST(value AS INTEGER) FROM runtime_settings WHERE key='tg_event_cursor'),0));
                    DELETE FROM runtime_settings WHERE key IN (SELECT 'tg_text:'||id FROM turns WHERE session_id IN (SELECT id FROM discard_sessions));
                    DELETE FROM telegram_messages WHERE turn_id IN (SELECT id FROM turns WHERE session_id IN (SELECT id FROM discard_sessions));
                    DELETE FROM interactions WHERE turn_id IN (SELECT id FROM turns WHERE session_id IN (SELECT id FROM discard_sessions));
                    DELETE FROM events WHERE session_id IN (SELECT id FROM discard_sessions);
                    DELETE FROM turns WHERE session_id IN (SELECT id FROM discard_sessions);
                    DELETE FROM sessions WHERE id IN (SELECT id FROM discard_sessions);
                    DELETE FROM closed_sessions WHERE id IN (SELECT id FROM discard_sessions);
                    DROP TABLE discard_sessions;")?;
            }
            tx.commit()?;
            Ok(())
        }).await
    }
}
