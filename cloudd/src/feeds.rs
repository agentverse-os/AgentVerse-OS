//! Прокси RSS/Atom и погоды для виджетов: браузер не может ходить в чужие сайты из-за CORS, ядро ходит за него,
//! кэширует на 10 минут и не обращается во внутренние сети (SSRF: private/loopback/link-local/overlay-диапазоны).

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FeedItem {
    pub title: String,
    pub link: String,
    pub published: Option<String>,
    pub source: String,
    /// Короткий анонс без HTML (до 240 символов).
    pub summary: Option<String>,
    /// Картинка записи: media:thumbnail / media:content / первый <img> в тексте.
    pub image: Option<String>,
    /// Полный HTML записи, если лента его отдаёт (до 64 КБ; Desktop чистит его DOMPurify перед показом).
    pub content: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Feed {
    pub url: String,
    pub title: String,
    pub items: Vec<FeedItem>,
    pub fetched_at: String,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct Feeds {
    cache: Arc<Mutex<HashMap<String, (std::time::Instant, Feed)>>>,
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => !(v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || v.is_documentation() || (v.octets()[0] == 100 && (64..=127).contains(&v.octets()[1])) || v.octets()[0] == 0),
        IpAddr::V6(v) => !(v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80),
    }
}

/// Проверка URL: только http(s), хост резолвится в публичные адреса.
pub async fn check_url(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url).map_err(|e| anyhow!("имя URL: {e}"))?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("имя URL: только http(s)");
    }
    let host = u.host_str().ok_or_else(|| anyhow!("имя URL: нет хоста"))?.to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|e| anyhow!("имя URL: {host}: {e}"))?;
    let mut any = false;
    for a in addrs {
        any = true;
        if !is_public(a.ip()) {
            bail!("имя URL: {host} указывает во внутреннюю сеть");
        }
    }
    if !any {
        bail!("имя URL: {host} не резолвится");
    }
    Ok(u)
}

impl Feeds {
    pub async fn fetch(&self, url: &str, max_age_s: u64) -> Result<Feed> {
        if let Some((t, f)) = self.cache.lock().ok().and_then(|c| c.get(url).cloned()) {
            if t.elapsed().as_secs() < max_age_s {
                return Ok(f);
            }
        }
        let u = check_url(url).await?;
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(12)).redirect(reqwest::redirect::Policy::limited(3)).user_agent("CloudOS-Desktop/0.1 (+rss widget)").build()?;
        let feed = match client.get(u).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => {
                let bytes = r.bytes().await?;
                if bytes.len() > 4 * 1024 * 1024 { bail!("лента больше 4 MB"); }
                let parsed = feed_rs::parser::parse(&bytes[..]).map_err(|e| anyhow!("разбор ленты: {e}"))?;
                let title = parsed.title.map(|t| t.content).unwrap_or_else(|| url.to_string());
                let items = parsed.entries.into_iter().take(50).map(|e| {
                    let summary_html = e.summary.as_ref().map(|s| s.content.clone());
                    let content_html = e.content.as_ref().and_then(|c| c.body.clone()).filter(|b| !b.trim().is_empty());
                    let mut image = e.media.iter().find_map(|m| {
                        m.thumbnails.first().map(|t| t.image.uri.clone()).or_else(|| m.content.iter().find(|c| c.content_type.as_ref().map(|t| t.to_string().starts_with("image/")).unwrap_or(false)).and_then(|c| c.url.as_ref().map(|u| u.to_string())))
                    });
                    if image.is_none() {
                        image = content_html.as_deref().or(summary_html.as_deref()).and_then(first_img);
                    }
                    let plain_summary = summary_html.as_deref().or(content_html.as_deref()).map(|s| strip_html(s, 240)).filter(|s| !s.is_empty());
                    FeedItem {
                        title: e.title.map(|t| t.content).unwrap_or_default(),
                        link: e.links.first().map(|l| l.href.clone()).unwrap_or_default(),
                        published: e.published.or(e.updated).map(|d| d.to_rfc3339()),
                        source: title.clone(),
                        summary: plain_summary,
                        image,
                        content: content_html.or_else(|| summary_html.filter(|s| s.contains('<'))).map(|c| c.chars().take(64 * 1024).collect()),
                        author: e.authors.first().map(|p| p.name.clone()).filter(|n| !n.is_empty()),
                    }
                }).collect();
                Feed { url: url.into(), title, items, fetched_at: crate::store::now(), error: None }
            }
            Err(e) => Feed { url: url.into(), title: url.into(), items: vec![], fetched_at: crate::store::now(), error: Some(e.to_string()) },
        };
        if let Ok(mut c) = self.cache.lock() {
            c.insert(url.into(), (std::time::Instant::now(), feed.clone()));
            if c.len() > 200 { c.clear(); }
        }
        Ok(feed)
    }

    /// Погода Open-Meteo (без ключа): текущая и прогноз на 5 дней.
    pub async fn weather(&self, lat: f64, lon: f64) -> Result<serde_json::Value> {
        let key = format!("weather:{lat:.2}:{lon:.2}");
        if let Some((t, f)) = self.cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            if t.elapsed().as_secs() < 900 {
                return Ok(serde_json::from_str(&f.title).unwrap_or_default());
            }
        }
        let url = format!("https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,relative_humidity_2m,weather_code,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=5");
        let v: serde_json::Value = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build()?.get(&url).send().await?.error_for_status()?.json().await?;
        if let Ok(mut c) = self.cache.lock() {
            c.insert(key, (std::time::Instant::now(), Feed { url, title: v.to_string(), items: vec![], fetched_at: crate::store::now(), error: None }));
        }
        Ok(v)
    }

    /// Геокодинг города (Open-Meteo).
    pub async fn geocode(&self, q: &str) -> Result<serde_json::Value> {
        let url = format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=5&language=ru&format=json", urlencoding::encode(q));
        Ok(reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build()?.get(&url).send().await?.error_for_status()?.json().await?)
    }
}

/// Первый <img src="…"> в HTML (абсолютный http(s) URL).
fn first_img(html: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r#"<img[^>]+src=["']([^"']+)["']"#).ok()?;
    re.captures(html).map(|c| c[1].to_string()).filter(|u| u.starts_with("http"))
}

fn strip_html(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch { '<' => in_tag = true, '>' => in_tag = false, c if !in_tag => out.push(c), _ => {} }
    }
    let t = out.split_whitespace().collect::<Vec<_>>().join(" ").replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    if t.chars().count() > max { t.chars().take(max).collect::<String>() + "…" } else { t }
}
