//! Voice: dictation into the input field (docs/agents-ux.md §4.7). Recognition settings — provider, model, language, keys —
//! live in the service kv under the `voice` key, separately from `settings` (that one goes to the browser whole); keys are not
//! exposed, only the tail. `POST /voice/transcribe` accepts audio from MediaRecorder and returns the provider's text:
//! ElevenLabs Scribe (`/v1/speech-to-text`) or an OpenAI-compatible `/audio/transcriptions` (OpenAI, Groq, a proxy — by API
//! address).
use crate::{App, ApiError};
use anyhow::{anyhow, bail, Result};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Audio body limit: OpenAI has 25 MB, two minutes of opus is about a
/// megabyte.
pub const MAX_AUDIO: usize = 25 * 1024 * 1024;
pub const ELEVEN_BASE: &str = "https://api.elevenlabs.io";
pub const OPENAI_BASE: &str = "https://api.openai.com/v1";
const ELEVEN_MODELS: &[&str] = &["scribe_v2", "scribe_v1"];
const OPENAI_MODELS: &[&str] = &["gpt-4o-transcribe", "gpt-4o-mini-transcribe", "gpt-4o-transcribe-diarize", "whisper-1"];

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ProviderCfg {
    #[serde(default)] pub model: String,
    #[serde(default)] pub key: String,
    /// API address: for OpenAI-compatible ones — the base with `/v1` (Groq, a proxy), for ElevenLabs — the root;
    /// empty = default
    #[serde(default)] pub base: String,
    /// Answer read-aloud (slice 64): the synthesis model and voice; empty — the
    /// provider defaults
    #[serde(default)] pub tts_model: String,
    #[serde(default)] pub tts_voice: String,
    /// Streaming recognition (slice 72): the realtime model; empty — the provider
    /// default
    #[serde(default)] pub stream_model: String,
}
/// Streaming recognition models: ElevenLabs has one, OpenAI — the live model by default (gpt-4o-transcribe is accepted too)
pub const ELEVEN_STREAM_MODEL: &str = "scribe_v2_realtime";
pub const OPENAI_STREAM_MODELS: &[&str] = &["gpt-live-transcribe", "gpt-transcribe", "gpt-4o-transcribe", "gpt-4o-mini-transcribe"];
pub fn stream_model(provider: &str, m: &str) -> String { let m = m.trim(); if provider == "openai" { if m.is_empty() { OPENAI_STREAM_MODELS[0].into() } else { m.to_string() } } else { ELEVEN_STREAM_MODEL.into() } }
/// The first one is the default: multilingual_v2 reads Russian noticeably cleaner than flash; flash is faster
/// and for short phrases
const ELEVEN_TTS_MODELS: &[&str] = &["eleven_multilingual_v2", "eleven_flash_v2_5", "eleven_v3", "eleven_turbo_v2_5"];
const OPENAI_TTS_MODELS: &[&str] = &["gpt-4o-mini-tts", "tts-1", "tts-1-hd"];
const OPENAI_VOICES: &[&str] = &["alloy", "ash", "ballad", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer"];
/// Default voices — female, calm (the owner's request, 2026-10-16): ElevenLabs "Sarah" from the presets (every account has
/// it; Russian voices from the library the owner picks in the settings), OpenAI — "nova"
const ELEVEN_DEFAULT_VOICE: &str = "EXAVITQu4vr4xnSDxMaL";
const ELEVEN_DEFAULT_VOICE_NAME: &str = "Sarah";
const OPENAI_DEFAULT_VOICE: &str = "nova";
/// Read-aloud text limit: ElevenLabs characters cost money, and nobody will listen to a long answer read
/// aloud anyway
pub const MAX_SPEAK_CHARS: usize = 2000;
pub fn tts_model(provider: &str, m: &str) -> String { let m = m.trim(); if !m.is_empty() { m.to_string() } else if provider == "openai" { OPENAI_TTS_MODELS[0].into() } else { ELEVEN_TTS_MODELS[0].into() } }
pub fn tts_voice(provider: &str, v: &str) -> String { let v = v.trim(); if !v.is_empty() { v.to_string() } else if provider == "openai" { OPENAI_DEFAULT_VOICE.into() } else { ELEVEN_DEFAULT_VOICE.into() } }
#[derive(Clone, Serialize, Deserialize)]
pub struct VoiceCfg {
    #[serde(default = "default_provider")] pub provider: String,
    /// ISO-639-1 language code or empty —
    /// auto-detection
    #[serde(default)] pub language: String,
    #[serde(default)] pub elevenlabs: ProviderCfg,
    #[serde(default)] pub openai: ProviderCfg,
    /// Text as you speak (slice 72): streaming recognition instead of batch; off by default — more expensive and not at every provider
    #[serde(default)] pub stream: bool,
}
fn default_provider() -> String { "elevenlabs".into() }
impl Default for VoiceCfg { fn default() -> Self { Self { provider: default_provider(), language: String::new(), elevenlabs: ProviderCfg::default(), openai: ProviderCfg::default(), stream: false } } }
impl VoiceCfg {
    pub fn provider_cfg(&self) -> &ProviderCfg { if self.provider == "openai" { &self.openai } else { &self.elevenlabs } }
    pub fn ready(&self) -> bool { !self.provider_cfg().key.trim().is_empty() }
    pub fn model(&self) -> String { effective_model(&self.provider, &self.provider_cfg().model) }
}
pub fn effective_model(provider: &str, model: &str) -> String {
    let m = model.trim();
    if !m.is_empty() { return m.to_string(); }
    if provider == "openai" { OPENAI_MODELS[0].into() } else { ELEVEN_MODELS[0].into() }
}
pub fn base_or(base: &str, default: &str) -> String { let b = base.trim().trim_end_matches('/'); if b.is_empty() { default.to_string() } else { b.to_string() } }
fn key_tail(key: &str) -> String { let k = key.trim(); if k.len() < 6 { String::new() } else { k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect() } }

pub fn load(app: &App) -> VoiceCfg {
    app.store.get_kv("voice").ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}
fn save(app: &App, cfg: &VoiceCfg) -> Result<()> { app.store.put_kv("voice", &serde_json::to_value(cfg)?) }

/// Public view of the settings: without keys (only the tail and a flag), with readiness and the
/// model catalog.
pub fn public(cfg: &VoiceCfg) -> Value {
    let prov = |p: &ProviderCfg, default_base: &str| json!({ "model": p.model, "base": p.base, "default_base": default_base, "has_key": !p.key.trim().is_empty(), "key_tail": key_tail(&p.key), "tts_model": p.tts_model, "tts_voice": p.tts_voice, "stream_model": p.stream_model });
    json!({
        "provider": cfg.provider, "language": cfg.language, "ready": cfg.ready(), "model": cfg.model(),
        "stream": cfg.stream, "stream_model": stream_model(&cfg.provider, &cfg.provider_cfg().stream_model), "stream_models": { "elevenlabs": [ELEVEN_STREAM_MODEL], "openai": OPENAI_STREAM_MODELS },
        "elevenlabs": prov(&cfg.elevenlabs, ELEVEN_BASE), "openai": prov(&cfg.openai, OPENAI_BASE),
        "models": { "elevenlabs": ELEVEN_MODELS, "openai": OPENAI_MODELS },
        "tts": { "model": tts_model(&cfg.provider, &cfg.provider_cfg().tts_model), "voice": tts_voice(&cfg.provider, &cfg.provider_cfg().tts_voice), "max_chars": MAX_SPEAK_CHARS,
                 "models": { "elevenlabs": ELEVEN_TTS_MODELS, "openai": OPENAI_TTS_MODELS }, "openai_voices": OPENAI_VOICES,
                 "default_voice": { "elevenlabs": ELEVEN_DEFAULT_VOICE_NAME, "openai": OPENAI_DEFAULT_VOICE } },
    })
}

pub async fn get(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> { Ok(Json(public(&load(&app)))) }

#[derive(Deserialize, Default)]
pub struct ProviderPatch { model: Option<String>, base: Option<String>, key: Option<String>, #[serde(default)] clear: bool, tts_model: Option<String>, tts_voice: Option<String>, stream_model: Option<String> }
#[derive(Deserialize)]
pub struct Patch { provider: Option<String>, language: Option<String>, elevenlabs: Option<ProviderPatch>, openai: Option<ProviderPatch>, stream: Option<bool> }

fn apply(p: &mut ProviderCfg, patch: &ProviderPatch) {
    if let Some(m) = &patch.model { p.model = m.trim().to_string(); }
    if let Some(b) = &patch.base { p.base = b.trim().to_string(); }
    if patch.clear { p.key.clear(); }
    if let Some(k) = &patch.key { if !k.trim().is_empty() { p.key = k.trim().to_string(); } }
    if let Some(m) = &patch.tts_model { p.tts_model = m.trim().to_string(); }
    if let Some(v) = &patch.tts_voice { p.tts_voice = v.trim().to_string(); }
    if let Some(m) = &patch.stream_model { p.stream_model = m.trim().to_string(); }
}
pub async fn put(State(app): State<Arc<App>>, Json(b): Json<Patch>) -> Result<Json<Value>, ApiError> {
    let mut cfg = load(&app);
    if let Some(p) = &b.provider {
        if p != "elevenlabs" && p != "openai" { return Err(ApiError(StatusCode::BAD_REQUEST, "провайдер: elevenlabs или openai".into())); }
        cfg.provider = p.clone();
    }
    if let Some(l) = &b.language {
        let l = l.trim().to_lowercase();
        if !l.is_empty() && !(l.len() <= 3 && l.chars().all(|c| c.is_ascii_alphabetic())) { return Err(ApiError(StatusCode::BAD_REQUEST, "язык: код ISO-639-1 (ru, en, …) или пусто".into())); }
        cfg.language = l;
    }
    if let Some(p) = &b.elevenlabs { apply(&mut cfg.elevenlabs, p); }
    if let Some(p) = &b.openai { apply(&mut cfg.openai, p); }
    if let Some(s) = b.stream { cfg.stream = s; }
    save(&app, &cfg)?;
    Ok(Json(public(&cfg)))
}

#[derive(Deserialize)]
pub struct CheckBody { provider: Option<String> }
/// Key check: ElevenLabs — the subscription (tier, characters), OpenAI-compatible — the model
/// list.
pub async fn check(State(app): State<Arc<App>>, Json(b): Json<CheckBody>) -> Result<Json<Value>, ApiError> {
    let cfg = load(&app);
    let provider = b.provider.unwrap_or_else(|| cfg.provider.clone());
    let p = if provider == "openai" { &cfg.openai } else { &cfg.elevenlabs };
    if p.key.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "ключ не задан".into())); }
    let v = check_with(&app.http, &provider, p).await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(v))
}
pub async fn check_with(http: &reqwest::Client, provider: &str, p: &ProviderCfg) -> Result<Value> {
    if provider == "openai" {
        let base = base_or(&p.base, OPENAI_BASE);
        let r = http.get(format!("{base}/models")).bearer_auth(p.key.trim()).timeout(Duration::from_secs(20)).send().await.map_err(|e| anyhow!("OpenAI: {e}"))?;
        let v = provider_json("OpenAI", r).await?;
        let n = v.get("data").and_then(|d| d.as_array()).map(|a| a.len()).unwrap_or(0);
        let stt = v.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str())).filter(|id| id.contains("transcribe") || id.contains("whisper")).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
        Ok(json!({ "ok": true, "provider": "openai", "models": n, "stt_models": stt }))
    } else {
        let base = base_or(&p.base, ELEVEN_BASE);
        let r = http.get(format!("{base}/v1/user/subscription")).header("xi-api-key", p.key.trim()).timeout(Duration::from_secs(20)).send().await.map_err(|e| anyhow!("ElevenLabs: {e}"))?;
        let v = provider_json("ElevenLabs", r).await?;
        Ok(json!({ "ok": true, "provider": "elevenlabs", "tier": v.get("tier").cloned().unwrap_or(Value::Null), "used": v.get("character_count").cloned().unwrap_or(Value::Null), "limit": v.get("character_limit").cloned().unwrap_or(Value::Null), "reset_unix": v.get("next_character_count_reset_unix").cloned().unwrap_or(Value::Null) }))
    }
}

/// Provider voices to choose from in the settings: ElevenLabs — `GET /v1/voices` (name, id, language/accent from labels),
/// OpenAI — a list.
pub async fn voices(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    let cfg = load(&app);
    if cfg.provider == "openai" { return Ok(Json(json!({ "provider": "openai", "voices": OPENAI_VOICES.iter().map(|v| json!({ "id": v, "name": v })).collect::<Vec<_>>() }))); }
    let p = &cfg.elevenlabs;
    if p.key.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "ключ ElevenLabs не задан".into())); }
    let base = base_or(&p.base, ELEVEN_BASE);
    let r = app.http.get(format!("{base}/v1/voices")).header("xi-api-key", p.key.trim()).timeout(Duration::from_secs(20)).send().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("ElevenLabs: {e}")))?;
    let v = provider_json("ElevenLabs", r).await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    let list: Vec<Value> = v.get("voices").and_then(|a| a.as_array()).cloned().unwrap_or_default().into_iter().filter_map(|x| {
        let id = x.get("voice_id")?.as_str()?.to_string(); let name = x.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let lab = x.get("labels").cloned().unwrap_or(json!({}));
        let desc = ["language", "accent", "gender", "age"].iter().filter_map(|k| lab.get(*k).and_then(|v| v.as_str()).filter(|v| !v.is_empty())).collect::<Vec<_>>().join(" · ");
        Some(json!({ "id": id, "name": name, "desc": desc, "category": x.get("category") }))
    }).collect();
    Ok(Json(json!({ "provider": "elevenlabs", "voices": list })))
}

#[derive(Deserialize)]
pub struct SpeakBody { text: String }
/// Text read-aloud (slice 64): markdown is stripped, the text is cut to MAX_SPEAK_CHARS, the provider returns mp3
/// as is.
pub async fn speak(State(app): State<Arc<App>>, Json(b): Json<SpeakBody>) -> Result<axum::response::Response, ApiError> {
    let cfg = load(&app);
    let p = cfg.provider_cfg();
    if p.key.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "ключ провайдера озвучки не задан".into())); }
    let text = speech_text(&b.text);
    if text.is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "нечего озвучивать".into())); }
    let (bytes, mime) = speak_with(&app.http, &cfg.provider, p, &text).await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    let mut out = axum::response::Response::new(axum::body::Body::from(bytes));
    out.headers_mut().insert(axum::http::header::CONTENT_TYPE, mime.parse().unwrap_or_else(|_| "audio/mpeg".parse().unwrap()));
    out.headers_mut().insert("x-speech-chars", text.chars().count().to_string().parse().unwrap());
    Ok(out)
}
pub async fn speak_with(http: &reqwest::Client, provider: &str, p: &ProviderCfg, text: &str) -> Result<(Bytes, String)> {
    let model = tts_model(provider, &p.tts_model); let voice = tts_voice(provider, &p.tts_voice);
    let r = if provider == "openai" {
        let base = base_or(&p.base, OPENAI_BASE);
        http.post(format!("{base}/audio/speech")).bearer_auth(p.key.trim()).timeout(Duration::from_secs(60)).json(&json!({ "model": model, "voice": voice, "input": text, "response_format": "mp3" })).send().await.map_err(|e| anyhow!("OpenAI: {e}"))?
    } else {
        let base = base_or(&p.base, ELEVEN_BASE);
        http.post(format!("{base}/v1/text-to-speech/{voice}?output_format=mp3_44100_128")).header("xi-api-key", p.key.trim()).timeout(Duration::from_secs(60)).json(&json!({ "text": text, "model_id": model })).send().await.map_err(|e| anyhow!("ElevenLabs: {e}"))?
    };
    let status = r.status();
    let mime = r.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("audio/mpeg").to_string();
    if !status.is_success() {
        let text = r.text().await.unwrap_or_default();
        let v: Option<Value> = serde_json::from_str(&text).ok();
        let msg = v.as_ref().and_then(|v| v.pointer("/detail/message").or_else(|| v.pointer("/error/message")).and_then(|m| m.as_str()).map(str::to_string)).unwrap_or_else(|| text.chars().take(300).collect());
        bail!("{}: {} {}", if provider == "openai" { "OpenAI" } else { "ElevenLabs" }, status.as_u16(), msg.trim());
    }
    Ok((r.bytes().await?, if mime.starts_with("audio/") { mime } else { "audio/mpeg".into() }))
}
/// Text for synthesis: no markdown (code blocks — "(код)", links — the label, markers and headings removed), whitespace
/// collapsed, no longer than MAX_SPEAK_CHARS (cut at a word boundary with an ellipsis).
pub fn speech_text(md: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("```") { if !in_code { out.push_str(" (код) "); } in_code = !in_code; continue; }
        if in_code { continue; }
        let mut l = t.trim_start_matches('#').trim_start().to_string();
        for pre in ["- ", "* ", "+ ", "> "] { if l.starts_with(pre) { l = l[pre.len()..].to_string(); } }
        if l.starts_with('|') { l = l.trim_matches('|').split('|').map(str::trim).filter(|c| !c.is_empty() && !c.chars().all(|ch| ch == '-' || ch == ':')).collect::<Vec<_>>().join(", "); }
        // links and images → the label; `code` → without backticks; **bold** / *italic* → text
        let mut s = String::new(); let chars: Vec<char> = l.chars().collect(); let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if (c == '[' || (c == '!' && i + 1 < chars.len() && chars[i + 1] == '[')) && l[..].contains("](") {
                let start = if c == '!' { i + 2 } else { i + 1 };
                if let Some(close) = chars[start..].iter().position(|&x| x == ']') { let end = start + close; if end + 1 < chars.len() && chars[end + 1] == '(' { if let Some(p) = chars[end + 1..].iter().position(|&x| x == ')') { s.extend(&chars[start..end]); i = end + 1 + p + 1; continue; } } }
            }
            if c == '`' || c == '*' || c == '_' && (i == 0 || !chars[i - 1].is_alphanumeric()) { i += 1; continue; }
            s.push(c); i += 1;
        }
        out.push_str(s.trim()); out.push(' ');
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_SPEAK_CHARS { return collapsed }
    let cut: String = collapsed.chars().take(MAX_SPEAK_CHARS).collect();
    let cut = match cut.rfind(' ') { Some(i) if i > MAX_SPEAK_CHARS / 2 => cut[..i].to_string(), _ => cut };
    format!("{cut}…")
}

#[derive(Deserialize)]
pub struct TranscribeQuery { #[serde(default)] lang: String }
/// Audio from MediaRecorder → text. The language from the request overrides the setting; empty —
/// auto-detection.
pub async fn transcribe(State(app): State<Arc<App>>, Query(q): Query<TranscribeQuery>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, ApiError> {
    let cfg = load(&app);
    let p = cfg.provider_cfg();
    if p.key.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "ключ не задан".into())); }
    if body.is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "пустая запись".into())); }
    if body.len() > MAX_AUDIO { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "запись больше 25 МБ".into())); }
    let mime = headers.get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.split(';').next().unwrap_or("").trim().to_string()).filter(|s| s.starts_with("audio/") || s.starts_with("video/")).unwrap_or_else(|| "audio/webm".into());
    let lang = if q.lang.trim().is_empty() { cfg.language.clone() } else { q.lang.trim().to_lowercase() };
    let t0 = Instant::now();
    let (text, language) = transcribe_with(&app.http, &cfg.provider, p, body, &mime, &lang).await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(json!({ "text": text, "language": language, "ms": t0.elapsed().as_millis() as u64, "provider": cfg.provider, "model": cfg.model() })))
}

fn ext_of(mime: &str) -> &'static str {
    match mime { "audio/webm" | "video/webm" => "webm", "audio/mp4" | "video/mp4" | "audio/x-m4a" | "audio/m4a" => "mp4", "audio/ogg" | "audio/opus" => "ogg", "audio/wav" | "audio/x-wav" | "audio/wave" => "wav", "audio/mpeg" | "audio/mp3" => "mp3", "audio/flac" => "flac", _ => "webm" }
}
/// Provider request: multipart with the file and model, the language — if set. Returns the text and the detected language (if the provider
/// reports it).
pub async fn transcribe_with(http: &reqwest::Client, provider: &str, p: &ProviderCfg, audio: Bytes, mime: &str, lang: &str) -> Result<(String, Option<String>)> {
    let part = reqwest::multipart::Part::bytes(audio.to_vec()).file_name(format!("audio.{}", ext_of(mime))).mime_str(mime).map_err(|e| anyhow!("тип аудио: {e}"))?;
    let model = effective_model(provider, &p.model);
    if provider == "openai" {
        let base = base_or(&p.base, OPENAI_BASE);
        let mut form = reqwest::multipart::Form::new().text("model", model).text("response_format", "json");
        if !lang.is_empty() { form = form.text("language", lang.to_string()); }
        let form = form.part("file", part); // the file last: text fields are read before the large body
        let r = http.post(format!("{base}/audio/transcriptions")).bearer_auth(p.key.trim()).multipart(form).timeout(Duration::from_secs(120)).send().await.map_err(|e| anyhow!("OpenAI: {e}"))?;
        let v = provider_json("OpenAI", r).await?;
        let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
        Ok((text, v.get("language").and_then(|l| l.as_str()).map(str::to_string)))
    } else {
        let base = base_or(&p.base, ELEVEN_BASE);
        let mut form = reqwest::multipart::Form::new().text("model_id", model).text("tag_audio_events", "false").text("diarize", "false").text("timestamps_granularity", "none");
        if !lang.is_empty() { form = form.text("language_code", lang.to_string()); }
        let form = form.part("file", part);
        let r = http.post(format!("{base}/v1/speech-to-text")).header("xi-api-key", p.key.trim()).multipart(form).timeout(Duration::from_secs(120)).send().await.map_err(|e| anyhow!("ElevenLabs: {e}"))?;
        let v = provider_json("ElevenLabs", r).await?;
        let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
        Ok((text, v.get("language_code").and_then(|l| l.as_str()).map(str::to_string)))
    }
}
/// Provider answer: non-2xx — an error with its text (OpenAI `error.message`, ElevenLabs `detail.message` or
/// `detail`).
async fn provider_json(name: &str, r: reqwest::Response) -> Result<Value> {
    let status = r.status();
    let text = r.text().await.unwrap_or_default();
    if !status.is_success() {
        let v: Option<Value> = serde_json::from_str(&text).ok();
        let msg = v.as_ref().and_then(|v| v.get("error").and_then(|e| e.get("message").and_then(|m| m.as_str()).map(str::to_string).or_else(|| e.as_str().map(str::to_string))))
            .or_else(|| v.as_ref().and_then(|v| v.get("detail")).map(|d| d.get("message").and_then(|m| m.as_str()).map(str::to_string).unwrap_or_else(|| d.as_str().map(str::to_string).unwrap_or_else(|| d.to_string()))))
            .unwrap_or_else(|| text.chars().take(300).collect());
        bail!("{name}: {} {}", status.as_u16(), msg.trim());
    }
    serde_json::from_str(&text).map_err(|e| anyhow!("{name}: неверный JSON ответа: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn speech_text_strips_markdown_and_caps() {
        let t = super::speech_text("## Итог\n\n- **Готово**: файл `a.rs` [в репо](https://x) и ![схема](img.png)\n\n```rust\nfn main() {}\n```\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert_eq!(t, "Итог Готово: файл a.rs в репо и схема (код) a, b 1, 2");
        let long = "слово ".repeat(1000);
        let c = super::speech_text(&long); assert!(c.chars().count() <= super::MAX_SPEAK_CHARS + 1 && c.ends_with('…'));
        assert_eq!(super::tts_model("elevenlabs", ""), "eleven_multilingual_v2"); assert_eq!(super::tts_voice("openai", " nova "), "nova");
        assert_eq!(super::tts_voice("openai", ""), "nova"); assert_eq!(super::tts_voice("elevenlabs", ""), "EXAVITQu4vr4xnSDxMaL");
    }
    use super::*;
    use axum::routing::{get, post};
    use axum::Router;
    use std::sync::Mutex;

    /// Provider mock: remembers the headers and body of the last request, answers with a
    /// canned response.
    struct Seen { auth: String, xi: String, body: String }
    async fn mock(reply: Value, status: u16, seen: Arc<Mutex<Option<Seen>>>) -> String {
        let st = StatusCode::from_u16(status).unwrap();
        let handler = move |headers: HeaderMap, body: Bytes| {
            let seen = seen.clone(); let reply = reply.clone();
            async move {
                *seen.lock().unwrap() = Some(Seen { auth: headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").into(), xi: headers.get("xi-api-key").and_then(|v| v.to_str().ok()).unwrap_or("").into(), body: String::from_utf8_lossy(&body).into_owned() });
                (st, Json(reply))
            }
        };
        let app = Router::new().route("/v1/speech-to-text", post(handler.clone())).route("/v1/audio/transcriptions", post(handler)).route("/v1/models", get(|| async { Json(json!({ "data": [{ "id": "whisper-1" }, { "id": "gpt-4o-transcribe" }, { "id": "gpt-4o" }] })) })).route("/v1/user/subscription", get(|| async { Json(json!({ "tier": "starter", "character_count": 120, "character_limit": 30000 })) }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap(); });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn elevenlabs_multipart_and_text() {
        let seen = Arc::new(Mutex::new(None));
        let base = mock(json!({ "text": "  привет, мир ", "language_code": "ru" }), 200, seen.clone()).await;
        let p = ProviderCfg { model: String::new(), key: "k-eleven-123456".into(), base, ..Default::default() };
        let (text, lang) = transcribe_with(&reqwest::Client::new(), "elevenlabs", &p, Bytes::from_static(b"OPUS"), "audio/webm", "ru").await.unwrap();
        assert_eq!(text, "привет, мир"); assert_eq!(lang.as_deref(), Some("ru"));
        let s = seen.lock().unwrap().take().unwrap();
        assert_eq!(s.xi, "k-eleven-123456"); assert!(s.auth.is_empty());
        for needle in ["name=\"model_id\"", "scribe_v2", "name=\"language_code\"", "name=\"file\"; filename=\"audio.webm\"", "Content-Type: audio/webm", "name=\"diarize\"", "OPUS"] { assert!(s.body.contains(needle), "нет {needle} в {}", s.body); }
        assert!(s.body.find("name=\"language_code\"").unwrap() < s.body.find("name=\"file\"").unwrap(), "текстовые поля идут до файла");
    }

    #[tokio::test]
    async fn openai_compatible_multipart() {
        let seen = Arc::new(Mutex::new(None));
        let base = mock(json!({ "text": "hello" }), 200, seen.clone()).await;
        let p = ProviderCfg { model: "whisper-1".into(), key: "sk-test-123456".into(), base: format!("{base}/v1/"), ..Default::default() };
        let (text, lang) = transcribe_with(&reqwest::Client::new(), "openai", &p, Bytes::from_static(b"MP4A"), "audio/mp4", "").await.unwrap();
        assert_eq!(text, "hello"); assert!(lang.is_none());
        let s = seen.lock().unwrap().take().unwrap();
        assert_eq!(s.auth, "Bearer sk-test-123456");
        for needle in ["name=\"model\"", "whisper-1", "name=\"response_format\"", "filename=\"audio.mp4\"", "MP4A"] { assert!(s.body.contains(needle), "нет {needle}"); }
        assert!(!s.body.contains("name=\"language\""), "пустой язык не отправляется");
    }

    #[tokio::test]
    async fn provider_error_is_readable() {
        let seen = Arc::new(Mutex::new(None));
        let base = mock(json!({ "detail": { "status": "invalid_api_key", "message": "Invalid API key" } }), 401, seen).await;
        let p = ProviderCfg { model: String::new(), key: "bad-key-000000".into(), base, ..Default::default() };
        let e = transcribe_with(&reqwest::Client::new(), "elevenlabs", &p, Bytes::from_static(b"x"), "audio/webm", "").await.unwrap_err();
        assert_eq!(format!("{e:#}"), "ElevenLabs: 401 Invalid API key");
        let seen = Arc::new(Mutex::new(None));
        let base = mock(json!({ "error": { "message": "Incorrect API key provided", "type": "invalid_request_error" } }), 401, seen).await;
        let p = ProviderCfg { model: String::new(), key: "sk-bad-000000".into(), base: format!("{base}/v1"), ..Default::default() };
        let e = transcribe_with(&reqwest::Client::new(), "openai", &p, Bytes::from_static(b"x"), "audio/webm", "").await.unwrap_err();
        assert_eq!(format!("{e:#}"), "OpenAI: 401 Incorrect API key provided");
    }

    #[tokio::test]
    async fn check_reports_subscription_and_models() {
        let base = mock(json!({}), 200, Arc::new(Mutex::new(None))).await;
        let v = check_with(&reqwest::Client::new(), "elevenlabs", &ProviderCfg { model: String::new(), key: "k-123456".into(), base: base.clone(), ..Default::default() }).await.unwrap();
        assert_eq!(v["tier"], "starter"); assert_eq!(v["limit"], 30000);
        let v = check_with(&reqwest::Client::new(), "openai", &ProviderCfg { model: String::new(), key: "sk-123456".into(), base: format!("{base}/v1"), ..Default::default() }).await.unwrap();
        assert_eq!(v["models"], 3); assert_eq!(v["stt_models"], json!(["whisper-1", "gpt-4o-transcribe"]));
    }

    #[test]
    fn public_view_hides_keys() {
        let mut cfg = VoiceCfg::default();
        cfg.elevenlabs.key = "k-very-secret-ab12".into();
        let v = public(&cfg);
        assert_eq!(v["ready"], true); assert_eq!(v["elevenlabs"]["key_tail"], "ab12"); assert_eq!(v["elevenlabs"]["has_key"], true); assert_eq!(v["openai"]["has_key"], false);
        assert!(!v.to_string().contains("very-secret"));
        assert_eq!(v["model"], "scribe_v2");
        cfg.provider = "openai".into();
        assert_eq!(public(&cfg)["ready"], false); assert_eq!(public(&cfg)["model"], "gpt-4o-transcribe");
        assert_eq!(key_tail("short"), "");
    }
}
