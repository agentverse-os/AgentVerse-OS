//! Каталог навыков сервиса (docs/agents-ux.md §7.1). Навык — повторяемая процедура: когда применять, шаги, что проверить.
//! Хранится в kv `skills`; агент получает свои навыки в задании (routines::compose), у Claude они ещё материализуются в
//! `~/.claude/skills/<имя>/SKILL.md` workspace'а (доступны как /имя и в интерактивных сессиях). Агенты сами предлагают
//! навыки строками «НАВЫК: имя: текст» в итоге запуска — такие навыки помечены источником `agent:<id>`; навык владельца
//! (`user`) агент не перезаписывает. Стартовый набор кладётся при первом обращении к пустому каталогу.
use crate::{App, ApiError};
use anyhow::{anyhow, bail, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

const BODY_MAX: usize = 12_000;
const DESC_MAX: usize = 160;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Skill {
    #[serde(default)] pub name: String,
    #[serde(default)] pub description: String,
    #[serde(default)] pub body: String,
    /// user | agent:<id>
    #[serde(default = "s_user")] pub source: String,
    #[serde(default)] pub created_at: i64,
    #[serde(default)] pub updated_at: i64,
}
fn s_user() -> String { "user".into() }
fn now() -> i64 { chrono::Utc::now().timestamp() }

/// Имя навыка: латиница строчными, цифры, дефис; 1–40 символов, не начинается с дефиса.
pub fn valid_name(s: &str) -> bool {
    let n = s.len();
    (1..=40).contains(&n) && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn starters() -> Vec<Skill> {
    let t = now();
    let mk = |name: &str, description: &str, body: &str| Skill { name: name.into(), description: description.into(), body: body.into(), source: "user".into(), created_at: t, updated_at: t };
    vec![
        mk("save-tokens", "экономить контекст и деньги: не перечитывать, не пересказывать", "Применяй всегда. Не перечитывай файлы, которые уже видел в этом запуске; большие файлы читай кусками по нужным строкам; не выводи содержимое файлов и длинные логи в ответ целиком — только нужные строки; не запускай широкие поиски по всему репозиторию, если знаешь, где искать. Итог — коротко, без пересказа процесса."),
        mk("lean-code", "минимальная правка кода без «заодно»", "Применяй при любой правке кода. Меняй только то, что нужно для задачи: без новых абстракций, зависимостей, переименований и переформатирования «заодно». Сохраняй стиль и язык файла. После правки прогони имеющиеся проверки (тесты, линтер, сборку) и назови их в итоге. Если правка требует решения владельца (схема данных, публичный API, деньги, удаление) — остановись и спроси."),
        mk("report", "итог запуска в три пункта", "Применяй в конце каждого запуска. Итог: 1) что сделано (файлы, команды, результат проверок), 2) что не удалось и почему, 3) что проверить или решить владельцу. Один-два абзаца, без пересказа процесса и без вопросов, на которые ты сам можешь ответить. Факты для будущих запусков — отдельными строками «ПАМЯТЬ: …»."),
    ]
}

pub fn load(app: &App) -> Result<Vec<Skill>> {
    match app.store.get_kv("skills")? {
        Some(v) => Ok(serde_json::from_value(v).unwrap_or_default()),
        None => { let s = starters(); save(app, &s)?; Ok(s) }
    }
}
pub fn save(app: &App, v: &[Skill]) -> Result<()> { app.store.put_kv("skills", &serde_json::to_value(v)?) }

/// Навыки агента в порядке его списка; пропавшие из каталога пропускаются.
pub fn for_agent(app: &App, agent: &crate::routines::AgentDef) -> Vec<Skill> {
    let all = load(app).unwrap_or_default();
    agent.skills.iter().filter_map(|n| all.iter().find(|s| &s.name == n).cloned()).collect()
}

/// Навык, предложенный агентом строкой «НАВЫК: имя: текст»: новый — в каталог с источником `agent:<id>`; свой прежний —
/// обновляется; навык владельца (`user`) не трогается.
pub fn upsert_from_agent(app: &App, agent_id: &str, name: &str, text: &str) -> Result<()> {
    if !valid_name(name) { bail!("неверное имя навыка: {name}"); }
    let mut all = load(app)?;
    let src = format!("agent:{agent_id}");
    let (description, body) = split_desc(text);
    match all.iter_mut().find(|s| s.name == name) {
        Some(s) if s.source == "user" => return Ok(()),
        Some(s) => { s.description = description; s.body = body; s.source = src; s.updated_at = now(); }
        None => all.push(Skill { name: name.into(), description, body, source: src, created_at: now(), updated_at: now() }),
    }
    save(app, &all)
}

/// Первая фраза (до точки или 160 символов) — описание, весь текст — тело.
fn split_desc(text: &str) -> (String, String) {
    let t = text.trim();
    let first = t.split_once(['.', '\n']).map(|(a, _)| a).unwrap_or(t).trim();
    let desc: String = first.chars().take(DESC_MAX).collect();
    let body: String = t.chars().take(BODY_MAX).collect();
    (desc, body)
}

/// Содержимое SKILL.md (frontmatter Claude Code + тело).
pub fn skill_md(s: &Skill) -> String {
    let desc = s.description.replace('\n', " ").replace('"', "'");
    format!("---\nname: {}\ndescription: \"{}\"\n---\n\n{}\n", s.name, desc, s.body.trim())
}

/// Разложить навыки агента файлами в `~/.claude/skills/<имя>/SKILL.md` workspace'а (через ядро → agentd fs). Ошибки только в журнал.
pub async fn materialize(app: &App, project: &str, skills: &[Skill]) {
    for s in skills {
        let dir = format!(".claude/skills/{}", s.name);
        if let Err(e) = app.core_post(&format!("/projects/{project}/fs/mkdir"), &json!({ "path": dir })).await { tracing::debug!("навык {}: mkdir {dir}: {e:#}", s.name); }
        let url = format!("{}/api/projects/{project}/fs/file?path={dir}/SKILL.md", app.cloudd);
        match app.http.put(&url).header("content-type", "text/markdown").body(skill_md(s)).send().await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!("навык {} в {project}: {}", s.name, r.status()),
            Err(e) => tracing::warn!("навык {} в {project}: {e}", s.name),
        }
    }
}

// ---------- HTTP ----------

type R = Result<Json<Value>, ApiError>;

pub async fn list(State(app): State<Arc<App>>) -> R {
    let agents = crate::routines::agents_with(&app)?;
    let out: Vec<Value> = load(&app)?.iter().map(|s| {
        let mut v = serde_json::to_value(s).unwrap_or(json!({}));
        v["agents"] = json!(agents.iter().filter(|a| a.skills.iter().any(|n| n == &s.name)).map(|a| a.name.clone()).collect::<Vec<_>>());
        v
    }).collect();
    Ok(Json(json!({ "skills": out })))
}

fn check(s: &mut Skill) -> Result<()> {
    s.name = s.name.trim().to_lowercase();
    if !valid_name(&s.name) { bail!("имя навыка — латиница строчными, цифры и дефис, до 40 символов"); }
    s.description = s.description.trim().chars().take(DESC_MAX).collect();
    s.body = s.body.trim().chars().take(BODY_MAX).collect();
    if s.body.is_empty() { bail!("у навыка нет текста"); }
    if s.description.is_empty() { s.description = split_desc(&s.body).0; }
    Ok(())
}

pub async fn create(State(app): State<Arc<App>>, Json(mut s): Json<Skill>) -> R {
    check(&mut s)?;
    let mut all = load(&app)?;
    if all.iter().any(|x| x.name == s.name) { return Err(ApiError(StatusCode::CONFLICT, format!("навык «{}» уже есть", s.name))); }
    s.source = "user".into(); s.created_at = now(); s.updated_at = now();
    all.push(s.clone()); save(&app, &all)?;
    Ok(Json(serde_json::to_value(s)?))
}

pub async fn update(State(app): State<Arc<App>>, Path(name): Path<String>, Json(patch): Json<Value>) -> R {
    let mut all = load(&app)?;
    let s = all.iter_mut().find(|s| s.name == name).ok_or_else(|| anyhow!("навык не найден: {name}"))?;
    let mut v = serde_json::to_value(&*s)?;
    for (k, val) in patch.as_object().cloned().unwrap_or_default() { if matches!(k.as_str(), "description" | "body") { v[k] = val; } }
    let mut next: Skill = serde_json::from_value(v)?;
    check(&mut next)?;
    next.source = "user".into(); next.updated_at = now();
    *s = next.clone(); save(&app, &all)?;
    Ok(Json(serde_json::to_value(next)?))
}

pub async fn delete(State(app): State<Arc<App>>, Path(name): Path<String>) -> Result<StatusCode, ApiError> {
    let mut all = load(&app)?;
    let before = all.len();
    all.retain(|s| s.name != name);
    if all.len() == before { return Err(anyhow!("навык не найден: {name}").into()); }
    save(&app, &all)?;
    crate::routines::agents_drop_skill(&app, &name)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_md() {
        assert!(valid_name("lean-code") && valid_name("a1") && !valid_name("Lean") && !valid_name("-x") && !valid_name("") && !valid_name(&"a".repeat(41)));
        let (d, b) = split_desc("Сначала npm test. Потом чини.");
        assert_eq!(d, "Сначала npm test"); assert_eq!(b, "Сначала npm test. Потом чини.");
        let md = skill_md(&Skill { name: "x".into(), description: "a \"b\"".into(), body: "тело".into(), ..Default::default() });
        assert!(md.starts_with("---\nname: x\ndescription: \"a 'b'\"\n---\n") && md.ends_with("тело\n"));
        assert_eq!(starters().len(), 3);
    }
}

#[derive(serde::Deserialize)]
pub struct MaterializeIn { pub project: String, #[serde(default)] pub names: Vec<String> }
/// Разложить выбранные навыки каталога в `~/.claude/skills` workspace'а (срез 36: навыки в интерактивной беседе Claude).
pub async fn materialize_api(State(app): State<Arc<App>>, Json(b): Json<MaterializeIn>) -> Result<Json<serde_json::Value>, ApiError> {
    if b.project.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "нужен project".into())) }
    let all = load(&app)?;
    let chosen: Vec<Skill> = all.into_iter().filter(|s| b.names.contains(&s.name)).collect();
    materialize(&app, &b.project, &chosen).await;
    Ok(Json(json!({ "ok": true, "count": chosen.len() })))
}
