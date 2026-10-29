pub mod adapter;
pub mod domain;
pub mod process;
pub mod security;
pub mod store;
pub mod telegram;
mod telegram_render;
mod telegram_activity;

use anyhow::{anyhow, bail, Result};
use domain::*;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use store::Store;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

struct Active {
    cancel: CancellationToken,
    control: mpsc::Sender<Control>,
    remote: bool,
}
struct PendingPairing {
    code: zeroize::Zeroizing<String>,
    hash: String,
    pepper: zeroize::Zeroizing<String>,
    issued_at: i64,
    expires_at: i64,
    deadline: std::time::Instant,
    epoch: String,
}
pub struct Hub {
    pub db: Store,
    pub config: RwLock<Config>,
    pub data_dir: PathBuf,
    pub events: broadcast::Sender<Event>,
    active: Mutex<HashMap<String, Active>>,
    locks: Mutex<HashMap<String, Arc<Semaphore>>>,
    slots: Arc<Semaphore>,
    pub telegram_stop: Mutex<Option<CancellationToken>>,
    pub telegram_status: RwLock<String>,
    pub online_at: std::sync::atomic::AtomicI64,
    pairing: Mutex<Option<PendingPairing>>,
}
impl Hub {
    pub async fn open(data_dir: PathBuf) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)?;
        security::purge_tab_log_exports(&data_dir)?;
        let db = Store::open(&data_dir.join("hub.db"))?;
        let mut config: Config = match db.setting("config").await? {
            Some(s) => serde_json::from_str(&s)?,
            None => Config::default(),
        };
        if data_dir.file_name().is_some_and(|name| name == "TAB") {
            if let Some(parent) = data_dir.parent() {
                let legacy = parent.join("AgentHub");
                let canonical_legacy = parent.canonicalize()?.join("AgentHub");
                for project in &mut config.projects {
                    if let Ok(relative) = project.root.strip_prefix(&legacy)
                        .or_else(|_| project.root.strip_prefix(&canonical_legacy)) {
                        project.root = security::safe_root(&data_dir.join(relative))?;
                    }
                }
            }
        }
        config.auth_epoch = id();
        db.put("config", serde_json::to_string(&config)?).await?;
        let (events, _) = broadcast::channel(512);
        Ok(Arc::new(Self {
            db,
            config: RwLock::new(config),
            data_dir,
            events,
            active: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(2)),
            telegram_stop: Mutex::new(None),
            telegram_status: RwLock::new("Отключён".into()),
            online_at: std::sync::atomic::AtomicI64::new(0),
            pairing: Mutex::new(None),
        }))
    }
    async fn persist_config(&self, c: &Config) -> Result<()> {
        self.db.put("config", serde_json::to_string(c)?).await
    }
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let config = self.config.read().await.clone();
        let (sessions, turns, requests) = self
            .db
            .call(|c| {
                let mut s = c.prepare("SELECT body FROM sessions ORDER BY rowid DESC")?;
                let sessions = s
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|s| serde_json::from_str(&s))
                    .collect::<Result<Vec<Session>, _>>()?;
                let mut s =
                    c.prepare("SELECT body,state FROM turns ORDER BY created_at DESC LIMIT 100")?;
                let rows = s
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut turns = vec![];
                for (body, state) in rows {
                    let mut t: Turn = serde_json::from_str(&body)?;
                    t.state = state;
                    turns.push(t)
                }
                let mut s = c.prepare("SELECT body FROM interactions WHERE state='pending'")?;
                let requests = s
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|s| serde_json::from_str(&s))
                    .collect::<Result<Vec<Interaction>, _>>()?;
                Ok((sessions, turns, requests))
            })
            .await?;
        let telegram_authorized = match config.telegram_user { Some(user) => self.authorized(user, false).await?, None => false };
        Ok(Snapshot {
            config,
            sessions,
            turns,
            requests,
            telegram_status: self.telegram_status.read().await.clone(),
            telegram_active: self.telegram_stop.lock().await.is_some(),
            telegram_authorized,
            pairing_code: self.pairing_snapshot().await,
        })
    }
    pub async fn discover(&self) -> Result<Vec<Installation>> {
        let discovered = adapter::discover().await;
        let mut c = self.config.write().await;
        let old = c.installations.clone();
        c.installations = discovered
            .into_iter()
            .map(|mut i| {
                if let Some(o) = old
                    .iter()
                    .find(|o| o.executable == i.executable && o.version == i.version)
                {
                    i.enabled = o.enabled;
                    i.accept_unsandboxed = o.accept_unsandboxed
                };
                i
            })
            .collect();
        self.persist_config(&c).await?;
        Ok(c.installations.clone())
    }
    pub async fn enable(
        &self,
        provider: Provider,
        enabled: bool,
        accept_unsandboxed: bool,
    ) -> Result<()> {
        let mut c = self.config.write().await;
        let i = c
            .installations
            .iter_mut()
            .find(|i| i.provider == provider)
            .ok_or_else(|| anyhow!("Агент не обнаружен"))?;
        i.enabled = enabled;
        i.accept_unsandboxed = accept_unsandboxed;
        self.persist_config(&c).await
    }
    pub async fn add_project(&self, name: String, root: PathBuf) -> Result<Project> {
        let root = security::safe_root(&root)?;
        if name.trim().is_empty() {
            bail!("Укажите имя проекта")
        };
        let mut c = self.config.write().await;
        if c.projects.iter().any(|p| p.root == root) {
            bail!("Проект уже добавлен")
        };
        let project = Project {
            id: id(),
            name,
            root,
        };
        c.projects.push(project.clone());
        self.persist_config(&c).await?;
        Ok(project)
    }
    pub async fn create_agent_session(&self, provider: Provider) -> Result<Session> {
        let mut c = self.config.write().await;
        if !c.installations.iter().any(|i| i.provider == provider && i.enabled) {
            bail!("Агент отключён или не обнаружен")
        }
        // A neutral cwd is required by CLI transports, not a user-selected project.
        // Keep it separate from Hub credentials and database; retain native approvals.
        let workspaces = self.data_dir.join("workspaces");
        security::reject_links(&self.data_dir)?;
        std::fs::create_dir_all(&workspaces)?;
        security::reject_links(&workspaces)?;
        let root = workspaces.join(provider.key());
        std::fs::create_dir_all(&root)?;
        let root = security::safe_root(&root)?;
        let project_id = if let Some(p) = c.projects.iter().find(|p| p.root == root) {
            p.id.clone()
        } else {
            let p = Project { id: id(), name: format!("{} workspace", provider.key()), root };
            let project_id = p.id.clone();
            c.projects.push(p);
            self.persist_config(&c).await?;
            project_id
        };
        drop(c);
        self.create_session(project_id, provider, "Telegram".into(), None).await
    }
    pub async fn create_session(
        &self,
        project_id: String,
        provider: Provider,
        title: String,
        model: Option<String>,
    ) -> Result<Session> {
        if !self
            .config
            .read()
            .await
            .projects
            .iter()
            .any(|p| p.id == project_id)
        {
            bail!("Неизвестный проект")
        }
        let s = Session {
            id: id(),
            project_id,
            provider,
            title: if title.trim().is_empty() {
                "Новая сессия".into()
            } else {
                title
            },
            native_id: None,
            model: model.filter(|x| !x.trim().is_empty()),
        };
        let body = serde_json::to_string(&s)?;
        let sid = s.id.clone();
        self.db
            .call(move |c| {
                c.execute("INSERT INTO sessions VALUES(?1,?2)", params![sid, body])?;
                Ok(())
            })
            .await?;
        Ok(s)
    }
    pub async fn session(&self, sid: &str) -> Result<Session> {
        let sid = sid.to_string();
        self.db
            .call(move |c| {
                let body: String =
                    c.query_row("SELECT body FROM sessions WHERE id=?1", [sid], |r| r.get(0))?;
                Ok(serde_json::from_str(&body)?)
            })
            .await
    }
    pub async fn import_native(&self, sid: String, native_id: String) -> Result<()> {
        if native_id.trim().is_empty() || native_id.len() > 4096 {
            bail!("Неверный native ID")
        };
        let mut s = self.session(&sid).await?;
        let target = sid.clone();
        let active=self.db.call(move|c|Ok(c.query_row("SELECT COUNT(*) FROM turns WHERE session_id=?1 AND state IN ('running','dispatching','queued')",[target],|r|r.get::<_,i64>(0))?>0)).await?;
        if active {
            bail!("Сначала остановите задачи сессии")
        };
        if s.provider == Provider::Omp {
            let p = PathBuf::from(&native_id);
            if !p.is_absolute() || !p.is_file() {
                bail!("OMP: выберите существующий native session file")
            }
        }
        s.native_id = Some(native_id);
        let body = serde_json::to_string(&s)?;
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE sessions SET body=?2 WHERE id=?1",
                    params![sid, body],
                )?;
                Ok(())
            })
            .await
    }
    pub async fn setup_telegram(
        &self,
        mut token: String,
        user_id: i64,
    ) -> Result<()> {
        if self.telegram_stop.lock().await.is_some() {
            bail!("Сначала отключите Telegram и дождитесь завершения соединения")
        }
        if user_id <= 0 || !token.contains(':') || token.len() > 256 {
            bail!("Неверный token или Telegram User ID")
        }
        let client = telegram::Client::new(token.clone());
        client.check().await?;
        let machine = self.config.read().await.machine_id.clone();
        security::secret_set(&format!("{machine}/telegram"), &token)?;
        use zeroize::Zeroize;
        token.zeroize();
        let mut c = self.config.write().await;
        c.telegram_user = Some(user_id);
        c.auth_epoch = id();
        self.persist_config(&c).await?;
        drop(c);
        self.revoke().await?;
        self.db
            .audit("telegram_configured", &user_id.to_string())
            .await
    }
    pub async fn pairing_snapshot(&self) -> Option<PairingCode> {
        let mut pairing = self.pairing.lock().await;
        if pairing.as_ref().is_some_and(|p| p.expires_at <= now() || p.deadline <= std::time::Instant::now()) { *pairing = None; }
        pairing.as_ref().map(|p| PairingCode { code: p.code.to_string(), expires_at: p.expires_at })
    }
    // Local-only command. The code and pepper never enter persistent storage or bot messages.
    pub async fn issue_pairing_code(&self) -> Result<()> {
        let c = self.config.read().await.clone();
        if c.telegram_user.is_none() { bail!("Configure Telegram first") }
        let mut pending = self.pairing.lock().await;
        let code = zeroize::Zeroizing::new(security::random_pairing_code());
        let pepper = zeroize::Zeroizing::new(security::random_secret());
        let p = code.clone(); let pe = pepper.clone();
        let hash = tokio::task::spawn_blocking(move || security::hash_pin(&p, &pe)).await??;
        let issued_at = now();
        *pending = Some(PendingPairing { code, hash, pepper, issued_at, expires_at: issued_at + 300, deadline: std::time::Instant::now() + std::time::Duration::from_secs(300), epoch: c.auth_epoch });
        Ok(())
    }
    pub async fn unlock(&self, user: i64, pin: &str, sent_at: i64) -> Result<bool> {
        let c = self.config.read().await.clone();
        if c.telegram_user != Some(user) {
            return Ok(false);
        }
        // Hold this guard until commit + consume, serializing verification, renewal and revoke.
        let mut pending = self.pairing.lock().await;
        let Some(p) = pending.as_ref() else { return Ok(false) };
        if p.expires_at <= now() || p.deadline <= std::time::Instant::now() { *pending = None; return Ok(false) }
        if p.epoch != c.auth_epoch || sent_at < p.issued_at || sent_at > now() + 30 { return Ok(false) }
        let hash = p.hash.clone(); let pepper = p.pepper.clone();
        let expires_at = p.expires_at; let deadline = p.deadline;
        let pin = zeroize::Zeroizing::new(pin.to_string());
        let accepted = self.db.call(move|db|{
            let ts=now();let tx=db.transaction()?;
            let(failures,window,blocked,strikes)=tx.query_row("SELECT failures,window_at,blocked_until,strikes FROM attempts WHERE user_id=?1",[user],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).optional()?.unwrap_or((0,ts,0,0));
            if blocked>ts || expires_at<=ts || deadline<=std::time::Instant::now(){return Ok(false)}
            if security::verify_pin(&pin,&pepper,&hash){if expires_at<=now() || deadline<=std::time::Instant::now(){return Ok(false)}tx.execute("INSERT OR REPLACE INTO grants VALUES(?1,?2,?3,?4)",params![user,c.auth_epoch,ts,ts+8*3600])?;tx.execute("DELETE FROM attempts WHERE user_id=?1",[user])?;tx.commit()?;return Ok(true)}
            let failures=if ts-window>900{1}else{failures+1};let window=if ts-window>900{ts}else{window};let strikes=if failures>=5{strikes+1}else{strikes};let blocked=if failures>=5{ts+(900*(1i64<<(strikes-1).min(7))).min(86400)}else{0};
            tx.execute("INSERT OR REPLACE INTO attempts VALUES(?1,?2,?3,?4,?5)",params![user,failures,window,blocked,strikes])?;tx.commit()?;Ok(false)
        }).await?;
        if accepted { *pending = None; }
        Ok(accepted)
    }
    pub async fn authorized(&self, user: i64, touch: bool) -> Result<bool> {
        let c = self.config.read().await.clone();
        if c.telegram_user != Some(user) {
            return Ok(false);
        }
        self.db
            .call(move |db| {
                let ts = now();
                let row = db
                    .query_row(
                        "SELECT epoch,last_used,expires FROM grants WHERE user_id=?1",
                        [user],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .optional()?;
                let valid = row
                    .map(|(e, last, exp)| e == c.auth_epoch && last + 900 > ts && exp > ts)
                    .unwrap_or(false);
                if valid && touch {
                    db.execute(
                        "UPDATE grants SET last_used=?2 WHERE user_id=?1",
                        params![user, ts],
                    )?;
                }
                Ok(valid)
            })
            .await
    }
    pub async fn revoke(&self) -> Result<()> {
        let mut pending = self.pairing.lock().await;
        *pending = None;
        self.db
            .call(|c| {
                c.execute("DELETE FROM grants", [])?;
                Ok(())
            })
            .await?;
        for a in self.active.lock().await.values() {
            if a.remote {
                a.cancel.cancel();
            }
        }
        self.db.audit("access_revoked", "all").await
    }
    pub async fn emergency_stop(&self) -> Result<()> {
        for a in self.active.lock().await.values() {
            a.cancel.cancel();
        }
        self.revoke().await?;
        self.db.audit("emergency_stop", "all").await
    }
    pub async fn cancel(&self, turn_id: &str) -> Result<()> {
        if let Some(a) = self.active.lock().await.get(turn_id) {
            a.cancel.cancel();
        }
        let tid = turn_id.to_string();
        self.db.call(move |db| { db.execute("UPDATE turns SET state='cancelled' WHERE id=?1 AND state='push_unknown'", [tid])?; Ok(()) }).await?;
        Ok(())
    }
    pub async fn steer(&self, turn_id: &str, text: String) -> Result<()> {
        let tid = turn_id.to_string();
        let running = self.db.call(move |db| Ok(db.query_row("SELECT state FROM turns WHERE id=?1", [tid], |r| r.get::<_, String>(0))? == "running")).await?;
        if !running { bail!("Задача ещё не запущена; сообщение остаётся в очереди") }
        let map = self.active.lock().await;
        let a = map.get(turn_id).ok_or_else(|| anyhow!("Turn не активен"))?;
        a.control.send(Control::Steer(text)).await?;
        Ok(())
    }
    pub async fn agent_options(&self, sid: &str) -> Result<AgentOptions> {
        let key = format!("agent_options:{sid}");
        self.db.call(move |db| {
            let value: Option<String> = db.query_row("SELECT value FROM runtime_settings WHERE key=?1", [key], |r| r.get(0)).optional()?;
            Ok(value.map(|v| serde_json::from_str(&v)).transpose()?.unwrap_or_default())
        }).await
    }
    pub async fn push_queued(&self, sid: &str, tid: &str, chat: i64, user: i64) -> Result<String> {
        if !self.authorized(user, true).await? { bail!("Доступ истёк") }
        let session = self.session(sid).await?;
        if !matches!(session.provider, Provider::Codex | Provider::Omp) { return Ok("Push поддерживают Codex и OMP.".into()); }
        let (sid, tid) = (sid.to_string(), tid.to_string());
        let queued_id = tid.clone();
        let claimed = self.db.call(move |db| {
            let tx = db.transaction()?;
            let row: Option<String> = tx.query_row("SELECT body FROM turns WHERE id=?1 AND session_id=?2 AND state='queued'", params![tid,sid], |r| r.get(0)).optional()?;
            let Some(body) = row else { return Ok(None) };
            let queued: Turn = serde_json::from_str(&body)?;
            if queued.chat_id != Some(chat) || queued.user_id != Some(user) || !queued.assets.is_empty() { return Ok(None) }
            let active: Option<String> = tx.query_row("SELECT body FROM turns WHERE session_id=?1 AND state='running'", [sid], |r|r.get(0)).optional()?;
            let Some(active) = active else { return Ok(None) };
            let active: Turn = serde_json::from_str(&active)?;
            if active.chat_id != Some(chat) || active.user_id != Some(user) { return Ok(None) }
            tx.execute("UPDATE turns SET state='steering' WHERE id=?1", [tid])?;
            tx.commit()?;
            Ok(Some((active.id, queued.text)))
        }).await?;
        let Some((active_id, text)) = claimed else { return Ok("Push недоступен: нужна работающая задача и ожидающее текстовое сообщение без вложений.".into()) };
        let (reply, result) = tokio::sync::oneshot::channel();
        let sent = {
            let map = self.active.lock().await;
            map.get(&active_id).map(|a| a.control.try_send(Control::SteerWithAck { text, reply }).is_ok()).unwrap_or(false)
        };
        let (state, message) = if !sent { ("queued", "Адаптер недоступен; сообщение осталось в очереди.") } else {
            match tokio::time::timeout(std::time::Duration::from_secs(15), result).await {
                Ok(Ok(true)) => ("pushed", "Сообщение принято в текущую задачу и снято с очереди."),
                Ok(Ok(false)) => ("queued", "CLI отклонил push; сообщение осталось в очереди."),
                _ => ("push_unknown", "CLI не подтвердил push за 15 секунд. Исход неизвестен; автоматического повтора нет. Текст сохранён в /queue."),
            }
        };
        self.db.call(move |db| { db.execute("UPDATE turns SET state=?2 WHERE id=?1 AND state='steering'",params![queued_id,state])?;Ok(()) }).await?;
        Ok(message.into())
    }
    pub async fn set_agent_options(&self, sid: &str, options: AgentOptions) -> Result<()> {
        let key = format!("agent_options:{sid}");
        let value = serde_json::to_string(&options)?;
        self.db.call(move |db| { db.execute("INSERT OR REPLACE INTO runtime_settings VALUES(?1,?2)", params![key,value])?; Ok(()) }).await
    }
    pub async fn edit_queue(&self, sid: String, tid: String, user: i64, action: String, text: Option<String>) -> Result<()> {
        if !self.authorized(user, true).await? { bail!("Доступ истёк") }
        self.db.call(move |db| {
            let tx = db.transaction()?;
            let body: Option<String> = tx.query_row("SELECT body FROM turns WHERE id=?1 AND session_id=?2 AND state='queued'",params![tid,sid],|r|r.get(0)).optional()?;
            let mut turn: Turn = serde_json::from_str(&body.ok_or_else(||anyhow!("Сообщение уже запущено или снято с очереди. Обновите /queue."))?)?;
            if turn.user_id != Some(user) || turn.chat_id != Some(user) { bail!("Недоступная задача") }
            let ids = {
                let mut stmt=tx.prepare("SELECT id FROM turns WHERE session_id=?1 AND state='queued' ORDER BY COALESCE((SELECT CAST(value AS INTEGER) FROM runtime_settings WHERE key='queue:'||turns.id),rowid),rowid")?;
                let rows=stmt.query_map([&sid],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;rows
            };
            let at=ids.iter().position(|id|id==&tid).ok_or_else(||anyhow!("Очередь изменилась"))?;
            match action.as_str() {
                "edit" => { let text=text.unwrap_or_default();if text.trim().is_empty() || text.len()>80000 { bail!("Нужен непустой текст до 80000 байт") }turn.text=text; },
                "up" => {
                    let mut ids=ids;
                    if at>0 { ids.swap(at,at-1); }
                    for (i,id) in ids.iter().enumerate() { tx.execute("INSERT OR REPLACE INTO runtime_settings VALUES(?1,?2)",params![format!("queue:{id}"),(i as i64 - 1000).to_string()])?; }
                },
                "merge" => {
                    let next=ids.get(at+1).ok_or_else(||anyhow!("После этого сообщения нет ожидающей задачи"))?;
                    let raw:String=tx.query_row("SELECT body FROM turns WHERE id=?1",[next],|r|r.get(0))?;
                    let other:Turn=serde_json::from_str(&raw)?;
                    if other.user_id!=Some(user) || other.chat_id!=Some(user) { bail!("Недоступная задача") }
                    turn.text=format!("{}\n\n{}",turn.text,other.text);
                    turn.assets.extend(other.assets);
                    if turn.text.len()>80000 || turn.assets.len()>10 { bail!("После объединения превышен лимит текста или 10 вложений") }
                    tx.execute("UPDATE turns SET state='merged',body=json_set(body,'$.text','','$.assets',json('[]')) WHERE id=?1",[next])?;
                },
                _ => bail!("Неизвестное действие очереди"),
            }
            tx.execute("UPDATE turns SET body=?2 WHERE id=?1",params![tid,serde_json::to_string(&turn)?])?;
            tx.commit()?;Ok(())
        }).await
    }
    pub async fn submit(
        self: &Arc<Self>,
        session_id: String,
        text: String,
        assets: Vec<Asset>,
        remote: Option<(i64, i64)>,
    ) -> Result<Turn> {
        if text.trim().is_empty() && assets.is_empty() {
            bail!("Сообщение пустое")
        };
        if text.len() > 80000 || assets.len() > 10 {
            bail!("Слишком большой запрос")
        }
        let session = self.session(&session_id).await?;
        let root = self
            .config
            .read()
            .await
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .ok_or_else(|| anyhow!("Проект отсутствует"))?
            .root
            .clone();
        let mut verified = Vec::new();
        for asset in assets {
            let aid = asset.id;
            let a: Asset = self
                .db
                .call(move |c| {
                    let body: String =
                        c.query_row("SELECT body FROM assets WHERE id=?1", [aid], |r| r.get(0))?;
                    Ok(serde_json::from_str(&body)?)
                })
                .await?;
            security::safe_file(&a.path, &root, 20 * 1024 * 1024)?;
            verified.push(a)
        }
        let assets = verified;
        if let Some((_, u)) = remote {
            if !self.authorized(u, true).await? {
                bail!("Доступ истёк")
            }
        }
        let c = self.config.read().await;
        let turn = Turn {
            id: id(),
            session_id,
            text,
            assets,
            chat_id: remote.map(|r| r.0),
            user_id: remote.map(|r| r.1),
            epoch: c.auth_epoch.clone(),
            state: "queued".into(),
            created_at: now(),
        };
        drop(c);
        let t = turn.clone();
        self.db.call(move|db|{let tx=db.transaction()?;let count:i64=tx.query_row("SELECT COUNT(*) FROM turns WHERE session_id=?1 AND state IN ('queued','running','dispatching')",[&t.session_id],|r|r.get(0))?;if count>=11{bail!("Очередь сессии заполнена")};tx.execute("INSERT INTO turns VALUES(?1,?2,?3,?4,?5)",params![t.id,t.session_id,t.state,serde_json::to_string(&t)?,t.created_at])?;tx.commit()?;Ok(())}).await?;
        let (cancel, hub, t) = (CancellationToken::new(), self.clone(), turn.clone());
        let (control, rx) = mpsc::channel(32);
        self.active.lock().await.insert(
            turn.id.clone(),
            Active {
                cancel: cancel.clone(),
                control: control.clone(),
                remote: remote.is_some(),
            },
        );
        tokio::spawn(async move {
            if let Err(e) = hub.execute(t.clone(), cancel, control, rx).await {
                let _ = hub
                    .publish(&t, EventKind::Error, json!({"text":e.to_string()}))
                    .await;
                let _ = hub.finish(&t, "failed").await;
            }
            hub.active.lock().await.remove(&t.id);
        });
        Ok(turn)
    }
    async fn lock(&self, key: String) -> Arc<Semaphore> {
        self.locks
            .lock()
            .await
            .entry(key)
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }
    async fn execute(
        self: &Arc<Self>,
        turn: Turn,
        cancel: CancellationToken,
        control: mpsc::Sender<Control>,
        rx: mpsc::Receiver<Control>,
    ) -> Result<()> {
        let initial = self.session(&turn.session_id).await?;
        let project = self
            .config
            .read()
            .await
            .projects
            .iter()
            .find(|p| p.id == initial.project_id)
            .cloned()
            .ok_or_else(|| anyhow!("Проект отсутствует"))?;
        let (_permits, turn) = loop {
        let permits = async {
            let s = self
                .lock(format!("session:{}", initial.id))
                .await
                .acquire_owned()
                .await?;
            let p = self
                .lock(format!("project:{}", project.root.display()))
                .await
                .acquire_owned()
                .await?;
            let g = self.slots.clone().acquire_owned().await?;
            Ok::<_, anyhow::Error>((s, p, g))
        };
        let permits = tokio::select! {p=permits=>p?,_=cancel.cancelled()=>{self.finish(&turn,"cancelled").await?;return Ok(())}};
        let tid=turn.id.clone();
        let (pending, claimed)=self.db.call(move |db| {
            let tx=db.transaction()?;
            let (body,state):(String,String)=tx.query_row("SELECT body,state FROM turns WHERE id=?1",[&tid],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if !["queued","steering"].contains(&state.as_str()) { return Ok((false,None)) }
            let current:Turn=serde_json::from_str(&body)?;
            let head:Option<String>=tx.query_row("SELECT id FROM turns WHERE session_id=?1 AND state IN ('queued','steering') ORDER BY COALESCE((SELECT CAST(value AS INTEGER) FROM runtime_settings WHERE key='queue:'||turns.id),rowid),rowid LIMIT 1",[&current.session_id],|r|r.get(0)).optional()?;
            if state=="steering" || head.as_deref()!=Some(tid.as_str()) { return Ok((true,None)) }
            tx.execute("UPDATE turns SET state='dispatching' WHERE id=?1",[tid])?;
            tx.commit()?;Ok((true,Some(current)))
        }).await?;
        if let Some(current)=claimed { break (permits,current) }
        drop(permits);
        if !pending { return Ok(()) }
        tokio::select! { _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{}, _=cancel.cancelled()=>{self.finish(&turn,"cancelled").await?;return Ok(())} }
        };
        let session = self.session(&turn.session_id).await?;
        let c = self.config.read().await.clone();
        if let Some(u) = turn.user_id {
            if c.auth_epoch != turn.epoch || !self.authorized(u, false).await? {
                bail!("Remote grant истёк до начала задачи")
            }
        }
        let install = if session.provider == Provider::Mock {
            Installation {
                provider: Provider::Mock,
                executable: PathBuf::new(),
                version: "mock".into(),
                enabled: true,
                accept_unsandboxed: false,
            }
        } else {
            c.installations
                .iter()
                .find(|i| i.provider == session.provider)
                .cloned()
                .ok_or_else(|| anyhow!("CLI не обнаружен"))?
        };
        security::reject_links(&project.root)?;
        let tid = turn.id.clone();
        self.db
            .call(move |c| {
                c.execute("UPDATE turns SET state='running' WHERE id=?1", [tid])?;
                Ok(())
            })
            .await?;
        self.publish(
            &turn,
            EventKind::Progress,
            json!({"text":"Задача запущена"}),
        )
        .await?;
        let (tx, mut events) = mpsc::channel(128);
        let options = self.agent_options(&session.id).await?;
        let mut task = tokio::spawn(adapter::run_with_options(
            install,
            session,
            turn.clone(),
            project.root,
            tx,
            rx,
            options,
        ));
        let _abort = AbortTask(task.abort_handle());
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut state = "interrupted".to_string();
        let mut cancel_sent = false;
        loop {
            tokio::select! {
                _=cancel.cancelled(),if !cancel_sent=>{let _=control.send(Control::Cancel).await;cancel_sent=true;},
                _=tick.tick()=>{
                    if let Some(u)=turn.user_id{if !self.authorized(u,false).await?||now()-self.online_at.load(std::sync::atomic::Ordering::Relaxed)>60{cancel.cancel();}}
                    let tid=turn.id.clone();let expired:bool=self.db.call(move|c|{let mut s=c.prepare("SELECT body FROM interactions WHERE turn_id=?1 AND state='pending'")?;for row in s.query_map([tid],|r|r.get::<_,String>(0))?{let r:Interaction=serde_json::from_str(&row?)?;if r.expires_at<now(){return Ok(true)}}Ok(false)}).await?;if expired{cancel.cancel();}
                },
                event=events.recv()=>{let Some(event)=event else{break};match event{
                    NativeEvent::Session(native)=>{let mut s=self.session(&turn.session_id).await?;s.native_id=Some(native);let sid=s.id.clone();let body=serde_json::to_string(&s)?;self.db.call(move|c|{c.execute("UPDATE sessions SET body=?2 WHERE id=?1",params![sid,body])?;Ok(())}).await?},
                    NativeEvent::Event(kind,data)=>{self.publish(&turn,kind,data).await?;},
                    NativeEvent::Request{native_id,kind,data,local_only}=>{let req=Interaction{id:id(),turn_id:turn.id.clone(),session_id:turn.session_id.clone(),native_id,kind:kind.clone(),data,expires_at:now()+if kind=="question"{900}else{300},state:"pending".into(),local_only};let r=req.clone();self.db.call(move|c|{let duplicate:i64=c.query_row("SELECT COUNT(*) FROM interactions WHERE turn_id=?1 AND state='pending' AND json_extract(body,'$.native_id')=json_extract(?2,'$.native_id')",params![r.turn_id,serde_json::to_string(&r)?],|row|row.get(0))?;if duplicate>0{bail!("Повторный native request ID; turn остановлен")};c.execute("INSERT INTO interactions VALUES(?1,?2,'pending',?3)",params![r.id,r.turn_id,serde_json::to_string(&r)?])?;Ok(())}).await?;self.publish(&turn,if kind=="question"{EventKind::UserQuestion}else{EventKind::ApprovalRequest},serde_json::to_value(req)?).await?;},
                    NativeEvent::Finished(s)=>state=s,
                }}
            }
        }
        match (&mut task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                self.publish(&turn, EventKind::Error, json!({"text":e.to_string()}))
                    .await?;
                state = if cancel_sent { "cancelled" } else { "unknown" }.into()
            }
            Err(_) => state = "unknown".into(),
        };
        self.finish(&turn, &state).await
    }
    pub async fn publish(&self, turn: &Turn, kind: EventKind, data: Value) -> Result<Event> {
        let mut event = Event {
            event_id: id(),
            session_id: turn.session_id.clone(),
            turn_id: turn.id.clone(),
            sequence: 0,
            kind,
            data,
            occurred_at: now(),
        };
        let e = event.clone();
        event.sequence = self
            .db
            .call(move |c| {
                c.execute(
                    "INSERT INTO events(session_id,turn_id,body) VALUES(?1,?2,?3)",
                    params![e.session_id, e.turn_id, serde_json::to_string(&e)?],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await?;
        let _ = self.events.send(event.clone());
        Ok(event)
    }
    async fn finish(&self, turn: &Turn, state: &str) -> Result<()> {
        let (t, s) = (turn.id.clone(), state.to_string());
        let changed = self.db
            .call(move |c| {
                let tx = c.transaction()?;
                let changed=tx.execute("UPDATE turns SET state=?2 WHERE id=?1 AND state NOT IN ('merged','pushed')", params![t, s])?;
                tx.execute(
                    "UPDATE interactions SET state='obsolete' WHERE turn_id=?1 AND state='pending'",
                    [t],
                )?;
                tx.commit()?;
                Ok(changed>0)
            })
            .await?;
        if !changed { return Ok(()) }
        self.publish(turn, EventKind::Completed, json!({"state":state}))
            .await?;
        Ok(())
    }
    pub async fn answer(
        &self,
        request_id: String,
        decision: String,
        answers: Value,
        remote: Option<i64>,
    ) -> Result<()> {
        if !["allow", "deny"].contains(&decision.as_str()) {
            bail!("Неверное решение")
        }
        if let Some(u) = remote {
            if !self.authorized(u, true).await? {
                bail!("Доступ истёк")
            }
        }
        let rid = request_id.clone();
        let request: Interaction = self
            .db
            .call(move |c| {
                let body: String = c.query_row(
                    "SELECT body FROM interactions WHERE id=?1 AND state='pending'",
                    [rid],
                    |r| r.get(0),
                )?;
                Ok(serde_json::from_str(&body)?)
            })
            .await?;
        if request.expires_at < now() || remote.is_some() && request.local_only {
            bail!("Запрос истёк или требует ответа на ПК")
        }
        if let Some(user) = remote {
            let tid = request.turn_id.clone();
            let owner = self
                .db
                .call(move |c| {
                    let body: String =
                        c.query_row("SELECT body FROM turns WHERE id=?1", [tid], |r| r.get(0))?;
                    Ok(serde_json::from_str::<Turn>(&body)?.user_id)
                })
                .await?;
            if owner != Some(user) {
                bail!("Запрос принадлежит локальной задаче")
            }
        }
        let map = self.active.lock().await;
        let a = map
            .get(&request.turn_id)
            .ok_or_else(|| anyhow!("Turn уже завершён"))?;
        let permit = a.control.reserve().await?;
        let rid = request_id.clone();
        let decision_db = decision.clone();
        let consumed = self
            .db
            .call(move |c| {
                Ok(c.execute(
                    "UPDATE interactions SET state=?2 WHERE id=?1 AND state='pending'",
                    params![rid, decision_db],
                )? == 1)
            })
            .await?;
        if !consumed {
            bail!("Запрос уже обработан")
        }
        permit.send(Control::Respond {
            request,
            decision,
            answers,
        });
        drop(map);
        self.db.audit("interaction_answered", &request_id).await
    }
    pub async fn history(&self, sid: String, after: i64) -> Result<Vec<Event>> {
        self.db.call(move|c|{let mut s=c.prepare("SELECT seq,body FROM events WHERE session_id=?1 AND seq>?2 ORDER BY seq LIMIT 1000")?;let rows=s.query_map(params![sid,after],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.collect::<Result<Vec<_>,_>>()?;let mut result=vec![];for(seq,body)in rows{let mut e:Event=serde_json::from_str(&body)?;e.sequence=seq;result.push(e)}Ok(result)}).await
    }
    pub async fn audit_rows(&self) -> Result<Vec<Value>> {
        self.db.call(|c|{let mut q=c.prepare("SELECT at,action,target FROM audit ORDER BY seq DESC LIMIT 200")?;let rows=q.query_map([],|r|Ok(json!({"at":r.get::<_,i64>(0)?,"action":r.get::<_,String>(1)?,"target":r.get::<_,String>(2)?})))?.collect::<Result<Vec<_>,_>>()?;Ok(rows)}).await
    }
    pub async fn stage(&self, sid: String, name: String, bytes: Vec<u8>) -> Result<Asset> {
        use sha2::{Digest, Sha256};
        if bytes.len() > 20 * 1024 * 1024 {
            bail!("Лимит вложения 20 MiB")
        };
        let session = self.session(&sid).await?;
        let root = self
            .config
            .read()
            .await
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .ok_or_else(|| anyhow!("Проект отсутствует"))?
            .root
            .clone();
        security::reject_links(&root)?;
        let base = security::migrate_directory(&root, ".agent-hub", ".tab")?;
        if base.exists() {
            security::reject_links(&base)?
        } else {
            std::fs::create_dir(&base)?;
            security::reject_links(&base)?
        };
        let dir = base.join("inbox");
        if dir.exists() {
            security::reject_links(&dir)?
        } else {
            std::fs::create_dir(&dir)?;
            security::reject_links(&dir)?
        };
        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "image/webp"
        } else if bytes.starts_with(b"OggS") {
            "audio/ogg"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
            "audio/wav"
        } else if bytes.starts_with(b"fLaC") {
            "audio/flac"
        } else if bytes.starts_with(b"ID3") {
            "audio/mpeg"
        } else {
            "application/octet-stream"
        };
        if mime.starts_with("image/") {
            let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()?
                .into_dimensions()?;
            if w == 0 || h == 0 || w > 32768 || h > 32768 || w as u64 * h as u64 > 40_000_000 {
                bail!("Изображение превышает лимит 40 MP / 32768 px")
            }
        }
        let aid = id();
        let extension = std::path::Path::new(&name).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        let path = if ["ogg","oga","mp3","mp4","m4a","wav","flac","webm","mov"].contains(&extension.as_str()) { dir.join(format!("{aid}.{extension}")) } else { dir.join(&aid) };
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        use std::io::Write;
        file.write_all(&bytes)?;
        let asset = Asset {
            id: aid,
            name: name.chars().filter(|c| !c.is_control()).take(160).collect(),
            path,
            mime: mime.into(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        let a = asset.clone();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO assets VALUES(?1,?2,?3)",
                    params![a.id, serde_json::to_string(&a)?, now()],
                )?;
                Ok(())
            })
            .await?;
        Ok(asset)
    }
    pub async fn output_files(&self, sid: &str) -> Result<Vec<Asset>> {
        use sha2::{Digest, Sha256};
        let session = self.session(sid).await?;
        let root = self
            .config
            .read()
            .await
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .ok_or_else(|| anyhow!("Проект отсутствует"))?
            .root
            .clone();
        let out = security::migrate_directory(&root, ".agent-hub", ".tab")?.join("outbox");
        if !out.exists() {
            return Ok(vec![]);
        }
        security::reject_links(&out)?;
        let mut result = vec![];
        for entry in std::fs::read_dir(&out)?.take(100) {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            let bytes = security::read_safe_file(&path, &out, 50 * 1024 * 1024)?;
            let a = Asset {
                id: id(),
                name: entry.file_name().to_string_lossy().into(),
                path,
                mime: "application/octet-stream".into(),
                bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            };
            let value = a.clone();
            self.db
                .call(move |c| {
                    c.execute(
                        "INSERT INTO assets VALUES(?1,?2,?3)",
                        params![value.id, serde_json::to_string(&value)?, now()],
                    )?;
                    Ok(())
                })
                .await?;
            result.push(a)
        }
        Ok(result)
    }
}

struct AbortTask(tokio::task::AbortHandle);
#[cfg(test)]
mod pairing_tests {
    use super::*;

    #[tokio::test]
    async fn pairing_snapshot_stays_hidden_until_explicit_request() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::open(dir.path().join("data")).await.unwrap();
        hub.config.write().await.telegram_user = Some(42);
        *hub.telegram_stop.lock().await = Some(CancellationToken::new());
        let initial = hub.snapshot().await.unwrap();
        assert!(initial.telegram_active);
        assert!(!initial.telegram_authorized);
        assert!(initial.pairing_code.is_none());
        hub.issue_pairing_code().await.unwrap();
        let code = hub.snapshot().await.unwrap().pairing_code.unwrap().code;
        assert!(hub.unlock(42, &code, now()).await.unwrap());
        let paired = hub.snapshot().await.unwrap();
        assert!(paired.telegram_authorized);
        assert!(paired.pairing_code.is_none());
    }

    async fn configured(path: PathBuf) -> Arc<Hub> {
        let hub = Hub::open(path).await.unwrap();
        let mut config = hub.config.write().await;
        config.telegram_user = Some(42);
        hub.persist_config(&config).await.unwrap();
        drop(config);
        hub.issue_pairing_code().await.unwrap();
        hub
    }

    #[tokio::test]
    async fn pairing_is_random_shaped_local_and_consumed_once() {
        let dir = tempfile::tempdir().unwrap();
        let hub = configured(dir.path().join("data")).await;
        let code = hub.pairing_snapshot().await.unwrap().code;
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
        let (a,b) = tokio::join!(hub.unlock(42, &code, now()), hub.unlock(42, &code, now()));
        assert_ne!(a.unwrap(), b.unwrap());
        assert!(hub.pairing_snapshot().await.is_none());
        assert!(hub.authorized(42, false).await.unwrap());
        let config = hub.db.setting("config").await.unwrap().unwrap();
        assert!(!config.contains("pin_hash"));
        assert!(!config.contains("pairing_code"));
        let outbox: i64 = hub.db.call(|db| Ok(db.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?)).await.unwrap();
        assert_eq!(outbox, 0);
    }

    #[tokio::test]
    async fn pairing_rejects_other_user_stale_message_expiry_and_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let hub = configured(dir.path().join("data")).await;
        let code = hub.pairing_snapshot().await.unwrap().code;
        assert!(!hub.unlock(7, &code, now()).await.unwrap());
        assert!(!hub.unlock(42, &code, now()-60).await.unwrap());
        hub.pairing.lock().await.as_mut().unwrap().expires_at = now()-1;
        assert!(!hub.unlock(42, &code, now()).await.unwrap());
        assert!(hub.pairing_snapshot().await.is_none());
        hub.issue_pairing_code().await.unwrap();
        let code = hub.pairing_snapshot().await.unwrap().code;
        hub.revoke().await.unwrap();
        assert!(!hub.unlock(42, &code, now()).await.unwrap());
        assert!(!hub.authorized(42, false).await.unwrap());
    }

    #[tokio::test]
    async fn pairing_lockout_survives_new_code_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        let hub = configured(path.clone()).await;
        for _ in 0..5 { assert!(!hub.unlock(42, "invalid", now()).await.unwrap()); }
        hub.issue_pairing_code().await.unwrap();
        let code = hub.pairing_snapshot().await.unwrap().code;
        assert!(!hub.unlock(42, &code, now()).await.unwrap());
        drop(hub);
        let hub = Hub::open(path).await.unwrap();
        assert!(hub.pairing_snapshot().await.is_none());
        hub.issue_pairing_code().await.unwrap();
        let code = hub.pairing_snapshot().await.unwrap().code;
        assert!(!hub.unlock(42, &code, now()).await.unwrap());
    }
}
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
