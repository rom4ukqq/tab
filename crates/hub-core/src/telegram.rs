use crate::{domain::*, security, Hub};
use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Client {
    token: Arc<String>,
    http: reqwest::Client,
}
#[derive(Debug)]
struct ApiError {
    code: i64,
    retry: i64,
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Telegram API {}", self.code)
    }
}
impl std::error::Error for ApiError {}
impl Client {
    pub fn new(token: String) -> Self {
        Self {
            token: Arc::new(token),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(40))
                .build()
                .expect("HTTP client"),
        }
    }
    async fn call(&self, method: &str, body: &Value) -> Result<Value> {
        let response = self
            .http
            .post(format!(
                "https://api.telegram.org/bot{}/{method}",
                self.token
            ))
            .json(body)
            .send()
            .await
            .map_err(|_| anyhow!("Telegram: network outcome unknown"))?;
        let value: Value = response
            .json()
            .await
            .map_err(|_| anyhow!("Telegram: invalid response"))?;
        if value["ok"] != true {
            return Err(ApiError {
                code: value["error_code"].as_i64().unwrap_or(500),
                retry: value["parameters"]["retry_after"].as_i64().unwrap_or(0),
            }
            .into());
        }
        Ok(value["result"].clone())
    }
    pub async fn check(&self) -> Result<()> {
        self.call("getMe", &json!({})).await?;
        let hook = self.call("getWebhookInfo", &json!({})).await?;
        if !hook["url"].as_str().unwrap_or("").is_empty() {
            bail!("У бота настроен webhook. Остановите прежний connector перед подключением Hub.")
        };
        Ok(())
    }
    async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let f = self.call("getFile", &json!({"file_id":file_id})).await?;
        if f["file_size"].as_u64().unwrap_or(0) > 20 * 1024 * 1024 {
            bail!("Лимит скачивания 20 MiB")
        }
        let path = f["file_path"]
            .as_str()
            .ok_or_else(|| anyhow!("Telegram не вернул файл"))?;
        let response = self
            .http
            .get(format!(
                "https://api.telegram.org/file/bot{}/{path}",
                self.token
            ))
            .send()
            .await
            .map_err(|_| anyhow!("Не удалось скачать вложение"))?;
        if !response.status().is_success() {
            bail!("Не удалось скачать вложение")
        };
        let mut stream = response.bytes_stream();
        let mut data = vec![];
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| anyhow!("Скачивание прервано"))?;
            if data.len() + chunk.len() > 20 * 1024 * 1024 {
                bail!("Лимит скачивания 20 MiB")
            };
            data.extend_from_slice(&chunk)
        }
        Ok(data)
    }
    pub async fn document(&self, chat: i64, path: &std::path::Path) -> Result<Value> {
        // Callers resolve the path against a local export root before invoking this method.
        let bytes = tokio::fs::read(path).await?;
        if bytes.len() > 50 * 1024 * 1024 {
            bail!("Лимит отправки 50 MiB")
        }
        let part = reqwest::multipart::Part::bytes(bytes).file_name(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        );
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat.to_string())
            .part("document", part);
        let response = self
            .http
            .post(format!(
                "https://api.telegram.org/bot{}/sendDocument",
                self.token
            ))
            .multipart(form)
            .send()
            .await
            .map_err(|_| anyhow!("Document delivery outcome unknown"))?;
        let v: Value = response
            .json()
            .await
            .map_err(|_| anyhow!("Invalid document response"))?;
        if v["ok"] != true {
            bail!("Telegram отклонил document")
        };
        Ok(v["result"].clone())
    }
    async fn asset(&self, hub: &Hub, chat: i64, aid: &str) -> Result<()> {
        use sha2::{Digest, Sha256};
        let aid = aid.to_string();
        let a: Asset = hub
            .db
            .call(move |c| {
                let body: String =
                    c.query_row("SELECT body FROM assets WHERE id=?1", [aid], |r| r.get(0))?;
                Ok(serde_json::from_str(&body)?)
            })
            .await?;
        let projects = hub.config.read().await.projects.clone();
        let root = projects
            .iter()
            .map(|p| p.root.join(".tab/outbox"))
            .find(|root| a.path.starts_with(root))
            .ok_or_else(|| anyhow!("Экспорт разрешён только из outbox"))?;
        let bytes = security::read_safe_file(&a.path, &root, 50 * 1024 * 1024)?;
        if format!("{:x}", Sha256::digest(&bytes)) != a.sha256 {
            bail!("Файл изменился; обновите /files")
        }
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat.to_string())
            .part(
                "document",
                reqwest::multipart::Part::bytes(bytes).file_name(a.name),
            );
        let response = self
            .http
            .post(format!(
                "https://api.telegram.org/bot{}/sendDocument",
                self.token
            ))
            .multipart(form)
            .send()
            .await
            .map_err(|_| anyhow!("Исход отправки файла неизвестен; автоматического повтора нет"))?;
        let value: Value = response
            .json()
            .await
            .map_err(|_| anyhow!("Invalid response"))?;
        if value["ok"] != true {
            bail!("Отправка файла отклонена")
        };
        hub.db.audit("artifact_exported", &a.id).await?;
        Ok(())
    }
}
pub async fn start(hub: Arc<Hub>) -> Result<()> {
    let mut stop = hub.telegram_stop.lock().await;
    if stop.is_some() {
        return Ok(());
    }
    let c = hub.config.read().await.clone();
    let token = security::secret_get(&format!("{}/telegram", c.machine_id))?
        .ok_or_else(|| anyhow!("Сначала настройте Telegram"))?;
    let client = Client::new(token);
    hub.revoke().await?;
    hub.db.put("telegram_autoconnect", "true".into()).await?;
    let cancel = CancellationToken::new();
    *stop = Some(cancel.clone());
    drop(stop);
    *hub.telegram_status.write().await = "Подключается".into();
    tokio::spawn(async move {
        let result = async {
            loop {
                let check = tokio::select! { _=cancel.cancelled()=>return Ok(()), r=client.check()=>r };
                match check {
                    Ok(()) => break,
                    Err(e) if retry_connection(&e) => {
                        *hub.telegram_status.write().await = "Сеть недоступна; переподключение…".into();
                        let delay = e.downcast_ref::<ApiError>().map(|e| e.retry.max(5)).unwrap_or(5) as u64;
                        tokio::select! { _=cancel.cancelled()=>return Ok(()), _=tokio::time::sleep(Duration::from_secs(delay))=>{} }
                    }
                    Err(e) => return Err(e),
                }
            }
            tokio::select! {r=poll(hub.clone(),client.clone(),cancel.clone())=>r,r=deliver(hub.clone(),client,cancel.clone())=>r}
        }.await;
        cancel.cancel();
        hub.revoke().await.ok();
        *hub.telegram_status.write().await = match result {
            Ok(()) => "Отключён".into(),
            Err(e) => e.to_string(),
        };
        *hub.telegram_stop.lock().await = None;
    });
    Ok(())
}
fn retry_connection(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>().is_some_and(|e| e.code == 429 || e.code >= 500)
        || matches!(e.to_string().as_str(), "Telegram: network outcome unknown" | "Telegram: invalid response")
}
pub async fn restore(hub: Arc<Hub>) -> Result<()> {
    let configured = {
        let c = hub.config.read().await;
        c.telegram_user.is_some()
    };
    // Existing configured installations opt in; explicit Disconnect survives restarts.
    let enabled = hub.db.setting("telegram_autoconnect").await?.map(|s| s == "true").unwrap_or(configured);
    if configured && enabled { start(hub).await?; }
    Ok(())
}
pub async fn disconnect(hub: &Hub) -> Result<()> {
    hub.db.put("telegram_autoconnect", "false".into()).await?;
    stop(hub).await
}
pub async fn stop(hub: &Hub) -> Result<()> {
    if let Some(t) = hub.telegram_stop.lock().await.as_ref() {
        t.cancel()
    };
    hub.revoke().await?;
    *hub.telegram_status.write().await = "Отключается".into();
    Ok(())
}
async fn enqueue(hub: &Hub, method: &str, body: Value, key: Option<String>) -> Result<()> {
    let method = method.to_string();
    hub.db
        .call(move |c| {
            c.execute(
                "INSERT INTO outbox(id,method,body,route_key) VALUES(?1,?2,?3,?4)",
                params![id(), method, body.to_string(), key],
            )?;
            Ok(())
        })
        .await
}
async fn say(hub: &Hub, chat: i64, text: &str, buttons: Vec<Vec<Value>>) -> Result<()> {
    enqueue(
        hub,
        "sendMessage",
        json!({"chat_id":chat,"text":clip(text,4000),"reply_markup":{"inline_keyboard":buttons}}),
        None,
    )
    .await
}
fn button(text: &str, data: String) -> Value {
    json!({"text":clip(text,60),"callback_data":data})
}
fn clip(s: &str, max: usize) -> String {
    let mut units = 0;
    s.chars().take_while(|c| { units += c.len_utf16(); units <= max }).collect()
}
fn session_buttons(sid: &str) -> Vec<Vec<Value>> {
    let mut end_session = button("Закончить сессию", format!("e:{sid}"));
    end_session["style"] = json!("danger");
    vec![vec![button("Очередь", format!("q:{sid}")), button("Статус", format!("i:{sid}"))], vec![button("Остановить задачу", format!("t:{sid}")), end_session]]
}

fn telemetry_label(data: &Value) -> String {
    let effort = data["effort"].as_str().unwrap_or("по настройкам CLI");
    let context = match (data["tokenUsage"]["last"]["totalTokens"].as_f64(), data["tokenUsage"]["modelContextWindow"].as_f64()) {
        (Some(used), Some(total)) if total > 0.0 => format!("{:.0}%", (100.0 - used / total * 100.0).clamp(0.0, 100.0)),
        _ => "нет данных".into(),
    };
    let mut limits = Vec::new();
    let mut add = |name: &str, snapshot: &Value| {
        for key in ["primary", "secondary"] {
            let w = &snapshot[key];
            if let Some(used) = w["usedPercent"].as_f64() {
                let window = w["windowDurationMins"].as_i64().map(|m| if m % 1440 == 0 { format!("{}д", m / 1440) } else if m % 60 == 0 { format!("{}ч", m / 60) } else { format!("{m}мин") }).unwrap_or_else(|| key.into());
                limits.push(format!("{name} {window}: {:.0}%", (100.0 - used).clamp(0.0, 100.0)));
            }
        }
    };
    if let Some(buckets) = data["rateLimitsByLimitId"].as_object().filter(|b| !b.is_empty()) {
        for (name, value) in buckets { add(name, value); }
    } else { add("", &data["rateLimits"]); }
    if limits.is_empty() { if let Some(left)=data["allowanceLeftPercent"].as_f64() { limits.push(format!("{:.0}%", left.clamp(0.0,100.0))); } }
    format!("Effort: {effort} · Доступ: {}\nКонтекст осталось: {context}\nЛимиты осталось: {}", if data["full_access"] == true { "full" } else { "обычный" }, if limits.is_empty() { "нет данных".into() } else { limits.join(" · ") })
}

async fn session_status(hub: &Hub, chat: i64, sid: &str) -> Result<()> {
    let key = format!("telemetry:{sid}");
    let data = hub.db.call(move |db| {
        let raw: Option<String> = db.query_row("SELECT value FROM runtime_settings WHERE key=?1", [key], |r| r.get(0)).optional()?;
        Ok(raw.and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!({})))
    }).await?;
    let options = hub.agent_options(sid).await?;
    let key=format!("activity:{sid}");
    let activity=hub.db.call(move|db|{
        let raw:Option<String>=db.query_row("SELECT value FROM runtime_settings WHERE key=?1",[key],|r|r.get(0)).optional()?;
        Ok(raw.and_then(|s|serde_json::from_str::<crate::telegram_activity::Activity>(&s).ok()))
    }).await?;
    let activity=activity.map(|a| {
        let terminal=["Готово","Остановлено","Ошибка","Результат не подтверждён"].contains(&a.phase.as_str());
        a.label(if terminal {a.updated_at} else {now()})
    }).unwrap_or_else(||"Нет запущенной задачи".into());
    say(hub, chat, &format!("TAB · {activity}\n{}\n\nСледующая задача: effort {}, доступ {}\n/effort LEVEL · /access full · /access normal", telemetry_label(&data), options.effort.as_deref().unwrap_or("default"), if options.full_access { "full" } else { "обычный" }), session_buttons(sid)).await
}

async fn queue_turns(hub: &Hub, sid: &str, chat: i64, user: i64) -> Result<Vec<Turn>> {
    let sid = sid.to_string();
    hub.db.call(move |db| {
        let mut stmt = db.prepare("SELECT body,state FROM turns WHERE session_id=?1 AND state IN ('queued','running','dispatching','steering','push_unknown') ORDER BY CASE WHEN state='running' THEN 0 ELSE 1 END,COALESCE((SELECT CAST(value AS INTEGER) FROM runtime_settings WHERE key='queue:'||turns.id),rowid),rowid")?;
        let rows = stmt.query_map([sid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        let mut turns = Vec::new();
        for (body, state) in rows { let mut t: Turn = serde_json::from_str(&body)?; t.state = state; if t.chat_id == Some(chat) && t.user_id == Some(user) { turns.push(t); } }
        Ok(turns)
    }).await
}

async fn show_queue(hub: &Hub, chat: i64, user: i64, sid: &str) -> Result<()> {
    let turns = queue_turns(hub, sid, chat, user).await?;
    let mut text = String::from("TAB · очередь\n");
    let mut buttons = Vec::new();
    for (i, t) in turns.iter().enumerate() {
        let state = match t.state.as_str() { "running" => "в работе", "steering" => "ожидает подтверждения push", "push_unknown" => "push: исход неизвестен", _ => "ожидает" };
        text.push_str(&format!("\n{}. {} · {}{}", i + 1, state, clip(&t.text.replace('\n', " "), 140), if t.assets.is_empty() { String::new() } else { format!(" · вложений {}", t.assets.len()) }));
        let mut row = vec![button(&format!("Отменить {}", i + 1), format!("qc:{}", t.id))];
        if t.state == "queued" && t.assets.is_empty() { row.push(button(&format!("Push {}", i + 1), format!("qp:{}", t.id))); }
        buttons.push(row);
        if t.state == "queued" {
            buttons.push(vec![button(&format!("Изменить {}",i+1),format!("qe:{}",t.id)),button("↑ Выше",format!("qu:{}",t.id)),button("Объединить со следующим",format!("qm:{}",t.id))]);
        }
    }
    if turns.is_empty() { text.push_str("Пусто."); }
    text.push_str("\n\nНовые сообщения добавляются в очередь.\n/push ТЕКСТ — передать уточнение в текущую задачу (Codex/OMP).");
    buttons.extend(session_buttons(sid));
    say(hub, chat, &text, buttons).await
}

async fn session_intro(hub: &Hub, chat: i64, session: &Session, resumed: bool) -> Result<()> {
    let text = {
        let config = hub.config.read().await;
        let installation = config.installations.iter().find(|i| i.provider == session.provider);
        let project = config.projects.iter().find(|p| p.id == session.project_id);
        let agent = match session.provider {
            Provider::Codex => "OpenAI Codex",
            Provider::Claude => "Claude Code",
            Provider::Omp => "Oh My Pi",
            Provider::Mock => "Mock",
        };
        let version = installation.map(|i| i.version.as_str()).filter(|v| !v.trim().is_empty()).unwrap_or("не определена");
        let directory = project.map(|p| p.root.display().to_string()).unwrap_or_else(|| "недоступна".into());
        let model = session.model.as_deref().unwrap_or("по настройкам CLI (определится при запуске)");
        let permissions = match session.provider {
            Provider::Codex => "Sandbox: workspace-write\nПодтверждения: on-request",
            Provider::Claude => "Подтверждения: manual\nOS sandbox: отсутствует",
            Provider::Omp => "Подтверждения: always-ask (локально на ПК)\nOS sandbox: не подтверждён",
            Provider::Mock => "Тестовый агент: команды не выполняются",
        };
        format!(
            "{agent} · {}\nВерсия CLI: {version}\nМодель: {model}\nРабочая папка: {directory}\n{permissions}\n\nCLI запустится при отправке задачи.\nНапишите задачу или прикрепите файлы.\n/stop — остановить задачу\n/start — выбрать агента\n/help — справка",
            if resumed { "диалог продолжен" } else { "новая сессия" },
        )
    };
    say(hub, chat, &text, session_buttons(&session.id)).await
}
async fn stop_session(hub: &Hub, chat: i64, user: i64, sid: &str, end: bool) -> Result<bool> {
    let sid = sid.to_string();
    let turns = hub.db.call(move |db| {
        let tx = db.transaction()?;
        let current: Option<String> = tx.query_row("SELECT session_id FROM routes WHERE chat_id=?1", [chat], |r| r.get(0)).optional()?.flatten();
        if current.as_deref() != Some(&sid) { return Ok(None); }
        if end { tx.execute("UPDATE routes SET session_id=NULL WHERE chat_id=?1 AND session_id=?2", params![chat, sid])?; tx.execute("INSERT OR IGNORE INTO closed_sessions VALUES(?1)", [&sid])?; }
        let turns = {
            let mut stmt = tx.prepare("SELECT body FROM turns WHERE state IN ('queued','dispatching','running')")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            let mut ids = Vec::new();
            for row in rows {
                let t: Turn = serde_json::from_str(&row)?;
                if t.session_id == sid && t.chat_id == Some(chat) && t.user_id == Some(user) { ids.push(t.id); }
            }
            ids
        };
        tx.commit()?;
        Ok(Some(turns))
    }).await?;
    let Some(turns) = turns else { return Ok(false) };
    for tid in turns { hub.cancel(&tid).await?; }
    Ok(true)
}
async fn selected(hub: &Hub, chat: i64) -> Result<String> {
    hub.db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT session_id FROM routes WHERE chat_id=?1",
                [chat],
                |r| r.get::<_, Option<String>>(0),
            )?
            .ok_or_else(|| anyhow!("Выберите агента через /start"))?)
        })
        .await
}
async fn agent_menu(hub: &Hub, chat: i64) -> Result<()> {
    if let Ok(sid) = selected(hub, chat).await {
        return say(hub, chat, "TAB · вы уже общаетесь с агентом. Чтобы выбрать другого, закончите текущую сессию.", session_buttons(&sid)).await;
    }
    let buttons = hub.config.read().await.installations.iter().filter(|i| i.enabled)
        .map(|i| vec![button(i.provider.key(), format!("p:{}", i.provider.key()))]).collect::<Vec<_>>();
    let text = if buttons.is_empty() { "TAB · включите нужного агента на ПК." } else { "TAB · выберите агента." };
    say(hub, chat, text, buttons).await
}
async fn poll(hub: Arc<Hub>, client: Client, stop: CancellationToken) -> Result<()> {
    let mut offset = hub
        .db
        .setting("tg_offset")
        .await?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let mut drafts: HashMap<i64, (String, Vec<Asset>, String, i64)> = HashMap::new();
    while !stop.is_cancelled() {
        let request =
            json!({"offset":offset,"timeout":25,"allowed_updates":["message","callback_query"]});
        let response =
            tokio::select! {_=stop.cancelled()=>break,r=client.call("getUpdates",&request)=>r};
        let updates = match response {
            Ok(v) => v,
            Err(e) => {
                if e.downcast_ref::<ApiError>()
                    .is_some_and(|e| e.code == 409 || e.code == 401)
                {
                    return Err(e);
                }
                *hub.telegram_status.write().await =
                    "Сеть недоступна; повтор через 5 секунд".into();
                tokio::select! {_=stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(5))=>{}};
                continue;
            }
        };
        hub.online_at
            .store(now(), std::sync::atomic::Ordering::Relaxed);
        *hub.telegram_status.write().await = "Подключён · один ПК".into();
        drafts.retain(|_, (_, _, _, at)| *at + 900 > now());
        for u in updates.as_array().into_iter().flatten() {
            let uid = u["update_id"]
                .as_i64()
                .ok_or_else(|| anyhow!("Missing update ID"))?;
            let fresh = hub
                .db
                .call(move |c| {
                    Ok(c.execute(
                        "INSERT OR IGNORE INTO updates VALUES(?1,'processing')",
                        [uid],
                    )? == 1)
                })
                .await?;
            if fresh {
                if let Err(e) = handle(&hub, &client, u, &mut drafts).await {
                    let chat = u["message"]["chat"]["id"]
                        .as_i64()
                        .or(u["callback_query"]["message"]["chat"]["id"].as_i64());
                    let user = u["message"]["from"]["id"]
                        .as_i64()
                        .or(u["callback_query"]["from"]["id"].as_i64());
                    if user == hub.config.read().await.telegram_user {
                        if let Some(chat) = chat {
                            say(&hub,chat,&format!("TAB · действие не выполнено: {}",clip(&e.to_string(),500)),vec![]).await?
                        }
                    }
                }
                hub.db
                    .call(move |c| {
                        c.execute("UPDATE updates SET state='handled' WHERE id=?1", [uid])?;
                        Ok(())
                    })
                    .await?;
            }
            offset = uid + 1;
            hub.db.put("tg_offset", offset.to_string()).await?;
        }
    }
    Ok(())
}
async fn handle(
    hub: &Arc<Hub>,
    client: &Client,
    u: &Value,
    drafts: &mut HashMap<i64, (String, Vec<Asset>, String, i64)>,
) -> Result<()> {
    let cb = &u["callback_query"];
    let msg = if cb.is_object() {
        &cb["message"]
    } else {
        &u["message"]
    };
    let user = if cb.is_object() {
        cb["from"]["id"].as_i64()
    } else {
        msg["from"]["id"].as_i64()
    };
    let Some(user) = user else { return Ok(()) };
    let c = hub.config.read().await.clone();
    if c.telegram_user != Some(user) || msg["chat"]["type"] != "private" {
        return Ok(());
    };
    let chat = msg["chat"]["id"]
        .as_i64()
        .ok_or_else(|| anyhow!("No chat"))?;
    if chat != user {
        return Ok(());
    }
    if cb.is_object() {
        let _ = client
            .call(
                "answerCallbackQuery",
                &json!({"callback_query_id":cb["id"]}),
            )
            .await;
    }
    let text = msg["text"].as_str().unwrap_or("");
    if !cb.is_object() && !hub.authorized(user, false).await?
        && (text.starts_with("/pin ")
            || text.len() >= 4 && text.len() <= 6 && text.bytes().all(|c| c.is_ascii_digit()))
    {
        let pin = text.strip_prefix("/pin ").unwrap_or(text).trim();
        let ok = hub.unlock(user, pin, msg["date"].as_i64().unwrap_or(0)).await?;
        let _ = client
            .call(
                "deleteMessage",
                &json!({"chat_id":chat,"message_id":msg["message_id"]}),
            )
            .await;
        if ok {
            return agent_menu(hub, chat).await;
        }
        return say(hub, chat, "Код неверен, истёк или ввод временно заблокирован. Посмотрите новый PIN в TAB на ПК.", vec![]).await;
    }
    if !cb.is_object() && text == "/lock" {
        drafts.remove(&chat);
        hub.revoke().await?;
        return say(hub, chat, "Доступ закрыт.", vec![]).await;
    }
    if !hub.authorized(user, true).await? {
        return say(hub,chat,&format!("{} · откройте TAB → Telegram на ПК, нажмите Connect и отправьте сюда показанный шестизначный PIN. Код одноразовый и действует 5 минут.",c.machine_name),vec![]).await;
    }
    if cb.is_object() {
        let data = cb["data"].as_str().unwrap_or("");
        if let Some(sid)=data.strip_prefix("of:") {
            if selected(hub,chat).await?!=sid { return say(hub,chat,"Сессия уже закрыта. Выберите текущую через /start.",vec![]).await }
            let files=hub.output_files(sid).await?;
            return say(hub,chat,if files.is_empty(){"В .tab/outbox пока нет файлов. Попросите агента сохранить туда результат."}else{"Файлы в .tab/outbox (могут включать результаты предыдущих задач):"},files.iter().map(|a|vec![button(&a.name,format!("f:{}",a.id))]).collect()).await;
        }
        if let Some(tid)=data.strip_prefix("qe:").or_else(||data.strip_prefix("qu:")).or_else(||data.strip_prefix("qm:")) {
            let sid=selected(hub,chat).await?;
            if data.starts_with("qe:") {
                let turns=queue_turns(hub,&sid,chat,user).await?;
                let Some(turn)=turns.iter().find(|t|t.id==tid && t.state=="queued") else { return say(hub,chat,"Сообщение уже запущено. Обновите /queue.",vec![]).await };
                let key=format!("queue_edit:{chat}");let value=json!({"sid":sid,"tid":tid}).to_string();
                hub.db.call(move|db|{db.execute("INSERT OR REPLACE INTO runtime_settings VALUES(?1,?2)",params![key,value])?;Ok(())}).await?;
                return say(hub,chat,&format!("Отправьте новый текст одним сообщением. Вложения сохранятся. Редактирование возможно, пока задача ожидает.\n/cancel_edit — отменить ввод\n\n{}",clip(&turn.text,2800)),vec![]).await;
            }
            hub.edit_queue(sid.clone(),tid.into(),user,if data.starts_with("qu:"){"up"}else{"merge"}.into(),None).await?;
            return show_queue(hub,chat,user,&sid).await;
        }
        if let Some(sid) = data.strip_prefix("q:").or_else(|| data.strip_prefix("i:")) {
            if selected(hub, chat).await? != sid { return say(hub, chat, "Карточка другого сеанса. Используйте /status или /queue.", vec![]).await; }
            return if data.starts_with("q:") { show_queue(hub, chat, user, sid).await } else { session_status(hub, chat, sid).await };
        }
        if let Some(tid) = data.strip_prefix("qc:") {
            let sid = selected(hub, chat).await?;
            if queue_turns(hub, &sid, chat, user).await?.iter().any(|t| t.id == tid) { hub.cancel(tid).await?; }
            return say(hub, chat, "TAB · отмена запрошена. /queue — обновить очередь.", session_buttons(&sid)).await;
        }
        if let Some(tid) = data.strip_prefix("qp:") {
            let sid = selected(hub, chat).await?;
            let result = hub.push_queued(&sid, tid, chat, user).await?;
            say(hub,chat,&format!("TAB · {result}"),session_buttons(&sid)).await?;
            return show_queue(hub,chat,user,&sid).await;
        }
        if data == "sessions" { return agent_menu(hub, chat).await; }
        if let Some(sid) = data.strip_prefix("e:").or_else(|| data.strip_prefix("t:")) {
            let end = data.starts_with("e:");
            if !stop_session(hub, chat, user, sid, end).await? {
                return say(hub, chat, "TAB · эта карточка относится к другому или уже закрытому сеансу.", vec![vec![button("Выбрать агента", "new".into())]]).await;
            }
            if end {
                if drafts.get(&chat).is_some_and(|d| d.0 == sid) { drafts.remove(&chat); }
                return say(hub, chat, "TAB · сеанс завершён, остановка задач запрошена. TAB не сохраняет переписку.", vec![vec![button("Выбрать агента", "new".into())]]).await;
            }
            return say(hub, chat, "TAB · остановка задачи запрошена. Диалог остаётся открытым.", session_buttons(sid)).await;
        }
        if data == "new" {
            return agent_menu(hub, chat).await;
        }
        if let Some(p) = data.strip_prefix("p:") {
            if selected(hub, chat).await.is_ok() { return agent_menu(hub, chat).await; }
            let p: Provider = serde_json::from_value(json!(p))?;
            let s = hub.create_agent_session(p).await?;
            set_session(hub, chat, &s.id).await?;
            return session_intro(hub, chat, &s, false).await;
        }
        if data.starts_with("j:") {
            return say(hub, chat, "Выбор проекта больше не нужен. Создайте сессию и выберите агента.", vec![vec![button("Новая сессия", "new".into())]]).await;
        }
        if data.starts_with("s:") {
            return agent_menu(hub, chat).await;
        }
        if let Some(rid) = data.strip_prefix("a:") {
            return hub
                .answer(rid.into(), "allow".into(), json!({}), Some(user))
                .await;
        }
        if let Some(rid) = data.strip_prefix("d:") {
            return hub
                .answer(rid.into(), "deny".into(), json!({}), Some(user))
                .await;
        }
        if let Some(tid) = data.strip_prefix("x:") {
            return hub.cancel(tid).await;
        }
        if let Some(aid) = data.strip_prefix("f:") {
            return client.asset(hub, chat, aid).await;
        }
        if data.starts_with("l:") {
            return say(hub, chat, "TAB больше не хранит журнал переписки.", vec![]).await;
        }
        if data == "draft" {
            let (sid, assets, text, _) = drafts
                .get(&chat).cloned()
                .ok_or_else(|| anyhow!("Draft expired"))?;
            hub.submit(sid.clone(), text, assets, Some((chat, user))).await?;
            drafts.remove(&chat);
            return show_queue(hub, chat, user, &sid).await;
        }
        return Ok(());
    }
    let edit_key=format!("queue_edit:{chat}");
    let lookup=edit_key.clone();
    let editing:Option<String>=hub.db.call(move|db|Ok(db.query_row("SELECT value FROM runtime_settings WHERE key=?1",[lookup],|r|r.get(0)).optional()?)).await?;
    if text=="/cancel_edit" || (editing.is_some() && !text.trim().is_empty() && !text.starts_with('/')) {
        hub.db.call(move|db|{db.execute("DELETE FROM runtime_settings WHERE key=?1",[edit_key])?;Ok(())}).await?;
        if text=="/cancel_edit" { return say(hub,chat,"Редактирование отменено.",vec![]).await }
        let edit:Value=serde_json::from_str(&editing.unwrap())?;
        let sid=selected(hub,chat).await?;
        if edit["sid"]!=sid { return say(hub,chat,"Сессия изменилась. Нажмите «Изменить» в текущей очереди.",vec![]).await }
        hub.edit_queue(sid.clone(),edit["tid"].as_str().unwrap_or("").into(),user,"edit".into(),Some(text.into())).await?;
        return show_queue(hub,chat,user,&sid).await;
    }
    match text{
        "/start"|"/new"|"/sessions"=>return agent_menu(hub,chat).await,
        "/pcs"=>return say(hub,chat,&format!("{} · {}",c.machine_name,hub.telegram_status.read().await),vec![]).await,
        "/status"|"/limits"|"/effort"|"/access"=>{let sid=selected(hub,chat).await?;return session_status(hub,chat,&sid).await},
        "/queue"=>{let sid=selected(hub,chat).await?;return show_queue(hub,chat,user,&sid).await},
        "/help"=>return say(hub,chat,"/start — выбрать агента\n/stop — остановить задачи сессии\n/lock — отозвать доступ\n/status, /limits — effort, контекст и лимиты\n/effort LEVEL — effort со следующей задачи (default — настройки CLI)\n/access full или /access normal — доступ со следующей задачи\n/queue — очередь, отмена и Push ожидающего текста\n/push ТЕКСТ — уточнение в текущую задачу (Codex/OMP)\n/answer ID JSON — ответ на вопрос\nФото, голосовые, аудио, видео и документы: прикрепите и нажмите «Отправить вложения». Голосовые передаются файлом; распознавание зависит от инструментов агента.",vec![]).await,
        "/stop"=>{let sid=selected(hub,chat).await?;for t in hub.snapshot().await?.turns.iter().filter(|t|t.session_id==sid&&t.user_id==Some(user)){hub.cancel(&t.id).await?}return Ok(())},
        "/files"=>{let sid=selected(hub,chat).await?;let files=hub.output_files(&sid).await?;return say(hub,chat,"Файлы из .tab/outbox. Нажатие отправляет оригинальные байты как document.",files.iter().map(|a|vec![button(&a.name,format!("f:{}",a.id))]).collect()).await},
        _=>{}
    }
    if let Some(value) = text.strip_prefix("/effort ").or_else(|| text.strip_prefix("/access ")) {
        let sid = selected(hub, chat).await?;
        let session = hub.session(&sid).await?;
        if session.provider == Provider::Mock { return say(hub, chat, "Mock не использует effort и права CLI.", vec![]).await; }
        let mut options = hub.agent_options(&sid).await?;
        let value = value.trim();
        if text.starts_with("/effort ") {
            let levels: &[&str] = match session.provider {
                Provider::Claude => &["default","low","medium","high","xhigh","max","ultracode"],
                Provider::Omp => &["default","off","minimal","low","medium","high","xhigh","max","auto"],
                _ => &["default","none","minimal","low","medium","high","xhigh","max","ultra"],
            };
            if !levels.contains(&value) { return say(hub,chat,&format!("Effort: {}. Доступность зависит от модели.",levels.join(", ")),vec![]).await; }
            options.effort = if value == "default" { None } else { Some(value.into()) };
        } else {
            if !["full","normal"].contains(&value) { return say(hub,chat,"/access full — полный доступ без подтверждений; /access normal — обычный режим.",vec![]).await; }
            options.full_access = value == "full";
        }
        hub.set_agent_options(&sid, options).await?;
        return say(hub,chat,"TAB · настройка сохранена для следующих задач этой сессии. Текущая задача продолжает работу с прежними настройками.",session_buttons(&sid)).await;
    }
    if let Some(prompt) = text.strip_prefix("/push ") {
        let sid = selected(hub, chat).await?;
        let session = hub.session(&sid).await?;
        if !matches!(session.provider, Provider::Codex | Provider::Omp) { return say(hub,chat,"Этот агент не поддерживает push. Отправьте обычное сообщение в очередь.",vec![]).await; }
        if prompt.trim().is_empty() { return say(hub,chat,"/push ТЕКСТ — уточнение для текущей задачи.",vec![]).await; }
        let turns = queue_turns(hub,&sid,chat,user).await?;
        let Some(active) = turns.iter().find(|t| t.state == "running") else { return say(hub,chat,"Нет работающей задачи. Отправьте текст обычным сообщением.",vec![]).await; };
        hub.steer(&active.id,prompt.into()).await?;
        return say(hub,chat,"TAB · уточнение передано адаптеру; ожидается ответ CLI.",session_buttons(&sid)).await;
    }
    if let Some(answer) = text.strip_prefix("/answer ") {
        let (rid, answer) = answer
            .split_once(' ')
            .ok_or_else(|| anyhow!("Формат /answer ID JSON"))?;
        return hub
            .answer(
                rid.into(),
                "allow".into(),
                serde_json::from_str(answer)?,
                Some(user),
            )
            .await;
    }
    let sid = selected(hub, chat).await?;
    let media_kind = ["document", "voice", "audio", "video", "video_note", "animation", "sticker"].into_iter().find(|key| msg[*key].is_object());
    let file = if let Some(kind) = media_kind {
        &msg[kind]
    } else {
        msg["photo"]
            .as_array()
            .and_then(|a| a.last())
            .unwrap_or(&Value::Null)
    };
    if let Some(fid) = file["file_id"].as_str() {
        if file["file_size"].as_u64().unwrap_or(0) > 20 * 1024 * 1024 {
            bail!("File too large")
        };
        let target = drafts.get(&chat).map(|d| d.0.clone()).unwrap_or(sid);
        if drafts.get(&chat).is_some_and(|d| d.1.len() >= 10) {
            bail!("Не более 10 вложений")
        };
        let bytes = client.download(fid).await?;
        let fallback_name = match media_kind { Some("voice") => "voice.ogg", Some("audio") => "audio.mp3", Some("video") | Some("video_note") | Some("animation") => "video.mp4", Some("sticker") => "sticker.webp", Some("document") => "document", _ => "photo.jpg" };
        let a = hub
            .stage(
                target.clone(),
                file["file_name"].as_str().unwrap_or(fallback_name).into(),
                bytes,
            )
            .await?;
        let draft = drafts
            .entry(chat)
            .or_insert((target, vec![], String::new(), now()));
        draft.1.push(a);
        draft.3 = now();
        if let Some(caption) = msg["caption"].as_str() {
            draft.2.push_str(caption);
            draft.2.push('\n')
        };
        return say(
            hub,
            chat,
            &format!(
                "Вложений: {}. Сессия закреплена за этим набором.",
                draft.1.len()
            ),
            vec![vec![button("Отправить вложения", "draft".into())]],
        )
        .await;
    }
    if text.starts_with('/') {
        return Ok(());
    }
    if !text.trim().is_empty() {
        if let Some(draft) = drafts.get_mut(&chat) {
            draft.2.push_str(text); draft.2.push('\n'); draft.3 = now();
            return say(hub,chat,"Текст добавлен к вложениям. Нажмите «Отправить вложения».",vec![vec![button("Отправить вложения","draft".into())]]).await;
        }
        hub.submit(sid.clone(), text.into(), vec![], Some((chat, user)))
            .await?;
        return show_queue(hub, chat, user, &sid).await;
    }
    Ok(())
}
async fn set_session(hub: &Hub, chat: i64, sid: &str) -> Result<()> {
    let sid = sid.to_string();
    hub.db.call(move|c|{c.execute("INSERT INTO routes(chat_id,session_id) VALUES(?1,?2) ON CONFLICT(chat_id) DO UPDATE SET session_id=excluded.session_id",params![chat,sid])?;Ok(())}).await
}

async fn project_events(hub: &Hub) -> Result<()> {
    hub.db.call(|c|{
        let tx=c.transaction()?;let cursor=tx.query_row("SELECT value FROM runtime_settings WHERE key='tg_event_cursor'",[],|r|r.get::<_,String>(0)).optional()?.and_then(|s|s.parse::<i64>().ok()).unwrap_or(0);
        let rows={let mut s=tx.prepare("SELECT e.seq,e.body,t.body FROM events e JOIN turns t ON t.id=e.turn_id WHERE e.seq>?1 ORDER BY e.seq LIMIT 100")?;let rows=s.query_map([cursor],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;rows};
        let mut last=cursor;
        for(seq,body,turn)in rows{last=seq;let e:Event=serde_json::from_str(&body)?;let t:Turn=serde_json::from_str(&turn)?;let Some(chat)=t.chat_id else{continue};
            let activity_key=format!("activity_turn:{}",t.id);
            let raw:Option<String>=tx.query_row("SELECT value FROM runtime_settings WHERE key=?1",[&activity_key],|r|r.get(0)).optional()?;
            let had_activity=raw.is_some();
            let mut activity=raw.and_then(|s|serde_json::from_str::<crate::telegram_activity::Activity>(&s).ok()).unwrap_or_default();
            let old_phase=activity.phase.clone();activity.update(&e);
            let activity_json=serde_json::to_string(&activity)?;
            tx.execute("INSERT OR REPLACE INTO runtime_settings VALUES(?1,?2)",params![activity_key,activity_json])?;
            if had_activity || !matches!(e.kind,EventKind::Completed|EventKind::Error) {
                tx.execute("INSERT OR REPLACE INTO runtime_settings VALUES(?1,?2)",params![format!("activity:{}",t.session_id),activity_json])?;
            }
            if !activity.phase.is_empty() && activity.phase!=old_phase {
                let status_key=format!("status:{}",t.id);
                let terminal=matches!(e.kind,EventKind::Completed);
                let buttons=if terminal {vec![vec![button("Файлы результата",format!("of:{}",t.session_id)),button("Очередь",format!("q:{}",t.session_id))]]} else {vec![vec![button("Остановить",format!("x:{}",t.id)),button("Обновить статус",format!("i:{}",t.session_id))]]};
                let body=json!({"chat_id":chat,"text":format!("TAB · {}",activity.label(e.occurred_at)),"reply_markup":{"inline_keyboard":buttons}});
                tx.execute("DELETE FROM outbox WHERE route_key=?1 AND state='queued'",[&status_key])?;
                tx.execute("INSERT INTO outbox(id,method,body,route_key) VALUES(?1,'sendMessage',?2,?3)",params![id(),body.to_string(),status_key])?;
            }
            let telemetry_key=format!("telemetry:{}",t.session_id);
            let raw:Option<String>=tx.query_row("SELECT value FROM runtime_settings WHERE key=?1",[&telemetry_key],|r|r.get(0)).optional()?;
            let mut telemetry:Value=raw.and_then(|s|serde_json::from_str(&s).ok()).unwrap_or(json!({}));
            if let Some(update)=e.data["telemetry"].as_object(){for(k,v)in update{telemetry[k]=v.clone();}tx.execute("INSERT OR REPLACE INTO runtime_settings VALUES(?1,?2)",params![telemetry_key,telemetry.to_string()])?;continue}
            let sbody:String=tx.query_row("SELECT body FROM sessions WHERE id=?1",[&t.session_id],|r|r.get(0))?;let s:Session=serde_json::from_str(&sbody)?;let label=format!("{} · {}\n{}",s.provider.key(),clip(&s.title,80),clip(&telemetry_label(&telemetry),400));
            let rich = matches!(e.kind, EventKind::Message | EventKind::Completed);
            let mut card=None;let mut buttons=vec![];let mut progress=false;let mut overflow=vec![];let mut result_card=None;
            match e.kind{
                EventKind::Message=>{let key=format!("tg_text:{}",t.id);let mut text=tx.query_row("SELECT value FROM runtime_settings WHERE key=?1",[&key],|r|r.get::<_,String>(0)).optional()?.unwrap_or_default();let delta=e.data["text"].as_str().unwrap_or("");if e.data["delta"]==true{text.push_str(delta)}else{text=delta.into()};text=clip(&text,200000);tx.execute("INSERT OR REPLACE INTO runtime_settings VALUES(?1,?2)",params![key,text])?;card=Some(format!("{label}\n{}",clip(&text,3000)));progress=true;},
                EventKind::ApprovalRequest|EventKind::UserQuestion=>{let r:Interaction=serde_json::from_value(e.data)?;card=Some(format!("{label}\n{}\n{}{}",r.kind,clip(&serde_json::to_string_pretty(&r.data)?,2500),if r.local_only{"\nОтветьте на ПК: этот native dialog не имеет проверенного remote scope.".into()}else if r.kind=="question"{format!("\n/answer {} JSON\nФормат ответа зависит от вопросов; ключ — native question ID.",r.id)}else{String::new()}));if !r.local_only&&r.kind!="question"{buttons.push(vec![button("Разрешить один раз",format!("a:{}",r.id)),button("Отказать",format!("d:{}",r.id))])}buttons.push(vec![button("Остановить",format!("x:{}",t.id))]);},
                EventKind::Completed=>{
                    let key=format!("tg_text:{}",t.id);let text=tx.query_row("SELECT value FROM runtime_settings WHERE key=?1",[key],|r|r.get::<_,String>(0)).optional()?.unwrap_or_default();let first=clip(&text,3000);let mut rest=&text[first.len()..];while !rest.is_empty(){let chunk=clip(rest,3500);rest=&rest[chunk.len()..];overflow.push(chunk);}card=Some(format!("{label}\n{}\n{}",e.data["state"].as_str().unwrap_or("interrupted"),first));progress=true;
                    let files=if activity.files.is_empty(){"CLI не сообщил об изменённых файлах.".into()}else{format!("Изменённые файлы по событиям CLI ({}):\n{}",activity.files.len(),clip(&activity.files.iter().take(20).cloned().collect::<Vec<_>>().join("\n"),1800))};
                    let excerpt=if text.trim().is_empty(){"Агент не прислал текстовый итог.".into()}else{format!("Из ответа агента:\n{}{}",clip(text.trim(),900),if text.encode_utf16().count()>900{"…"}else{""})};
                    result_card=Some(format!("TAB · результат · {}\n\n{excerpt}\n\n{files}",activity.label(e.occurred_at)));
                },
                EventKind::Error=>{card=Some(format!("{label}\nОшибка: {}",clip(&e.data.to_string(),3000)));},
                EventKind::Progress=>{if e.data["text"]!="Задача запущена" && e.data["text"]!="Агент думает" {card=Some(format!("{label}\n{}",clip(e.data["text"].as_str().unwrap_or("Статус обновлён"),1000)));}},
                _=>{}
            }
            if let Some(text)=card{
                buttons.extend(session_buttons(&t.session_id));
                let mut body=json!({"chat_id":chat,"text":text,"reply_markup":{"inline_keyboard":buttons}});
                if rich {
                    body["text"] = json!(crate::telegram_render::monospace(&text));
                    body["parse_mode"] = json!("HTML");
                    body["_plain_text"] = json!(text);
                }
                let key=if progress{Some(t.id.clone())}else{None};if let Some(ref key)=key{tx.execute("DELETE FROM outbox WHERE route_key=?1 AND state='queued'",[key])?;}tx.execute("INSERT INTO outbox(id,method,body,route_key) VALUES(?1,'sendMessage',?2,?3)",params![id(),body.to_string(),key])?;
                for chunk in overflow { let body=json!({"chat_id":chat,"text":crate::telegram_render::monospace(&chunk),"parse_mode":"HTML","_plain_text":chunk,"reply_markup":{"inline_keyboard":session_buttons(&t.session_id)}});tx.execute("INSERT INTO outbox(id,method,body) VALUES(?1,'sendMessage',?2)",params![id(),body.to_string()])?; }
            }
            if let Some(summary)=result_card {
                let body=json!({"chat_id":chat,"text":summary,"reply_markup":{"inline_keyboard":[[button("Скачать файлы результата",format!("of:{}",t.session_id)),button("Очередь",format!("q:{}",t.session_id))]]}});
                tx.execute("INSERT INTO outbox(id,method,body) VALUES(?1,'sendMessage',?2)",params![id(),body.to_string()])?;
            }
        }
        tx.execute("INSERT OR REPLACE INTO runtime_settings VALUES('tg_event_cursor',?1)",[last.to_string()])?;tx.commit()?;Ok(())
    }).await
}
async fn deliver(hub: Arc<Hub>, client: Client, stop: CancellationToken) -> Result<()> {
    let mut last_chat: HashMap<i64, tokio::time::Instant> = HashMap::new();
    while !stop.is_cancelled() {
        project_events(&hub).await?;
        hub.db.prune_closed_sessions().await?;
        let next=hub.db.call(|c|{let row=c.query_row("SELECT id,method,body,route_key,attempts FROM outbox WHERE state='queued' AND next_at<=?1 ORDER BY rowid LIMIT 1",[now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,i64>(4)?))).optional()?;Ok(row)}).await?;
        let Some((oid, mut method, body, key, attempts)) = next else {
            tokio::select! {_=stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(250))=>{}}
            continue;
        };
        let mut body: Value = serde_json::from_str(&body)?;
        let plain_text = body.as_object_mut().and_then(|b| b.remove("_plain_text"));
        let chat = body["chat_id"].as_i64().unwrap_or(0);
        if let Some(last) = last_chat.get(&chat) {
            let delay = Duration::from_millis(1500).saturating_sub(last.elapsed());
            tokio::select! {_=stop.cancelled()=>break,_=tokio::time::sleep(delay)=>{}}
        }
        if let Some(ref key) = key {
            let k = key.clone();
            let mid = hub
                .db
                .call(move |c| {
                    Ok(c.query_row(
                        "SELECT message_id FROM telegram_messages WHERE turn_id=?1",
                        [k],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?)
                })
                .await?;
            if let Some(mid) = mid {
                method = "editMessageText".into();
                body["message_id"] = json!(mid);
            }
        }
        let id = oid.clone();
        hub.db
            .call(move |c| {
                c.execute("UPDATE outbox SET state='sending' WHERE id=?1", [id])?;
                Ok(())
            })
            .await?;
        let mut result = client.call(&method, &body).await;
        if result.as_ref().err().and_then(|e| e.downcast_ref::<ApiError>()).is_some_and(|e| e.code == 400) {
            if let Some(text) = plain_text {
                body.as_object_mut().unwrap().remove("parse_mode");
                body["text"] = text;
                result = client.call(&method, &body).await;
            }
        }
        last_chat.insert(chat, tokio::time::Instant::now());
        let mut state = "sent";
        let mut retry = 0;
        let mut mid = None;
        match result {
            Ok(v) => mid = v["message_id"].as_i64(),
            Err(e) => {
                if let Some(api) = e.downcast_ref::<ApiError>() {
                    match api.code {
                        401 | 409 => return Err(e),
                        429 => {
                            state = "queued";
                            retry = api.retry.max(1) + 1
                        }
                        500..=599 => {
                            state = "queued";
                            retry = (1i64 << attempts.min(8)) + 1
                        }
                        400 if method == "editMessageText" => state = "failed",
                        _ => state = "failed",
                    }
                } else {
                    state = "unknown"
                }
            }
        }
        let state = state.to_string();
        hub.db
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute(
                    "UPDATE outbox SET state=?2,next_at=?3,attempts=attempts+1 WHERE id=?1",
                    params![oid, state, now() + retry],
                )?;
                if let (Some(key), Some(mid)) = (key, mid) {
                    tx.execute(
                        "INSERT OR REPLACE INTO telegram_messages VALUES(?1,?2,?3)",
                        params![key, chat, mid],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        tokio::select! {_=stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(50))=>{}}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disconnected_preference_survives_shutdown_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::open(dir.path().join("data")).await.unwrap();
        {
            let mut c = hub.config.write().await;
            c.telegram_user = Some(1);
        }
        hub.db.put("telegram_autoconnect", "true".into()).await.unwrap();
        stop(&hub).await.unwrap();
        assert_eq!(hub.db.setting("telegram_autoconnect").await.unwrap().as_deref(), Some("true"));
        disconnect(&hub).await.unwrap();
        restore(hub.clone()).await.unwrap(); // Must not read credentials or contact Telegram.
        assert!(hub.telegram_stop.lock().await.is_none());
        assert_eq!(hub.db.setting("telegram_autoconnect").await.unwrap().as_deref(), Some("false"));
        let count: i64 = hub.db.call(|db| Ok(db.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?)).await.unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn connection_retries_only_transient_errors() {
        assert!(retry_connection(&anyhow!("Telegram: network outcome unknown")));
        for code in [429, 500, 503] { assert!(retry_connection(&ApiError { code, retry: 5 }.into())); }
        for code in [401, 403, 409] { assert!(!retry_connection(&ApiError { code, retry: 0 }.into())); }
        assert!(!retry_connection(&anyhow!("Webhook configured")));
    }
    #[tokio::test]
    async fn end_session_is_bound_to_current_route_and_discards_context() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::open(dir.path().join("data")).await.unwrap();
        let p = hub.add_project("test".into(), dir.path().to_path_buf()).await.unwrap();
        let first = hub.create_session(p.id.clone(), Provider::Mock, "first".into(), None).await.unwrap();
        let second = hub.create_session(p.id, Provider::Mock, "second".into(), None).await.unwrap();
        set_session(&hub, 1, &second.id).await.unwrap();
        assert!(!stop_session(&hub, 1, 1, &first.id, true).await.unwrap());
        assert_eq!(selected(&hub, 1).await.unwrap(), second.id);
        assert!(stop_session(&hub, 1, 1, &second.id, true).await.unwrap());
        assert!(selected(&hub, 1).await.is_err());
        hub.db.prune_closed_sessions().await.unwrap();
        assert!(hub.session(&second.id).await.is_err());
        assert!(!stop_session(&hub, 1, 1, &second.id, true).await.unwrap());
        assert!(session_buttons(&second.id).iter().flatten().all(|b| b["callback_data"].as_str().unwrap().len() <= 64));
    }
    #[test]
    fn unicode_limit() {
        assert_eq!(clip("😀абв", 2), "😀");
        assert!(
            button("Allow", format!("a:{}", id()))["callback_data"]
                .as_str()
                .unwrap()
                .len()
                <= 64
        );
    }
}
