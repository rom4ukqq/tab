use hub_core::{adapter, domain::*, process::Process, store::Store, Hub};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn wait_terminal(h: &Hub, id: &str) -> Turn {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let s = h.snapshot().await.unwrap();
            let t = s.turns.into_iter().find(|t| t.id == id).unwrap();
            if !["running", "queued", "dispatching"].contains(&t.state.as_str()) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
async fn wait_request(h: &Hub) -> Interaction {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(r) = h.snapshot().await.unwrap().requests.into_iter().next() {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
async fn hub() -> (tempfile::TempDir, Arc<Hub>, Session) {
    let d = tempfile::tempdir().unwrap();
    let project = d.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let h = Hub::open(d.path().join("data")).await.unwrap();
    let p = h.add_project("Fixture".into(), project).await.unwrap();
    let s = h
        .create_session(p.id, Provider::Mock, "test".into(), None)
        .await
        .unwrap();
    (d, h, s)
}

#[tokio::test]
async fn agent_session_needs_no_project_setup() {
    let d = tempfile::tempdir().unwrap();
    let h = Hub::open(d.path().join("data")).await.unwrap();
    assert!(h.create_agent_session(Provider::Mock).await.is_err());
    h.config.write().await.installations.push(Installation {
        provider: Provider::Mock,
        executable: PathBuf::new(),
        version: "mock".into(),
        enabled: true,
        accept_unsandboxed: false,
    });
    let first = h.create_agent_session(Provider::Mock).await.unwrap();
    let second = h.create_agent_session(Provider::Mock).await.unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.project_id, second.project_id);
    let root = h.config.read().await.projects[0].root.clone();
    assert_eq!(root, d.path().join("data/workspaces/mock").canonicalize().unwrap());
    let turn = h.submit(first.id, "hello".into(), vec![], None).await.unwrap();
    assert_eq!(wait_terminal(&h, &turn.id).await.state, "completed");
}

#[tokio::test]
async fn queue_and_atomic_approval_consume() {
    let (_d, h, s) = hub().await;
    let first = h
        .submit(s.id.clone(), "/approval".into(), vec![], None)
        .await
        .unwrap();
    let r = wait_request(&h).await;
    let second = h
        .submit(s.id.clone(), "second".into(), vec![], None)
        .await
        .unwrap();
    assert_eq!(
        h.snapshot()
            .await
            .unwrap()
            .turns
            .iter()
            .find(|t| t.id == second.id)
            .unwrap()
            .state,
        "queued"
    );
    h.answer(r.id.clone(), "allow".into(), json!({}), None)
        .await
        .unwrap();
    assert!(h
        .answer(r.id, "allow".into(), json!({}), None)
        .await
        .is_err());
    assert_eq!(wait_terminal(&h, &first.id).await.state, "completed");
    assert_eq!(wait_terminal(&h, &second.id).await.state, "completed");
    let es = h.history(s.id, 0).await.unwrap();
    assert!(es.windows(2).all(|w| w[0].sequence < w[1].sequence));
}
#[tokio::test]
async fn cancel_waiting_turn_and_reject_foreign_user() {
    let (_d, h, s) = hub().await;
    assert!(!h.authorized(456, false).await.unwrap());
    assert!(h
        .submit(s.id.clone(), "remote".into(), vec![], Some((456, 456)))
        .await
        .is_err());
    let t = h
        .submit(s.id, "/approval".into(), vec![], None)
        .await
        .unwrap();
    let r = wait_request(&h).await;
    h.cancel(&t.id).await.unwrap();
    assert_eq!(wait_terminal(&h, &t.id).await.state, "cancelled");
    assert!(h
        .answer(r.id, "allow".into(), json!({}), None)
        .await
        .is_err());
}
#[tokio::test]
async fn restart_does_not_replay_uncertain_work() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db.sqlite");
    let s = Store::open(&p).unwrap();
    s.call(|c|{c.execute_batch("INSERT INTO sessions VALUES('s','{}'); INSERT INTO turns VALUES('t','s','running','{}',0); INSERT INTO interactions VALUES('r','t','pending','{}'); INSERT INTO grants VALUES(1,'epoch',0,100); INSERT INTO attempts VALUES(1,5,0,99999,1); INSERT INTO outbox(id,method,body,state) VALUES('d','sendMessage','{}','sending');")?;Ok(())}).await.unwrap();
    drop(s);
    let s = Store::open(&p).unwrap();
    let got = s
        .call(|c| {
            Ok((
                c.query_row("SELECT COUNT(*) FROM turns", [], |r| r.get::<_, i64>(0))?,
                c.query_row("SELECT COUNT(*) FROM interactions", [], |r| {
                    r.get::<_, i64>(0)
                })?,
                c.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get::<_, i64>(0))?,
                c.query_row("SELECT COUNT(*) FROM grants", [], |r| r.get::<_, i64>(0))?,
                c.query_row("SELECT blocked_until FROM attempts", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        (
            0,
            0,
            0,
            0,
            99999
        )
    );
}
#[tokio::test]
async fn attachment_bytes_and_untrusted_paths() {
    let (_d, h, s) = hub().await;
    let a = h
        .stage(
            s.id.clone(),
            "../../evil.txt".into(),
            b"fixture bytes".to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&a.path).unwrap(), b"fixture bytes");
    assert!(!a.path.to_string_lossy().contains("evil.txt"));
    let mut forged = a.clone();
    forged.id = "not-registered".into();
    assert!(h
        .submit(s.id.clone(), "read".into(), vec![forged], None)
        .await
        .is_err());
    let mut spoof = a.clone();
    spoof.path = PathBuf::from("C:\\Windows\\win.ini");
    let t = h
        .submit(s.id, "read".into(), vec![spoof], None)
        .await
        .unwrap();
    assert_eq!(t.assets[0].path, a.path);
    wait_terminal(&h, &t.id).await;
}

#[tokio::test]
async fn three_structured_protocol_fixtures() {
    for provider in [Provider::Codex, Provider::Claude, Provider::Omp] {
        let d = tempfile::tempdir().unwrap();
        let i = Installation {
            provider: provider.clone(),
            executable: PathBuf::from(env!("CARGO_BIN_EXE_fake-cli")),
            version: "fixture".into(),
            enabled: true,
            accept_unsandboxed: true,
        };
        let s = Session {
            id: id(),
            project_id: id(),
            provider: provider.clone(),
            title: "fixture".into(),
            native_id: None,
            model: None,
        };
        let t = Turn {
            id: id(),
            session_id: s.id.clone(),
            text: "fixture".into(),
            assets: vec![],
            chat_id: None,
            user_id: None,
            epoch: "".into(),
            state: "running".into(),
            created_at: now(),
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let (control, controls) = tokio::sync::mpsc::channel(4);
        let task = tokio::spawn(adapter::run(i, s, t, d.path().into(), tx, controls));
        let mut finished = false;
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(e) = rx.recv().await {
                match e {
                    NativeEvent::Request {
                        native_id,
                        kind,
                        data,
                        local_only,
                    } => {
                        assert_eq!(local_only, provider == Provider::Omp);
                        control
                            .send(Control::Respond {
                                request: Interaction {
                                    id: id(),
                                    turn_id: "t".into(),
                                    session_id: "s".into(),
                                    native_id,
                                    kind,
                                    data,
                                    expires_at: now() + 60,
                                    state: "pending".into(),
                                    local_only,
                                },
                                decision: "allow".into(),
                                answers: json!({}),
                            })
                            .await
                            .unwrap();
                    }
                    NativeEvent::Finished(s) => {
                        assert_eq!(s, "completed");
                        finished = true
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        task.await.unwrap().unwrap();
        assert!(finished);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn job_terminates_descendants() {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let d = tempfile::tempdir().unwrap();
    let mut p = Process::spawn(
        std::path::Path::new(env!("CARGO_BIN_EXE_fake-cli")),
        &["--descendants".into()],
        d.path(),
    )
    .unwrap();
    let line = tokio::time::timeout(Duration::from_secs(10), p.lines.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    let pid = v["child_pid"].as_u64().unwrap() as u32;
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    assert!(!handle.is_null());
    p.stop();
    let mut exited = false;
    for _ in 0..50 {
        let mut code = 0;
        unsafe { GetExitCodeProcess(handle, &mut code) };
        if code != 259 {
            exited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    unsafe { CloseHandle(handle) };
    assert!(exited, "Job must terminate owned descendants");
}
