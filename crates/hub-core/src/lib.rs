pub mod adapter;
pub mod domain;
pub mod process;
pub mod security;
pub mod store;
pub mod telegram;

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
}
impl Hub {
    pub async fn open(data_dir: PathBuf) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)?;
        let db = Store::open(&data_dir.join("hub.db"))?;
        let mut config: Config = match db.setting("config").await? {
            Some(s) => serde_json::from_str(&s)?,
            None => Config::default(),
        };
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
        }))
    }
    async fn persist_config(&self, c: &Config) -> Result<()> {
        self.db.put("config", serde_json::to_string(c)?).await
    }
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let mut config = self.config.read().await.clone();
        config.pin_hash = config.pin_hash.map(|_| "configured".into());
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
        Ok(Snapshot {
            config,
            sessions,
            turns,
            requests,
            telegram_status: self.telegram_status.read().await.clone(),
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
        mut pin: String,
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
        let pepper = security::secret_get(&format!("{machine}/pepper"))?
            .unwrap_or_else(security::random_secret);
        let p = pin.clone();
        let pe = pepper.clone();
        let hash = tokio::task::spawn_blocking(move || security::hash_pin(&p, &pe)).await??;
        security::secret_set(&format!("{machine}/telegram"), &token)?;
        security::secret_set(&format!("{machine}/pepper"), &pepper)?;
        use zeroize::Zeroize;
        token.zeroize();
        pin.zeroize();
        let mut c = self.config.write().await;
        c.telegram_user = Some(user_id);
        c.pin_hash = Some(hash);
        c.auth_epoch = id();
        self.persist_config(&c).await?;
        drop(c);
        self.revoke().await?;
        self.db
            .audit("telegram_configured", &user_id.to_string())
            .await
    }
    pub async fn unlock(&self, user: i64, pin: &str) -> Result<bool> {
        let c = self.config.read().await.clone();
        if c.telegram_user != Some(user) {
            return Ok(false);
        }
        let Some(hash) = c.pin_hash else {
            return Ok(false);
        };
        let pepper = security::secret_get(&format!("{}/pepper", c.machine_id))?
            .ok_or_else(|| anyhow!("PIN pepper отсутствует"))?;
        let pin = pin.to_string();
        self.db.call(move|db|{
            let ts=now();let tx=db.transaction()?;
            let(failures,window,blocked,strikes)=tx.query_row("SELECT failures,window_at,blocked_until,strikes FROM attempts WHERE user_id=?1",[user],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).optional()?.unwrap_or((0,ts,0,0));
            if blocked>ts{return Ok(false)}
            if security::verify_pin(&pin,&pepper,&hash){tx.execute("INSERT OR REPLACE INTO grants VALUES(?1,?2,?3,?4)",params![user,c.auth_epoch,ts,ts+8*3600])?;tx.execute("DELETE FROM attempts WHERE user_id=?1",[user])?;tx.commit()?;return Ok(true)}
            let failures=if ts-window>900{1}else{failures+1};let window=if ts-window>900{ts}else{window};let strikes=if failures>=5{strikes+1}else{strikes};let blocked=if failures>=5{ts+(900*(1i64<<(strikes-1).min(7))).min(86400)}else{0};
            tx.execute("INSERT OR REPLACE INTO attempts VALUES(?1,?2,?3,?4,?5)",params![user,failures,window,blocked,strikes])?;tx.commit()?;Ok(false)
        }).await
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
        Ok(())
    }
    pub async fn steer(&self, turn_id: &str, text: String) -> Result<()> {
        let map = self.active.lock().await;
        let a = map.get(turn_id).ok_or_else(|| anyhow!("Turn не активен"))?;
        a.control.send(Control::Steer(text)).await?;
        Ok(())
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
        let _permits = tokio::select! {p=permits=>p?,_=cancel.cancelled()=>{self.finish(&turn,"cancelled").await?;return Ok(())}};
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
        let mut task = tokio::spawn(adapter::run(
            install,
            session,
            turn.clone(),
            project.root,
            tx,
            rx,
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
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute("UPDATE turns SET state=?2 WHERE id=?1", params![t, s])?;
                tx.execute(
                    "UPDATE interactions SET state='obsolete' WHERE turn_id=?1 AND state='pending'",
                    [t],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
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
        let base = root.join(".agent-hub");
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
        let path = dir.join(&aid);
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
        let out = root.join(".agent-hub/outbox");
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
    pub async fn export_log(&self, sid: String) -> Result<PathBuf> {
        let dir = self.data_dir.join("exports");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.jsonl", id()));
        let mut f = std::fs::File::create(&path)?;
        use std::io::Write;
        let mut cursor = 0;
        loop {
            let page = self.history(sid.clone(), cursor).await?;
            if page.is_empty() {
                break;
            }
            for e in page {
                cursor = e.sequence;
                writeln!(f, "{}", serde_json::to_string(&e)?)?
            }
        }
        Ok(path)
    }
}

struct AbortTask(tokio::task::AbortHandle);
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
