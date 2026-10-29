use crate::{domain::*, process::Process};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio::sync::mpsc;

pub async fn discover() -> Vec<Installation> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let mut result = vec![];
    for (provider, paths) in [
        (
            Provider::Codex,
            vec![PathBuf::from(&local).join("Programs/OpenAI/Codex/bin/codex.exe")],
        ),
        (
            Provider::Claude,
            vec![PathBuf::from(&home).join(".local/bin/claude.exe")],
        ),
        (
            Provider::Omp,
            vec![PathBuf::from(&local).join("omp/omp.exe")],
        ),
    ] {
        let mut candidates = paths;
        if let Some(path) = std::env::var_os("PATH") {
            for p in std::env::split_paths(&path) {
                if p.is_absolute() {
                    candidates.push(p.join(format!("{}.exe", provider.key())));
                }
            }
        }
        if let Some(executable) = candidates.into_iter().find(|p| p.is_file()) {
            let mut cmd = tokio::process::Command::new(&executable);
            cmd.arg("--version").kill_on_drop(true);
            #[cfg(windows)]
            cmd.creation_flags(0x08000000);
            let version = match tokio::time::timeout(Duration::from_secs(10), cmd.output()).await {
                Ok(Ok(o)) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                    .chars()
                    .take(120)
                    .collect::<String>()
                    .trim()
                    .to_owned(),
                _ => "version unavailable".into(),
            };
            result.push(Installation {
                provider,
                executable,
                version,
                enabled: false,
                accept_unsandboxed: false,
            });
        }
    }
    result
}

pub fn capabilities(provider: &Provider) -> Value {
    match provider {
        Provider::Codex => {
            json!({"transport":"app-server stdio","steering":true,"approvals":"structured","questions":true,"isolation":"provider sandbox; runtime qualification required","runtime_verified":false})
        }
        Provider::Claude => {
            json!({"transport":"stream-json control","steering":false,"approvals":"structured","questions":true,"isolation":"no native Windows OS sandbox","runtime_verified":false})
        }
        Provider::Omp => {
            json!({"transport":"rpc-ui JSONL v1","steering":true,"approvals":"native dialog: local confirmation only","questions":true,"isolation":"no verified OS sandbox","runtime_verified":false})
        }
        Provider::Mock => {
            json!({"transport":"mock","steering":false,"approvals":"structured","questions":true,"isolation":"no execution","runtime_verified":true})
        }
    }
}

async fn emit(tx: &mpsc::Sender<NativeEvent>, kind: EventKind, data: Value) -> Result<()> {
    tx.send(NativeEvent::Event(kind, data))
        .await
        .map_err(|_| anyhow!("Event consumer closed"))
}

pub async fn run(
    install: Installation,
    session: Session,
    turn: Turn,
    root: PathBuf,
    tx: mpsc::Sender<NativeEvent>,
    controls: mpsc::Receiver<Control>,
) -> Result<()> {
    run_with_options(install, session, turn, root, tx, controls, AgentOptions::default()).await
}

pub async fn run_with_options(
    install: Installation,
    session: Session,
    turn: Turn,
    root: PathBuf,
    tx: mpsc::Sender<NativeEvent>,
    mut controls: mpsc::Receiver<Control>,
    options: AgentOptions,
) -> Result<()> {
    if install.provider == Provider::Mock {
        return run_mock(turn, tx, controls).await;
    }
    if !install.enabled {
        bail!("Адаптер не включён локально")
    }
    if install.provider != Provider::Codex && !install.accept_unsandboxed {
        bail!("Нужно локально подтвердить отсутствие OS sandbox")
    }
    let mut args: Vec<String> = match install.provider {
        Provider::Codex => vec!["app-server".into(), "--stdio".into()],
        Provider::Claude => [
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-mode",
            "manual",
            "--permission-prompt-tool",
            "stdio",
            "--permission-prompts",
            "host",
        ]
        .iter()
        .map(|x| x.to_string())
        .collect(),
        Provider::Omp => ["--mode", "rpc-ui", "--approval-mode", "always-ask"]
            .iter()
            .map(|x| x.to_string())
            .collect(),
        _ => unreachable!(),
    };
    if install.provider == Provider::Claude {
        if options.full_access {
            if let Some(i) = args.iter().position(|a| a == "--permission-mode") { args[i + 1] = "bypassPermissions".into(); }
        }
        if let Some(effort) = &options.effort { args.extend(["--effort".into(), effort.clone()]); }
        if let Some(n) = &session.native_id {
            args.extend(["--resume".into(), n.clone()])
        }
        if let Some(m) = &session.model {
            args.extend(["--model".into(), m.clone()])
        }
    }
    if install.provider == Provider::Omp {
        if options.full_access {
            if let Some(i) = args.iter().position(|a| a == "--approval-mode") { args[i + 1] = "yolo".into(); }
        }
        if let Some(effort) = &options.effort { args.extend(["--thinking".into(), effort.clone()]); }
    }
    let mut process = Process::spawn(&install.executable, &args, &root)?;
    let content = prepare_input(&turn, &install.provider)?;
    let mut thread_id = session.native_id.clone().unwrap_or_default();
    let mut native_turn = String::new();
    let mut started = false;
    let mut cancelling = false;
    let mut cancel_at = None;
    let mut serial = 100i64;
    let mut steer_pending = std::collections::HashSet::<i64>::new();
    let mut steer_replies = std::collections::HashMap::<i64, tokio::sync::oneshot::Sender<bool>>::new();
    let mut omp_status_at = tokio::time::Instant::now();
    let approval = if options.full_access { "never" } else { "on-request" };
    let sandbox = if options.full_access { "danger-full-access" } else { "workspace-write" };
    emit(&tx, EventKind::Progress, json!({"telemetry":{"effort":options.effort,"full_access":options.full_access}})).await?;
    match install.provider{
        Provider::Codex=>process.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"tab","version":"0.1.0"},"capabilities":{"experimentalApi":true}}})).await?,
        Provider::Claude=>process.send(json!({"type":"control_request","request_id":"hub-init","request":{"subtype":"initialize","hooks":null}})).await?,
        _=>{}}
    let handshake = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            cmd=controls.recv()=>match cmd{
                Some(Control::Cancel)=>{cancelling=true;cancel_at=Some(tokio::time::Instant::now());match install.provider{
                    Provider::Codex if !native_turn.is_empty()=>process.send(json!({"id":99,"method":"turn/interrupt","params":{"threadId":thread_id,"turnId":native_turn}})).await?,
                    Provider::Claude=>process.send(json!({"type":"control_request","request_id":"hub-cancel","request":{"subtype":"interrupt"}})).await?,
                    Provider::Omp=>process.send(json!({"id":"hub-cancel","type":"abort"})).await?,_=>break}},
                Some(Control::Steer(text))=>{if !started||cancelling{emit(&tx,EventKind::Error,json!({"text":"Steering недоступен до запуска или во время отмены"})).await?;continue};serial+=1;match install.provider{
                    Provider::Codex if !native_turn.is_empty()=>{steer_pending.insert(serial);process.send(json!({"id":serial,"method":"turn/steer","params":{"threadId":thread_id,"expectedTurnId":native_turn,"input":[{"type":"text","text":text}]}})).await?},
                    Provider::Omp=>{steer_pending.insert(serial);process.send(json!({"id":serial.to_string(),"type":"steer","message":text})).await?},
                    _=>emit(&tx,EventKind::Error,json!({"text":"Этот адаптер не поддерживает steering; отправьте следующий turn"})).await?}},
                Some(Control::Respond{request,decision,answers})=>process.send(response(&install.provider,&request,&decision,answers)?).await?,
                Some(Control::SteerWithAck{text,reply})=>{
                    if !started||cancelling { let _=reply.send(false);continue }
                    serial+=1;
                    let request=match install.provider {
                        Provider::Codex if !native_turn.is_empty()=>json!({"id":serial,"method":"turn/steer","params":{"threadId":thread_id,"expectedTurnId":native_turn,"input":[{"type":"text","text":text}]}}),
                        Provider::Omp=>json!({"id":serial.to_string(),"type":"steer","message":text}),
                        _=>{let _=reply.send(false);continue}
                    };
                    steer_pending.insert(serial);steer_replies.insert(serial,reply);process.send(request).await?;
                },
                None=>break,
            },
            _=tick.tick()=>{
                if install.provider==Provider::Omp && started && !cancelling && omp_status_at.elapsed()>Duration::from_secs(10) {
                    process.send(json!({"id":"telemetry","type":"get_state"})).await?;
                    omp_status_at=tokio::time::Instant::now();
                }
                if !started&&handshake.elapsed()>Duration::from_secs(30){bail!("CLI handshake timeout")}
                if cancel_at.map(|t|t.elapsed()>Duration::from_secs(5)).unwrap_or(false){break}
            },
            line=process.lines.recv()=>{
                let Some(line)=line else{if cancelling{break}bail!("CLI завершился до terminal event; результат выполнения неопределён")};
                let line=line?;if line.trim().is_empty(){continue}
                let v:Value=serde_json::from_str(&line).map_err(|_|anyhow!("Неверный JSON от CLI"))?;
                match install.provider{
                    Provider::Codex=>{
                        if let Some(i)=v["id"].as_i64(){if steer_pending.remove(&i){let accepted=v.get("error").is_none();if let Some(reply)=steer_replies.remove(&i){let _=reply.send(accepted);}else{emit(&tx,EventKind::Progress,json!({"text":if accepted{"Уточнение принято"}else{"Уточнение отклонено; отправьте его обычным сообщением"}})).await?;}continue}}
                        if v["id"]==4{emit(&tx,EventKind::Progress,json!({"telemetry":{"rateLimits":v["result"]["rateLimits"],"rateLimitsByLimitId":v["result"]["rateLimitsByLimitId"]}})).await?;continue}
                        if v.get("error").is_some()&&v.get("method").is_none(){bail!("Codex отклонил запрос: {}",v["error"]["message"].as_str().unwrap_or("protocol error"))}
                        if v["id"]==1{process.send(json!({"method":"initialized","params":{}})).await?;process.send(json!({"id":4,"method":"account/rateLimits/read","params":{}})).await?;let mut params=json!({"cwd":root,"sandbox":sandbox,"approvalPolicy":approval});if let Some(m)=&session.model{params["model"]=json!(m)};let method=if let Some(n)=&session.native_id{params["threadId"]=json!(n);"thread/resume"}else{"thread/start"};process.send(json!({"id":2,"method":method,"params":params})).await?;continue}
                        if v["id"]==2{thread_id=v["result"]["thread"]["id"].as_str().ok_or_else(||anyhow!("Missing native thread ID"))?.into();tx.send(NativeEvent::Session(thread_id.clone())).await?;emit(&tx,EventKind::Progress,json!({"telemetry":{"effort":options.effort.as_ref().map(|s|json!(s)).unwrap_or(v["result"]["reasoningEffort"].clone()),"model":v["result"]["model"]}})).await?;let mut params=json!({"threadId":thread_id,"input":content,"approvalPolicy":approval});if let Some(effort)=&options.effort{params["effort"]=json!(effort)};process.send(json!({"id":3,"method":"turn/start","params":params})).await?;started=true;continue}
                        if v["id"]==3{native_turn=v["result"]["turn"]["id"].as_str().unwrap_or("").into();continue}
                        let method=v["method"].as_str().unwrap_or("");let p=&v["params"];
                        if v.get("id").is_some()&&!method.is_empty(){
                            let(kind,local)=match method{"item/commandExecution/requestApproval"|"item/fileChange/requestApproval"=>("approval",false),"item/tool/requestUserInput"=>("question",false),"item/permissions/requestApproval"|"mcpServer/elicitation/request"=>("unsupported",true),_=>{process.send(json!({"id":v["id"],"error":{"code":-32601,"message":"Unsupported server request"}})).await?;bail!("Неизвестный управляющий запрос Codex")}};
                            tx.send(NativeEvent::Request{native_id:v["id"].clone(),kind:kind.into(),data:json!({"method":method,"params":p}),local_only:local}).await?;continue
                        }
                        match method{
                            "thread/tokenUsage/updated"=>emit(&tx,EventKind::Progress,json!({"telemetry":{"tokenUsage":p["tokenUsage"]}})).await?,
                            "account/rateLimits/updated"=>emit(&tx,EventKind::Progress,json!({"telemetry":{"rateLimits":p["rateLimits"],"rateLimitsByLimitId":p["rateLimitsByLimitId"]}})).await?,
                            "turn/started"=>{native_turn=p["turn"]["id"].as_str().unwrap_or("").into();emit(&tx,EventKind::Progress,json!({"text":"Агент думает"})).await?;},
                            "item/agentMessage/delta"=>emit(&tx,EventKind::Message,json!({"text":p["delta"],"delta":true})).await?,
                            "item/reasoning/summaryTextDelta"=>emit(&tx,EventKind::ReasoningSummary,json!({"text":p["delta"],"delta":true})).await?,
                            "item/started"=>{if p["item"]["type"]!="agentMessage"{emit(&tx,EventKind::ToolCall,p["item"].clone()).await?}},
                            "item/completed"=>{if p["item"]["type"]=="agentMessage"{emit(&tx,EventKind::Message,json!({"text":p["item"]["text"],"delta":false})).await?}else{emit(&tx,EventKind::ToolResult,p["item"].clone()).await?}},
                            "turn/completed"=>{let status=p["turn"]["status"].as_str().unwrap_or("failed");tx.send(NativeEvent::Finished(if cancelling||status=="interrupted"{"cancelled"}else if status=="completed"{"completed"}else{"failed"}.into())).await?;return Ok(())},
                            "error"=>emit(&tx,EventKind::Error,p.clone()).await?,_=>{}}
                    },
                    Provider::Claude=>{
                        match v["type"].as_str().unwrap_or(""){
                            "control_response" if v["response"]["request_id"]=="hub-init"=>{if v["response"]["subtype"]=="error"{bail!("Claude rejected initialization")};process.send(json!({"type":"user","message":{"role":"user","content":content}})).await?;started=true;emit(&tx,EventKind::Progress,json!({"text":"Агент думает"})).await?;},
                            "system"=>{if let Some(s)=v["session_id"].as_str(){tx.send(NativeEvent::Session(s.into())).await?}},
                            "control_cancel_request"=>bail!("Native request отменён; turn остановлен, прежние ответы недействительны"),
                            "control_request"=>{if v["request"]["subtype"]!="can_use_tool"{process.send(json!({"type":"control_response","response":{"subtype":"error","request_id":v["request_id"],"error":"Unsupported control request"}})).await?;continue};let question=v["request"]["tool_name"]=="AskUserQuestion";tx.send(NativeEvent::Request{native_id:v["request_id"].clone(),kind:if question{"question"}else{"approval"}.into(),data:v["request"].clone(),local_only:false}).await?},
                            "stream_event"=>{let event=&v["event"];if event["type"]=="content_block_delta"&&event["delta"]["type"]=="text_delta"{emit(&tx,EventKind::Message,json!({"text":event["delta"]["text"],"delta":true})).await?}},
                            "assistant"=>{if let Some(blocks)=v["message"]["content"].as_array(){for b in blocks{if b["type"]=="tool_use"{emit(&tx,EventKind::ToolCall,b.clone()).await?}}}},
                            "user"=>{if let Some(blocks)=v["message"]["content"].as_array(){for b in blocks{if b["type"]=="tool_result"{emit(&tx,EventKind::ToolResult,b.clone()).await?}}}},
                            "result"=>{if let Some(s)=v["result"].as_str(){emit(&tx,EventKind::Message,json!({"text":s,"delta":false})).await?};emit(&tx,EventKind::Progress,json!({"usage":v["usage"]})).await?;tx.send(NativeEvent::Finished(if cancelling{"cancelled"}else if v["is_error"]==true{"failed"}else{"completed"}.into())).await?;return Ok(())},_=>{}
                        }
                    },
                    Provider::Omp=>{
                        if v["type"]=="response" && (v["id"]=="state" || v["id"]=="telemetry") {
                            if v["success"]==true { let d=&v["data"];emit(&tx,EventKind::Progress,json!({"telemetry":{"effort":d["thinkingLevel"],"tokenUsage":{"last":{"totalTokens":d["contextUsage"]["tokens"]},"modelContextWindow":d["contextUsage"]["contextWindow"]},"allowanceLeftPercent":d["usageLimit"]["allowanceLeftPercent"]}})).await?; }
                            if v["id"]=="telemetry" { continue }
                        }
                        if let Some(i)=v["id"].as_str().and_then(|s|s.parse::<i64>().ok()){if steer_pending.remove(&i){let accepted=v["success"]==true;if let Some(reply)=steer_replies.remove(&i){let _=reply.send(accepted);}else{emit(&tx,EventKind::Progress,json!({"text":if accepted{"Уточнение принято"}else{"Уточнение отклонено; отправьте его обычным сообщением"}})).await?;}continue}}
                        match v["type"].as_str().unwrap_or(""){
                            "ready"=>{if let Some(n)=&session.native_id{process.send(json!({"id":"resume","type":"switch_session","sessionPath":n})).await?}else{process.send(json!({"id":"state","type":"get_state"})).await?}},
                            "response"=>{if v["success"]==false{bail!("OMP отклонил команду: {}",v["error"])}if v["id"]=="resume"{process.send(json!({"id":"state","type":"get_state"})).await?}if v["id"]=="state"{if let Some(n)=v["data"]["sessionFile"].as_str(){tx.send(NativeEvent::Session(n.into())).await?};if let Some(model)=&session.model{let Some((provider,model_id))=model.split_once('/')else{bail!("OMP model: provider/modelId")};process.send(json!({"id":"model","type":"set_model","provider":provider,"modelId":model_id})).await?}else{process.send(content.clone()).await?;started=true}}if v["id"]=="model"{process.send(content.clone()).await?;started=true}},
                            "message_update"=>{let e=&v["assistantMessageEvent"];if e["type"]=="text_delta"{emit(&tx,EventKind::Message,json!({"text":e["delta"],"delta":true})).await?}},
                            "agent_start"=>emit(&tx,EventKind::Progress,json!({"text":"Агент думает"})).await?,
                            "tool_execution_start"=>emit(&tx,EventKind::ToolCall,v.clone()).await?,
                            "tool_execution_end"=>emit(&tx,EventKind::ToolResult,v.clone()).await?,
                            "extension_ui_request"=>{let m=v["method"].as_str().unwrap_or("");if ["select","confirm","input","editor","ask"].contains(&m){tx.send(NativeEvent::Request{native_id:v["id"].clone(),kind:"native_dialog".into(),data:v.clone(),local_only:true}).await?}else if m=="cancel"{bail!("OMP отменил native dialog; turn остановлен для аннулирования прежних ответов")}},
                            "rpc_chunk"=>bail!("OMP прислал chunk без согласования v2; transport остановлен"),
                            "agent_end" if started=>{tx.send(NativeEvent::Finished(if cancelling{"cancelled"}else{"completed"}.into())).await?;return Ok(())},_=>{}
                        }
                    },_=>unreachable!()
                }
            }
        }
    }
    process.stop();
    tx.send(NativeEvent::Finished("cancelled".into())).await?;
    Ok(())
}

fn prepare_input(turn: &Turn, provider: &Provider) -> Result<Value> {
    let mut text = turn.text.clone();
    let mut images = vec![];
    for a in &turn.assets {
        if a.mime.starts_with("image/") {
            images.push(a)
        } else {
            text.push_str(&format!("\nВложение {} ({}): {}", a.name, a.mime, a.path.display()));
            if a.mime.starts_with("audio/") || a.name.starts_with("voice.") {
                text.push_str("\nЭто аудиовложение. Если доступны инструменты распознавания речи, расшифруй его и используй содержание как сообщение пользователя. Если инструментов нет, сообщи об этом; не выдумывай расшифровку.");
            }
        }
    }
    Ok(match provider {
        Provider::Codex => {
            let mut c = vec![json!({"type":"text","text":text})];
            for a in images {
                c.push(json!({"type":"localImage","path":a.path}))
            }
            json!(c)
        }
        Provider::Claude => {
            use base64::Engine;
            let mut c = vec![json!({"type":"text","text":text})];
            for a in images {
                if a.bytes > 4 * 1024 * 1024 {
                    bail!("Изображение превышает лимит Claude transport 4 MiB")
                };
                c.push(json!({"type":"image","source":{"type":"base64","media_type":a.mime,"data":base64::engine::general_purpose::STANDARD.encode(std::fs::read(&a.path)?)}}))
            }
            json!(c)
        }
        Provider::Omp => {
            use base64::Engine;
            let mut c = vec![];
            for a in images {
                if a.bytes > 512 * 1024 {
                    bail!("Для OMP RPC v1 изображение должно быть меньше 512 KiB")
                };
                c.push(json!({"type":"image","mimeType":a.mime,"data":base64::engine::general_purpose::STANDARD.encode(std::fs::read(&a.path)?)}))
            }
            let v = json!({"id":"prompt","type":"prompt","message":text,"images":c});
            if serde_json::to_vec(&v)?.len() > 1024 * 1024 {
                bail!("Вложения превышают OMP v1 frame limit")
            };
            v
        }
        _ => json!(text),
    })
}
fn response(provider: &Provider, r: &Interaction, decision: &str, answers: Value) -> Result<Value> {
    Ok(match provider {
        Provider::Codex => {
            let method = r.data["method"].as_str().unwrap_or("");
            let result = match method {
                "item/tool/requestUserInput" => {
                    if decision != "allow" {
                        bail!("Остановите turn для отмены вопроса Codex")
                    };
                    json!({"answers":answers})
                }
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    json!({"decision":if decision=="allow"{"accept"}else{"decline"}})
                }
                _ => bail!("Этот permission request не поддерживается; остановите turn"),
            };
            json!({"id":r.native_id,"result":result})
        }
        Provider::Claude => {
            let mut input = r.data["input"].clone();
            if r.kind == "question" {
                input["answers"] = answers
            };
            json!({"type":"control_response","response":{"subtype":"success","request_id":r.native_id,"response":if decision=="allow"{json!({"behavior":"allow","updatedInput":input})}else{json!({"behavior":"deny","message":"User denied"})}}})
        }
        Provider::Omp => {
            let mut v = json!({"type":"extension_ui_response","id":r.native_id});
            if decision != "allow" {
                v["cancelled"] = json!(true)
            } else {
                match r.data["method"].as_str().unwrap_or("") {
                    "confirm" => v["confirmed"] = json!(true),
                    "ask" => v["answers"] = answers,
                    _ => {
                        v["value"] = if answers.is_string() {
                            answers
                        } else {
                            json!(answers.to_string())
                        }
                    }
                }
            }
            v
        }
        _ => json!({}),
    })
}
async fn run_mock(
    turn: Turn,
    tx: mpsc::Sender<NativeEvent>,
    mut controls: mpsc::Receiver<Control>,
) -> Result<()> {
    if turn.text.contains("/approval") {
        tx.send(NativeEvent::Request {
            native_id: json!(1),
            kind: "approval".into(),
            data: json!({"command":"mock operation (no side effects)"}),
            local_only: false,
        })
        .await?;
        match controls.recv().await {
            Some(Control::Respond { decision, .. }) if decision == "allow" => {}
            _ => {
                tx.send(NativeEvent::Finished("cancelled".into())).await?;
                return Ok(());
            }
        }
    }
    for word in format!("Mock: {}", turn.text).split_inclusive(' ') {
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(40))=>emit(&tx,EventKind::Message,json!({"text":word,"delta":true})).await?,_=controls.recv()=>{tx.send(NativeEvent::Finished("cancelled".into())).await?;return Ok(())}}
    }
    tx.send(NativeEvent::Finished("completed".into())).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_approval_is_not_allowed() {
        let r = Interaction {
            id: "x".into(),
            turn_id: "t".into(),
            session_id: "s".into(),
            native_id: json!(5),
            kind: "approval".into(),
            data: json!({"method":"unrecognized"}),
            expires_at: 0,
            state: "pending".into(),
            local_only: false,
        };
        assert!(response(&Provider::Codex, &r, "allow", json!({})).is_err());
    }
    #[tokio::test]
    async fn mock_streams_and_finishes() {
        let (tx, mut rx) = mpsc::channel(16);
        let (_control, controls) = mpsc::channel(2);
        let turn = Turn {
            id: id(),
            session_id: id(),
            text: "test".into(),
            assets: vec![],
            chat_id: None,
            user_id: None,
            epoch: "".into(),
            state: "running".into(),
            created_at: now(),
        };
        run_mock(turn, tx, controls).await.unwrap();
        let mut finish = false;
        while let Some(e) = rx.recv().await {
            finish |= matches!(e, NativeEvent::Finished(_))
        }
        assert!(finish);
    }
}
