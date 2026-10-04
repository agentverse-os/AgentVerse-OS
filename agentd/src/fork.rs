//! Форк беседы (ADR-006, срез «правка и форк» по образцу CloudCLI): у Claude — копия транскрипта под новым id сессии с
//! переписыванием `uuid` / `parentUuid` / `sessionId` (то же, что делает `forkSession` Agent SDK), у Codex — `thread/fork`
//! app-server. Наш журнал копируется до выбранного места. Новая запись не запускается — возобновляется из чата.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Копия транскрипта Claude Code до строки с `uuid == anchor` включительно (или целиком) под новым id.
/// Возвращает путь новой копии и число строк.
pub fn copy_claude_transcript(src: &Path, new_id: &str, anchor: Option<&str>) -> Result<(PathBuf, u64)> {
    let text = std::fs::read_to_string(src).with_context(|| format!("чтение {}", src.display()))?;
    let dst = src.with_file_name(format!("{new_id}.jsonl"));
    if dst.exists() { bail!("транскрипт {} уже есть", dst.display()); }
    let mut map: HashMap<String, String> = HashMap::new();
    let mut out = String::new();
    let mut lines = 0u64;
    let mut found = anchor.is_none();
    for line in text.lines() {
        let Ok(mut v) = serde_json::from_str::<Value>(line) else { continue };
        let old_uuid = v.get("uuid").and_then(|u| u.as_str()).map(str::to_string);
        if let Some(u) = &old_uuid {
            let fresh = uuid::Uuid::new_v4().to_string();
            map.insert(u.clone(), fresh.clone());
            v["uuid"] = Value::String(fresh);
        }
        if let Some(p) = v.get("parentUuid").and_then(|p| p.as_str()).map(str::to_string) {
            v["parentUuid"] = match map.get(&p) { Some(m) => Value::String(m.clone()), None => Value::Null };
        }
        if v.get("sessionId").is_some() { v["sessionId"] = Value::String(new_id.to_string()); }
        out.push_str(&v.to_string()); out.push('\n'); lines += 1;
        if let (Some(a), Some(u)) = (anchor, old_uuid.as_deref()) { if u == a { found = true; break; } }
    }
    if !found { bail!("в транскрипте нет сообщения {}", anchor.unwrap_or("")); }
    if lines == 0 { bail!("транскрипт пуст"); }
    std::fs::write(&dst, out).with_context(|| format!("запись {}", dst.display()))?;
    // каталог с большими результатами инструментов — рядом, копируем как есть
    let side = src.with_extension("");
    if side.is_dir() {
        let _ = std::process::Command::new("cp").args(["-a", &side.to_string_lossy(), &dst.with_extension("").to_string_lossy()]).status();
    }
    Ok((dst, lines))
}

/// uuid последнего ответа ассистента в журнале не позже `at_seq` — точка, до которой сохраняется история.
pub fn anchor_uuid(events: &[Value], at_seq: u64) -> Option<String> {
    events.iter().rev().filter(|e| e.get("seq").and_then(|s| s.as_u64()).unwrap_or(u64::MAX) <= at_seq)
        .find(|e| e.get("kind").and_then(|k| k.as_str()) == Some("assistant") && e.get("parent_tool_use_id").map(|p| p.is_null()).unwrap_or(true) && e.get("uuid").and_then(|u| u.as_str()).is_some())
        .and_then(|e| e.get("uuid").and_then(|u| u.as_str()).map(str::to_string))
}

pub fn ensure_json_lines(events: &[Value], upto: Option<u64>) -> String {
    events.iter().filter(|e| upto.map(|u| e.get("seq").and_then(|s| s.as_u64()).unwrap_or(0) <= u).unwrap_or(true)).map(|e| e.to_string() + "\n").collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn transcript_copy_remaps_uuids_and_cuts_at_anchor() {
        let dir = std::env::temp_dir().join(format!("fork-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("old.jsonl");
        let lines = [
            json!({ "type": "user", "uuid": "u1", "parentUuid": null, "sessionId": "old", "message": { "role": "user", "content": "hi" } }),
            json!({ "type": "assistant", "uuid": "a1", "parentUuid": "u1", "sessionId": "old", "message": { "role": "assistant", "content": [] } }),
            json!({ "type": "user", "uuid": "u2", "parentUuid": "a1", "sessionId": "old", "message": { "role": "user", "content": "more" } }),
        ];
        std::fs::write(&src, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
        let (dst, n) = copy_claude_transcript(&src, "new", Some("a1")).unwrap();
        assert_eq!(n, 2);
        let out: Vec<Value> = std::fs::read_to_string(&dst).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(out[0]["sessionId"], "new");
        assert_ne!(out[0]["uuid"], "u1");
        assert_eq!(out[1]["parentUuid"], out[0]["uuid"]);
        assert!(copy_claude_transcript(&src, "new2", Some("zzz")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn anchor_is_last_top_level_assistant_before_seq() {
        let ev = vec![json!({ "seq": 1, "kind": "user" }), json!({ "seq": 2, "kind": "assistant", "uuid": "a1" }), json!({ "seq": 3, "kind": "assistant", "uuid": "sub", "parent_tool_use_id": "t" }), json!({ "seq": 4, "kind": "user" })];
        assert_eq!(anchor_uuid(&ev, 3).as_deref(), Some("a1"));
        assert_eq!(anchor_uuid(&ev, 1), None);
        assert_eq!(ensure_json_lines(&ev, Some(2)).lines().count(), 2);
    }
}
