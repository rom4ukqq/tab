use crate::domain::{Event, EventKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Activity {
    pub turn_id: String,
    pub started_at: i64,
    pub updated_at: i64,
    pub phase: String,
    pub files: BTreeSet<String>,
    pending_files: HashMap<String, Vec<String>>,
}

impl Activity {
    pub fn update(&mut self, e: &Event) {
        if self.turn_id != e.turn_id {
            *self = Self { turn_id: e.turn_id.clone(), started_at: e.occurred_at, ..Self::default() };
        }
        self.updated_at = e.occurred_at;
        let d = &e.data;
        match e.kind {
            EventKind::ReasoningSummary => self.phase = "Думает".into(),
            EventKind::Message => self.phase = "Пишет ответ".into(),
            EventKind::ApprovalRequest => self.phase = "Ждёт подтверждения".into(),
            EventKind::UserQuestion => self.phase = "Ждёт вашего ответа".into(),
            EventKind::ToolCall => {
                let name = d["name"].as_str().or(d["toolName"].as_str()).or(d["type"].as_str()).unwrap_or("инструмент");
                let lower = name.to_ascii_lowercase();
                self.phase = if lower.contains("edit") || lower.contains("write") || lower.contains("patch") || name == "fileChange" {
                    "Редактирует файлы".into()
                } else if lower.contains("read") || lower.contains("search") { "Читает и ищет".into() }
                else { "Выполняет инструмент".into() };
                // Only tool calls with explicit file arguments can be attributed.
                if ["edit", "write", "multiedit", "write_file", "edit_file"].contains(&lower.as_str()) {
                    let input = d.get("input").or(d.get("args")).unwrap_or(&Value::Null);
                    let path = input["file_path"].as_str().or(input["path"].as_str());
                    let id = d["id"].as_str().or(d["toolCallId"].as_str());
                    if let (Some(id), Some(path)) = (id, path) { self.pending_files.insert(id.into(), vec![path.into()]); }
                }
            }
            EventKind::ToolResult => {
                let failed = d["is_error"] == true || d["isError"] == true || d["status"] == "failed";
                if let Some(id) = d["tool_use_id"].as_str().or(d["toolCallId"].as_str()).or(d["id"].as_str()) {
                    if let Some(paths) = self.pending_files.remove(id) { if !failed { self.files.extend(paths); } }
                }
                if d["type"] == "fileChange" && d["status"] == "completed" {
                    for change in d["changes"].as_array().into_iter().flatten() {
                        if let Some(path) = change["path"].as_str() { self.files.insert(path.into()); }
                    }
                }
                self.phase = if failed { "Инструмент сообщил об ошибке" } else { "Думает" }.into();
            }
            EventKind::Error => self.phase = "Ошибка".into(),
            EventKind::Completed => self.phase = match d["state"].as_str().unwrap_or("") {
                "completed" => "Готово", "cancelled" => "Остановлено", "failed" => "Ошибка", _ => "Результат не подтверждён",
            }.into(),
            EventKind::Progress if d["text"] == "Задача запущена" => self.phase = "Запускается".into(),
            EventKind::Progress if d["text"] == "Агент думает" => self.phase = "Думает".into(),
            _ => {}
        }
    }

    pub fn label(&self, at: i64) -> String {
        let seconds = (at - self.started_at).max(0);
        format!("{} · {} мин {} сек", self.phase, seconds / 60, seconds % 60)
    }
}
