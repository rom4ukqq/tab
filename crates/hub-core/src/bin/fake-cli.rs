//! Deterministic protocol fixture executable; never calls a model or external service.
use serde_json::{json, Value};
use std::io::{BufRead, Write};
fn send(v: Value) {
    println!("{v}");
    std::io::stdout().flush().unwrap();
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|s| s == "--worker") {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1))
        }
    }
    if args.iter().any(|s| s == "--descendants") {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--worker")
            .spawn()
            .unwrap();
        send(json!({"child_pid":child.id()}));
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1))
        }
    }
    let provider = if args.iter().any(|s| s == "app-server") {
        "codex"
    } else if args.iter().any(|s| s == "--print") {
        "claude"
    } else {
        "omp"
    };
    if provider == "omp" {
        send(json!({"type":"ready"}))
    }
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        match provider {
            "codex" => match v["method"].as_str().unwrap_or("") {
                "initialize" => send(json!({"id":v["id"],"result":{}})),
                "thread/start" | "thread/resume" => {
                    send(json!({"id":v["id"],"result":{"thread":{"id":"fixture-thread"}}}))
                }
                "turn/start" => {
                    assert_eq!(v["params"]["approvalPolicy"], "on-request");
                    send(json!({"id":v["id"],"result":{"turn":{"id":"fixture-turn"}}}));
                    send(
                        json!({"id":"approval-1","method":"item/commandExecution/requestApproval","params":{"threadId":"fixture-thread","turnId":"fixture-turn","command":"fixture only"}}),
                    );
                }
                _ => {
                    if v["id"] == "approval-1" {
                        assert_eq!(v["result"]["decision"], "accept");
                        send(
                            json!({"method":"item/agentMessage/delta","params":{"delta":"Fixture response"}}),
                        );
                        send(
                            json!({"method":"turn/completed","params":{"turn":{"status":"completed"}}}),
                        );
                    }
                }
            },
            "claude" => match v["type"].as_str().unwrap_or("") {
                "control_request" => send(
                    json!({"type":"control_response","response":{"subtype":"success","request_id":v["request_id"],"response":{}}}),
                ),
                "user" => {
                    send(json!({"type":"system","session_id":"fixture-session"}));
                    send(
                        json!({"type":"control_request","request_id":"permission-1","request":{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":"fixture.txt"}}}),
                    );
                }
                "control_response" => {
                    assert_eq!(v["response"]["response"]["behavior"], "allow");
                    send(
                        json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Fixture response"}}}),
                    );
                    send(json!({"type":"result","is_error":false,"result":"Fixture response"}));
                }
                _ => {}
            },
            _ => match v["type"].as_str().unwrap_or("") {
                "get_state" => send(
                    json!({"type":"response","id":v["id"],"success":true,"data":{"sessionFile":"fixture-session"}}),
                ),
                "prompt" => {
                    send(json!({"type":"response","id":v["id"],"success":true}));
                    send(
                        json!({"type":"extension_ui_request","id":"dialog-1","method":"confirm","title":"Fixture only"}),
                    );
                }
                "extension_ui_response" => {
                    assert_eq!(v["confirmed"], true);
                    send(
                        json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Fixture response"}}),
                    );
                    send(json!({"type":"agent_end"}));
                }
                _ => {}
            },
        }
    }
}
