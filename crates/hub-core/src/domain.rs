use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Claude,
    Omp,
    Mock,
}
impl Provider {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::Mock => "mock",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installation {
    pub provider: Provider,
    pub executable: PathBuf,
    pub version: String,
    pub enabled: bool,
    pub accept_unsandboxed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub machine_id: String,
    pub machine_name: String,
    pub installations: Vec<Installation>,
    pub projects: Vec<Project>,
    pub telegram_user: Option<i64>,
    pub telegram_enabled: bool,
    pub auth_epoch: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            machine_id: id(),
            machine_name: std::env::var("COMPUTERNAME").unwrap_or("Мой ПК".into()),
            installations: vec![],
            projects: vec![],
            telegram_user: None,
            telegram_enabled: false,
            auth_epoch: id(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub provider: Provider,
    pub title: String,
    pub native_id: Option<String>,
    pub model: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub mime: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub session_id: String,
    pub text: String,
    pub assets: Vec<Asset>,
    pub chat_id: Option<i64>,
    pub user_id: Option<i64>,
    pub epoch: String,
    pub state: String,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Message,
    ReasoningSummary,
    ToolCall,
    ToolResult,
    ApprovalRequest,
    UserQuestion,
    Progress,
    File,
    Image,
    Error,
    Completed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub sequence: i64,
    pub kind: EventKind,
    pub data: Value,
    pub occurred_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Interaction {
    pub id: String,
    pub turn_id: String,
    pub session_id: String,
    pub native_id: Value,
    pub kind: String,
    pub data: Value,
    pub expires_at: i64,
    pub state: String,
    pub local_only: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct PairingCode {
    pub code: String,
    pub expires_at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub config: Config,
    pub sessions: Vec<Session>,
    pub turns: Vec<Turn>,
    pub requests: Vec<Interaction>,
    pub telegram_status: String,
    pub telegram_active: bool,
    pub telegram_authorized: bool,
    pub pairing_code: Option<PairingCode>,
}
#[derive(Clone, Debug)]
pub enum Control {
    Cancel,
    Respond {
        request: Interaction,
        decision: String,
        answers: Value,
    },
    Steer(String),
}
#[derive(Clone, Debug)]
pub enum NativeEvent {
    Event(EventKind, Value),
    Request {
        native_id: Value,
        kind: String,
        data: Value,
        local_only: bool,
    },
    Session(String),
    Finished(String),
}
