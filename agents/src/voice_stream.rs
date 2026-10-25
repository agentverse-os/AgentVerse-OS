//! Streaming recognition — text as you speak (docs/agents-ux.md §4.7, slice 72): a WebSocket proxy browser ↔ service ↔
//! provider (ElevenLabs Scribe v2 Realtime, OpenAI Realtime transcription). The provider key never leaves the server; sound
//! and text are not logged. Protocol with the browser: the first frame `{"kind":"ready","rate",…}`, from the browser —
//! binary PCM16 mono frames and a text `{"kind":"stop"}`, back — `partial` / `final` (followed by close) / `error`.
use crate::voice::{self, VoiceCfg};
use crate::App;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::Message as TMsg;

/// Simultaneous streams per service: one owner dictates, the margin is for tabs and the
/// phone
pub const MAX_STREAMS: usize = 4;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Two minutes of recording plus a margin
/// for the result
const MAX_SESSION: Duration = Duration::from_secs(150);
const FINAL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
pub struct StreamQuery { pub lang: Option<String> }

pub async fn stream(State(app): State<Arc<App>>, Query(q): Query<StreamQuery>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |sock| run(app, sock, q.lang.unwrap_or_default()))
}

/// Stream provider: address, headers, rate and message shapes
#[derive(Clone, Debug)]
pub struct Upstream { pub url: String, pub headers: Vec<(String, String)>, pub rate: u32, pub provider: String, pub model: String, pub lang: String }

/// `https://…` → `wss://…`, `http://…` → `ws://…` (the mock on 127.0.0.1 — no
/// TLS)
pub fn ws_base(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if let Some(r) = b.strip_prefix("https://") { format!("wss://{r}") } else if let Some(r) = b.strip_prefix("http://") { format!("ws://{r}") } else { b.to_string() }
}

pub fn upstream_for(cfg: &VoiceCfg, lang: &str) -> anyhow::Result<Upstream> {
    let p = cfg.provider_cfg();
    if p.key.trim().is_empty() { anyhow::bail!("ключ не задан") }
    let lang = if lang.trim().is_empty() { cfg.language.trim().to_lowercase() } else { lang.trim().to_lowercase() };
    let model = voice::stream_model(&cfg.provider, &p.stream_model);
    if cfg.provider == "openai" {
        Ok(Upstream { url: format!("{}/realtime?intent=transcription", ws_base(&voice::base_or(&p.base, voice::OPENAI_BASE))), headers: vec![("authorization".into(), format!("Bearer {}", p.key.trim()))], rate: 24_000, provider: "openai".into(), model, lang })
    } else {
        let lang_q = if lang.is_empty() { String::new() } else { format!("&language_code={lang}") };
        Ok(Upstream { url: format!("{}/v1/speech-to-text/realtime?model_id={model}&audio_format=pcm_16000&commit_strategy=manual{lang_q}", ws_base(&voice::base_or(&p.base, voice::ELEVEN_BASE))), headers: vec![("xi-api-key".into(), p.key.trim().to_string())], rate: 16_000, provider: "elevenlabs".into(), model, lang })
    }
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

impl Upstream {
    fn label(&self) -> &'static str { if self.provider == "openai" { "OpenAI" } else { "ElevenLabs" } }
    /// First message after connecting: OpenAI configures the session, ElevenLabs — everything
    /// in the query
    pub fn hello(&self) -> Option<Value> {
        if self.provider != "openai" { return None }
        let legacy = self.model.starts_with("gpt-4o") || self.model.starts_with("whisper");
        let mut tr = json!({ "model": self.model });
        if !self.lang.is_empty() { if legacy { tr["language"] = json!(self.lang); } else { tr["languages"] = json!([self.lang]); } }
        Some(json!({ "type": "session.update", "session": { "type": "transcription", "audio": { "input": { "format": { "type": "audio/pcm", "rate": self.rate }, "transcription": tr, "turn_detection": Value::Null } } } }))
    }
    pub fn chunk(&self, pcm: &[u8]) -> Value {
        let b64 = B64.encode(pcm);
        if self.provider == "openai" { json!({ "type": "input_audio_buffer.append", "audio": b64 }) }
        else { json!({ "message_type": "input_audio_chunk", "audio_base_64": b64, "commit": false, "sample_rate": self.rate }) }
    }
    /// End of speech: OpenAI — commit the buffer, ElevenLabs — a 100 ms silence chunk with
    /// `commit: true`
    pub fn commit(&self) -> Value {
        if self.provider == "openai" { json!({ "type": "input_audio_buffer.commit" }) }
        else { let silence = vec![0u8; (self.rate as usize / 10) * 2]; json!({ "message_type": "input_audio_chunk", "audio_base_64": B64.encode(&silence), "commit": true, "sample_rate": self.rate }) }
    }
}

/// Provider message → an event for the
/// browser
#[derive(Debug, PartialEq)]
pub enum Ev { Partial(String), Final(String), Error(String), Ignore }

pub fn parse_upstream(provider: &str, text: &str, acc: &mut String) -> Ev {
    let v: Value = match serde_json::from_str(text) { Ok(v) => v, Err(_) => return Ev::Ignore };
    let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    if provider == "openai" {
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "conversation.item.input_audio_transcription.delta" => { acc.push_str(&s(&v, "delta")); Ev::Partial(acc.clone()) }
            "conversation.item.input_audio_transcription.completed" => Ev::Final(s(&v, "transcript")),
            "conversation.item.input_audio_transcription.failed" | "error" => Ev::Error(format!("OpenAI: {}", v.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("распознавание не удалось"))),
            _ => Ev::Ignore,
        }
    } else {
        let mt = v.get("message_type").and_then(|t| t.as_str()).unwrap_or("");
        match mt {
            "partial_transcript" => Ev::Partial(s(&v, "text")),
            "committed_transcript" | "committed_transcript_with_timestamps" => Ev::Final(s(&v, "text")),
            _ if v.get("error").is_some() => Ev::Error(format!("ElevenLabs: {mt}: {}", s(&v, "error"))),
            _ => Ev::Ignore,
        }
    }
}

/// What comes from the browser
pub enum In { Audio(Vec<u8>), Stop, Closed }

fn short_err(label: &str, e: &tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error as E;
    match e {
        E::Http(r) => { let body = r.body().as_ref().map(|b| String::from_utf8_lossy(b).chars().take(140).collect::<String>()).unwrap_or_default(); format!("{label}: {} {}", r.status().as_u16(), body.trim()) }
        other => { let t = other.to_string(); format!("{label}: {}", t.chars().take(160).collect::<String>()) }
    }
}

/// Bridge: connect to the provider and pump frames until a result, an error, the limit or the browser closes.
/// All messages to the browser go through `out`; returns `false` if the browser dropped before the result.
pub async fn bridge(up: Upstream, mut inp: mpsc::Receiver<In>, out: mpsc::Sender<Value>) -> bool {
    let tell = |v: Value| { let out = out.clone(); async move { out.send(v).await.is_ok() } };
    let err = |text: String| json!({ "kind": "error", "text": text });
    let mut req = match up.url.clone().into_client_request() { Ok(r) => r, Err(e) => { tell(err(format!("адрес провайдера: {e}"))).await; return false } };
    for (k, v) in &up.headers {
        match (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) { (Ok(k), Ok(v)) => { req.headers_mut().insert(k, v); } _ => { tell(err("заголовок провайдера не собрался".into())).await; return false } }
    }
    let upstream = match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(req)).await {
        Ok(Ok((s, _))) => s,
        Ok(Err(e)) => { tell(err(short_err(up.label(), &e))).await; return false }
        Err(_) => { tell(err(format!("{}: не ответил за 10 с", up.label()))).await; return false }
    };
    let (mut up_tx, mut up_rx) = upstream.split();
    if let Some(h) = up.hello() { if up_tx.send(TMsg::Text(h.to_string().into())).await.is_err() { tell(err(format!("{}: закрыл соединение", up.label()))).await; return false } }
    if !tell(json!({ "kind": "ready", "rate": up.rate, "provider": up.provider, "model": up.model })).await { return false }
    let deadline = tokio::time::sleep(MAX_SESSION); tokio::pin!(deadline);
    let final_sleep = tokio::time::sleep(Duration::from_secs(3600)); tokio::pin!(final_sleep);
    let mut acc = String::new(); let mut committed = false; let mut browser_alive = true;
    loop {
        tokio::select! {
            m = inp.recv() => match m {
                Some(In::Audio(b)) => { if committed { continue } if up_tx.send(TMsg::Text(up.chunk(&b).to_string().into())).await.is_err() { tell(err(format!("{}: закрыл соединение", up.label()))).await; break } }
                Some(In::Stop) => { if committed { continue } committed = true; let _ = up_tx.send(TMsg::Text(up.commit().to_string().into())).await; final_sleep.as_mut().reset(tokio::time::Instant::now() + FINAL_TIMEOUT); }
                Some(In::Closed) | None => { browser_alive = false; break }
            },
            m = up_rx.next() => match m {
                Some(Ok(TMsg::Text(t))) => match parse_upstream(&up.provider, &t, &mut acc) {
                    Ev::Partial(x) => { if !tell(json!({ "kind": "partial", "text": x })).await { browser_alive = false; break } }
                    Ev::Final(x) => { tell(json!({ "kind": "final", "text": x })).await; break }
                    Ev::Error(x) => { tell(err(x)).await; break }
                    Ev::Ignore => {}
                },
                Some(Ok(TMsg::Close(_))) | None => { tell(err(format!("{}: закрыл соединение", up.label()))).await; break }
                Some(Ok(_)) => {}
                Some(Err(e)) => { tell(err(short_err(up.label(), &e))).await; break }
            },
            _ = &mut deadline => { tell(err("предел длительности потока — 2,5 минуты".into())).await; break }
            _ = &mut final_sleep => { tell(err(format!("{}: не прислал итог за 20 с", up.label()))).await; break }
        }
    }
    let _ = up_tx.close().await;
    browser_alive
}

async fn tell_sock(sock: &mut WebSocket, v: Value) -> bool { sock.send(Message::Text(v.to_string().into())).await.is_ok() }

async fn run(app: Arc<App>, mut sock: WebSocket, lang: String) {
    let cfg = voice::load(&app);
    if !cfg.stream { tell_sock(&mut sock, json!({ "kind": "error", "text": "потоковое распознавание выключено в настройках" })).await; let _ = sock.close().await; return }
    let up = match upstream_for(&cfg, &lang) { Ok(u) => u, Err(e) => { tell_sock(&mut sock, json!({ "kind": "error", "text": e.to_string() })).await; let _ = sock.close().await; return } };
    let Ok(permit) = app.voice_streams.clone().try_acquire_owned() else { tell_sock(&mut sock, json!({ "kind": "error", "text": "слишком много одновременных диктовок" })).await; let _ = sock.close().await; return };
    let (in_tx, in_rx) = mpsc::channel::<In>(64);
    let (out_tx, mut out_rx) = mpsc::channel::<Value>(64);
    let bridge_task = tokio::spawn(bridge(up, in_rx, out_tx));
    // The end of the bridge is seen as `out_rx` closing (None) — after all its messages, including the last
    // `final`/`error`, reached the browser; there is deliberately no separate branch for task completion, otherwise the
    // last frame was lost
    loop {
        tokio::select! {
            m = sock.recv() => match m {
                Some(Ok(Message::Binary(b))) => { if in_tx.send(In::Audio(b.to_vec())).await.is_err() { break } }
                Some(Ok(Message::Text(t))) => { if t.contains("\"stop\"") && in_tx.send(In::Stop).await.is_err() { break } }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => { let _ = in_tx.send(In::Closed).await; break }
                Some(Ok(_)) => {}
            },
            v = out_rx.recv() => match v {
                Some(v) => { let last = matches!(v.get("kind").and_then(|k| k.as_str()), Some("final") | Some("error")); if !tell_sock(&mut sock, v).await || last { break } }
                None => break,
            },
        }
    }
    drop(in_tx);
    let _ = sock.close().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), bridge_task).await;
    drop(permit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::ProviderCfg;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::Mutex;
    use tokio::net::TcpListener;

    /// Provider mock: accepts a WebSocket, remembers the headers and messages, answers by a
    /// script. `script(n, msg)` — what to answer to the client's n-th message (a list of
    /// answers).
    async fn mock_ws(seen: Arc<Mutex<Vec<String>>>, headers: Arc<Mutex<Vec<(String, String)>>>, script: fn(usize, &Value) -> Vec<Value>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = l.accept().await {
                let seen = seen.clone(); let headers = headers.clone();
                tokio::spawn(async move {
                    let hdrs = headers.clone();
                    let cb = move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, resp: tokio_tungstenite::tungstenite::handshake::server::Response| {
                        let mut h = hdrs.lock().unwrap();
                        h.push(("path".into(), req.uri().to_string()));
                        for (k, v) in req.headers() { h.push((k.to_string(), v.to_str().unwrap_or("").to_string())); }
                        Ok(resp)
                    };
                    let Ok(ws) = tokio_tungstenite::accept_hdr_async(tcp, cb).await else { return };
                    let (mut tx, mut rx) = ws.split();
                    let mut n = 0;
                    while let Some(Ok(m)) = rx.next().await {
                        if let TMsg::Text(t) = m {
                            let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                            seen.lock().unwrap().push(t.to_string());
                            for r in script(n, &v) { if tx.send(TMsg::Text(r.to_string().into())).await.is_err() { return } }
                            n += 1;
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    fn eleven_script(n: usize, v: &Value) -> Vec<Value> {
        let commit = v.get("commit").and_then(|c| c.as_bool()).unwrap_or(false);
        if commit { return vec![json!({ "message_type": "committed_transcript", "text": "привет, мир" })] }
        if n == 0 { return vec![json!({ "message_type": "session_started", "session_id": "x" }), json!({ "message_type": "partial_transcript", "text": "привет" })] }
        vec![json!({ "message_type": "partial_transcript", "text": "привет, ми" })]
    }
    fn eleven_fail(_n: usize, _v: &Value) -> Vec<Value> { vec![json!({ "message_type": "quota_exceeded", "error": "characters quota exceeded" })] }
    fn openai_script(n: usize, v: &Value) -> Vec<Value> {
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session.update" => vec![json!({ "type": "session.updated" })],
            "input_audio_buffer.append" => if n == 1 { vec![json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": "hello" })] } else { vec![json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": " world" })] },
            "input_audio_buffer.commit" => vec![json!({ "type": "input_audio_buffer.committed" }), json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": "hello world" })],
            _ => vec![],
        }
    }

    fn cfg(provider: &str, base: String, stream_model: &str) -> VoiceCfg {
        let p = ProviderCfg { key: "k-secret-123456".into(), base, stream_model: stream_model.into(), ..Default::default() };
        VoiceCfg { provider: provider.into(), language: "ru".into(), elevenlabs: if provider == "elevenlabs" { p.clone() } else { Default::default() }, openai: if provider == "openai" { p } else { Default::default() }, stream: true }
    }

    async fn drive(up: Upstream, audio: &[&[u8]]) -> Vec<Value> {
        let (in_tx, in_rx) = mpsc::channel(16); let (out_tx, mut out_rx) = mpsc::channel(16);
        let task = tokio::spawn(bridge(up, in_rx, out_tx));
        let mut got = vec![];
        // wait for ready, then send sound and stop
        let first = tokio::time::timeout(Duration::from_secs(5), out_rx.recv()).await.unwrap().unwrap(); got.push(first.clone());
        if first.get("kind").and_then(|k| k.as_str()) == Some("ready") {
            for a in audio { in_tx.send(In::Audio(a.to_vec())).await.unwrap(); }
            in_tx.send(In::Stop).await.unwrap();
        }
        while let Ok(Some(v)) = tokio::time::timeout(Duration::from_secs(5), out_rx.recv()).await { got.push(v); }
        let _ = task.await;
        got
    }

    #[tokio::test]
    async fn elevenlabs_stream_partial_then_final() {
        let seen = Arc::new(Mutex::new(vec![])); let headers = Arc::new(Mutex::new(vec![]));
        let base = mock_ws(seen.clone(), headers.clone(), eleven_script).await;
        let up = upstream_for(&cfg("elevenlabs", base, ""), "").unwrap();
        assert_eq!(up.rate, 16_000); assert!(up.url.starts_with("ws://127.0.0.1:"), "{}", up.url);
        let got = drive(up, &[&[1, 2, 3, 4], &[5, 6, 7, 8]]).await;
        let kinds: Vec<String> = got.iter().map(|v| v["kind"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["ready", "partial", "partial", "final"], "{got:?}");
        assert_eq!(got[0]["rate"], 16_000); assert_eq!(got[0]["model"], "scribe_v2_realtime");
        assert_eq!(got[1]["text"], "привет"); assert_eq!(got[3]["text"], "привет, мир");
        let h = headers.lock().unwrap().clone();
        let path = h.iter().find(|(k, _)| k == "path").map(|(_, v)| v.clone()).unwrap_or_default();
        assert!(path.contains("/v1/speech-to-text/realtime?model_id=scribe_v2_realtime&audio_format=pcm_16000&commit_strategy=manual&language_code=ru"), "{path}");
        assert!(h.iter().any(|(k, v)| k == "xi-api-key" && v == "k-secret-123456"), "{h:?}");
        assert!(!h.iter().any(|(k, _)| k == "authorization"));
        let msgs = seen.lock().unwrap().clone();
        assert_eq!(msgs.len(), 3, "{msgs:?}");
        let m0: Value = serde_json::from_str(&msgs[0]).unwrap();
        assert_eq!(m0["message_type"], "input_audio_chunk"); assert_eq!(m0["commit"], false); assert_eq!(m0["sample_rate"], 16_000); assert_eq!(m0["audio_base_64"], B64.encode([1u8, 2, 3, 4]));
        let m2: Value = serde_json::from_str(&msgs[2]).unwrap();
        assert_eq!(m2["commit"], true); assert_eq!(B64.decode(m2["audio_base_64"].as_str().unwrap()).unwrap().len(), 3200, "чанк тишины 100 мс");
    }

    #[tokio::test]
    async fn elevenlabs_provider_error_is_reported() {
        let seen = Arc::new(Mutex::new(vec![])); let headers = Arc::new(Mutex::new(vec![]));
        let base = mock_ws(seen, headers, eleven_fail).await;
        let got = drive(upstream_for(&cfg("elevenlabs", base, ""), "en").unwrap(), &[&[0, 0]]).await;
        let last = got.last().unwrap();
        assert_eq!(last["kind"], "error"); assert_eq!(last["text"], "ElevenLabs: quota_exceeded: characters quota exceeded");
    }

    #[tokio::test]
    async fn openai_session_update_append_commit() {
        let seen = Arc::new(Mutex::new(vec![])); let headers = Arc::new(Mutex::new(vec![]));
        let base = mock_ws(seen.clone(), headers.clone(), openai_script).await;
        let up = upstream_for(&cfg("openai", format!("{base}/v1"), ""), "").unwrap();
        assert_eq!(up.rate, 24_000); assert!(up.url.ends_with("/v1/realtime?intent=transcription"), "{}", up.url);
        let got = drive(up, &[&[9, 9], &[8, 8]]).await;
        let kinds: Vec<String> = got.iter().map(|v| v["kind"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["ready", "partial", "partial", "final"], "{got:?}");
        assert_eq!(got[2]["text"], "hello world", "дельты копятся"); assert_eq!(got[3]["text"], "hello world");
        let h = headers.lock().unwrap().clone();
        assert!(h.iter().any(|(k, v)| k == "authorization" && v == "Bearer k-secret-123456"), "{h:?}");
        let msgs = seen.lock().unwrap().clone();
        let m0: Value = serde_json::from_str(&msgs[0]).unwrap();
        assert_eq!(m0["type"], "session.update"); assert_eq!(m0["session"]["type"], "transcription");
        assert_eq!(m0["session"]["audio"]["input"]["format"]["rate"], 24_000); assert_eq!(m0["session"]["audio"]["input"]["transcription"]["model"], "gpt-live-transcribe");
        assert_eq!(m0["session"]["audio"]["input"]["transcription"]["languages"], json!(["ru"])); assert!(m0["session"]["audio"]["input"]["turn_detection"].is_null());
        let m1: Value = serde_json::from_str(&msgs[1]).unwrap();
        assert_eq!(m1["type"], "input_audio_buffer.append"); assert_eq!(m1["audio"], B64.encode([9u8, 9]));
        let m3: Value = serde_json::from_str(&msgs[3]).unwrap();
        assert_eq!(m3["type"], "input_audio_buffer.commit");
    }

    #[test]
    fn legacy_openai_model_uses_language_and_no_key_fails() {
        let up = upstream_for(&cfg("openai", "https://api.openai.com/v1".into(), "gpt-4o-transcribe"), "de").unwrap();
        assert_eq!(up.url, "wss://api.openai.com/v1/realtime?intent=transcription");
        let h = up.hello().unwrap();
        assert_eq!(h["session"]["audio"]["input"]["transcription"]["language"], "de"); assert!(h["session"]["audio"]["input"]["transcription"].get("languages").is_none());
        let e = upstream_for(&VoiceCfg { stream: true, ..Default::default() }, "").unwrap_err();
        assert_eq!(e.to_string(), "ключ не задан");
        let up = upstream_for(&cfg("elevenlabs", String::new(), "whatever"), "").unwrap();
        assert_eq!(up.url, "wss://api.elevenlabs.io/v1/speech-to-text/realtime?model_id=scribe_v2_realtime&audio_format=pcm_16000&commit_strategy=manual&language_code=ru", "у ElevenLabs одна модель потока");
    }

    #[tokio::test]
    async fn unreachable_provider_is_an_error_frame() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap(); let addr = l.local_addr().unwrap(); drop(l);
        let got = drive(upstream_for(&cfg("elevenlabs", format!("http://{addr}"), ""), "").unwrap(), &[]).await;
        assert_eq!(got.len(), 1); assert_eq!(got[0]["kind"], "error"); assert!(got[0]["text"].as_str().unwrap().starts_with("ElevenLabs: "), "{got:?}");
    }

    #[test]
    fn parse_upstream_shapes() {
        let mut acc = String::new();
        assert_eq!(parse_upstream("elevenlabs", r#"{"message_type":"warning","warning":"x"}"#, &mut acc), Ev::Ignore);
        assert_eq!(parse_upstream("elevenlabs", r#"{"message_type":"committed_transcript_with_timestamps","text":"a b","words":[]}"#, &mut acc), Ev::Final("a b".into()));
        assert_eq!(parse_upstream("elevenlabs", r#"{"message_type":"auth_error","error":"bad key"}"#, &mut acc), Ev::Error("ElevenLabs: auth_error: bad key".into()));
        assert_eq!(parse_upstream("openai", r#"{"type":"error","error":{"message":"invalid model"}}"#, &mut acc), Ev::Error("OpenAI: invalid model".into()));
        assert_eq!(parse_upstream("openai", "not json", &mut acc), Ev::Ignore);
    }
}
