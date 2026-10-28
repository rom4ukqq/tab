use anyhow::{anyhow, Result};
use rusqlite::Connection;
use std::path::Path;
use tokio::sync::{mpsc, oneshot};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;
#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<Job>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            return Err(anyhow!(
                "База создана более новой версией Agent Hub; downgrade запрещён"
            ));
        }
        conn.execute_batch(include_str!("../migrations/001.sql"))?;
        conn.execute_batch("UPDATE turns SET state='interrupted' WHERE state IN ('running','dispatching'); UPDATE turns SET state='cancelled' WHERE state='queued'; UPDATE interactions SET state='obsolete' WHERE state='pending'; DELETE FROM grants; UPDATE outbox SET state='unknown' WHERE state='sending';")?;
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
}
