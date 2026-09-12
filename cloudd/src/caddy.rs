//! Edge Caddy: `cloudd` владеет конфигурацией целиком и загружает её через Admin API (`POST /load`, идемпотентно; 3.1).
//! Один hostname, режимы `path` и `port` (2.2). TLS — внутренний CA Caddy (NetBird) или сертификат от tailscaled (Tailscale).

use crate::model::{AppState, RouteMode};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct EdgeCaddy {
    admin: String,
    http: reqwest::Client,
}

/// Срок листовых сертификатов внутреннего CA. По умолчанию Caddy выпускает их на 12 часов и перевыпускает каждые ~8: браузер без
/// установленного корня (исключение «принять риск», Firefox) теряет исключение при каждом перевыпуске — оболочка открывается из кеша,
/// а запросы к API отклоняются. 90 дней и подпись корнем (без промежуточного, живущего 7 дней) делают исключение стабильным.
pub const LEAF_LIFETIME: &str = "2160h";

pub struct EdgeInput<'a> {
    pub host: &'a str,
    /// Дополнительные имена/IP того же Entry Point: те же маршруты и сертификат (internal CA умеет IP SAN).
    pub alt_hosts: &'a [String],
    pub cloudd_upstream: &'a str,
    pub coder_upstream: &'a str,
    pub coder_port: u16,
    pub apps: &'a [AppState],
    /// `internal` — внутренний CA; `auto` — обычный ACME/tailscale (без явного issuer).
    pub tls_internal: bool,
}

impl EdgeCaddy {
    pub fn new(admin: &str) -> Self {
        Self { admin: admin.trim_end_matches('/').to_string(), http: reqwest::Client::new() }
    }

    pub async fn ping(&self) -> Result<()> {
        let r = self.http.get(format!("{}/config/", self.admin)).send().await.context("caddy admin")?;
        if !r.status().is_success() {
            return Err(anyhow!("caddy admin: HTTP {}", r.status()));
        }
        Ok(())
    }

    pub async fn load(&self, config: &Value) -> Result<()> {
        let r = self.http.post(format!("{}/load", self.admin)).json(config).send().await.context("caddy /load")?;
        if !r.status().is_success() {
            return Err(anyhow!("caddy /load: HTTP {}: {}", r.status(), r.text().await.unwrap_or_default()));
        }
        Ok(())
    }

    /// Полная JSON-конфигурация edge.
    pub fn render(input: &EdgeInput<'_>) -> Value {
        let host = input.host;
        let mut hosts: Vec<String> = vec![host.to_string()];
        hosts.extend(input.alt_hosts.iter().filter(|h| !h.is_empty() && h.as_str() != host).cloned());
        let origins = hosts.iter().map(|h| format!("https://{h}")).collect::<Vec<_>>().join(" ");
        let mut servers = serde_json::Map::new();

        // ---- Desktop + cloudd API (:443), плюс path-приложения под /apps/<name>/ ----
        let mut desktop_routes = Vec::new();
        for app in input.apps.iter().filter(|a| a.manifest.route.mode == RouteMode::Path) {
            let prefix = app.manifest.route.prefix.clone().unwrap_or_else(|| format!("/apps/{}/", app.name));
            let prefix = prefix.trim_end_matches('/').to_string();
            desktop_routes.push(json!({
                "match": [{ "host": hosts.clone(), "path": [format!("{prefix}/*"), prefix.clone()] }],
                "handle": [ frame_ancestors_rewrite(&origins), reverse_proxy(&format!("{}:{}", app.manifest.endpoint.service, app.manifest.endpoint.port)) ],
                "terminal": true
            }));
        }
        desktop_routes.push(json!({
            "match": [{ "host": hosts.clone() }],
            "handle": [
                { "handler": "headers", "response": { "set": {
                    "X-Frame-Options": ["DENY"], "Content-Security-Policy": ["frame-ancestors 'none'"] }, "delete": ["Server"] } },
                // каталог Store — 2 МБ JSON; через WireGuard с MTU 1200 без сжатия это секунды
                { "handler": "encode", "encodings": { "gzip": {}, "zstd": {} }, "prefer": ["zstd", "gzip"], "minimum_length": 1024 },
                reverse_proxy(input.cloudd_upstream)
            ],
            "terminal": true
        }));
        servers.insert("desktop".into(), json!({ "listen": [":443"], "routes": desktop_routes }));

        // ---- Coder: route: port. Coder ставит `frame-ancestors 'self'` (origin :8444) — расширяем до origin Desktop,
        // чтобы VS Code Web и терминал открывались в окнах Desktop (3.12: точечная перезапись только на origin Desktop'а).
        servers.insert(
            "coder".into(),
            json!({ "listen": [format!(":{}", input.coder_port)], "routes": [ { "match": [{ "host": hosts.clone() }], "handle": [
                { "handler": "headers", "response": { "deferred": true, "delete": ["X-Frame-Options"],
                    "replace": { "Content-Security-Policy": [ { "search_regexp": "frame-ancestors 'self'", "replace": format!("frame-ancestors 'self' {origins}") } ] } } },
                reverse_proxy(input.coder_upstream)
            ], "terminal": true } ] }),
        );

        // ---- приложения route: port — отдельный origin на порт ----
        for app in input.apps.iter().filter(|a| a.manifest.route.mode == RouteMode::Port) {
            let Some(port) = app.port else { continue };
            servers.insert(
                format!("app-{}", app.name),
                json!({ "listen": [format!(":{port}")], "routes": [ { "match": [{ "host": hosts.clone() }], "handle": [
                    frame_ancestors_rewrite(&origins),
                    reverse_proxy(&format!("{}:{}", app.manifest.endpoint.service, app.manifest.endpoint.port))
                ], "terminal": true } ],
                "errors": error_page(host, &app.name, app.manifest.title.as_deref().unwrap_or(&app.name)) }),
            );
        }

        // Клиент без SNI (браузер по IP-адресу) получает сертификат IP: default_sni на всех серверах. Иначе Caddy отвечает alert internal error.
        if let Some(ip) = hosts.iter().find(|h| h.parse::<std::net::IpAddr>().is_ok()) {
            for (_, srv) in servers.iter_mut() {
                srv["tls_connection_policies"] = json!([{ "default_sni": ip }]);
            }
        }
        // Только h1/h2: контейнер edge публикует лишь TCP, а заголовок Alt-Svc h3 заставляет браузеры (особенно Firefox) пробовать QUIC и ждать
        for (_, srv) in servers.iter_mut() {
            srv["protocols"] = json!(["h1", "h2"]);
        }

        let mut cfg = json!({
            "admin": { "listen": "0.0.0.0:2019" },
            "apps": { "http": { "servers": Value::Object(servers) } }
        });
        if input.tls_internal {
            cfg["apps"]["tls"] = json!({ "automation": { "policies": [ { "subjects": hosts.clone(), "issuers": [ { "module": "internal", "lifetime": LEAF_LIFETIME, "sign_with_root": true } ] } ] } });
        } else {
            // Tailscale: для имени машины (<box>.<tailnet>.ts.net) Caddy сам берёт сертификат Let's Encrypt у tailscaled через
            // /var/run/tailscale/tailscaled.sock — политику для него не задаём. Запасные имена и IP публичного сертификата не получат:
            // им остаётся внутренний CA (с исключением в браузере).
            let others: Vec<String> = hosts.iter().skip(1).cloned().collect();
            if !others.is_empty() {
                cfg["apps"]["tls"] = json!({ "automation": { "policies": [ { "subjects": others, "issuers": [ { "module": "internal", "lifetime": LEAF_LIFETIME, "sign_with_root": true } ] } ] } });
            }
        }
        cfg
    }
}

fn reverse_proxy(upstream: &str) -> Value {
    json!({ "handler": "reverse_proxy", "upstreams": [{ "dial": upstream }] })
}

/// Приложение с `open: iframe` фреймит Desktop: снимаем запрет фреймить только на origin Desktop'а (3.12).
fn frame_ancestors_rewrite(origins: &str) -> Value {
    json!({ "handler": "headers", "response": { "deferred": true, "delete": ["X-Frame-Options"],
        "set": { "Content-Security-Policy": [format!("frame-ancestors 'self' {origins}")] } } })
}

/// Вместо голого 502/503/504 от reverse_proxy — страница AgentVerse OS: приложение ещё запускается или упало, ссылка на карточку и логи,
/// автообновление раз в 5 секунд. Пользователь работает только из браузера, и это единственное, что он увидит.
fn error_page(host: &str, name: &str, title: &str) -> Value {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let body = format!(
        "<!doctype html><html lang=\"ru\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"refresh\" content=\"5\"><title>{t} не отвечает</title>\
<style>body{{margin:0;min-height:100vh;display:grid;place-items:center;font:15px/1.5 system-ui,sans-serif;background:#121416;color:#F2F4F4}}.c{{max-width:520px;padding:28px 32px;border:1px solid #2A3438;border-radius:14px;background:#171c1f}}h1{{font-size:1.15rem;margin:0 0 8px}}p{{margin:6px 0;color:#9AA3A8}}a{{color:#5BBFCB}}.s{{display:inline-block;width:12px;height:12px;border:2px solid #5BBFCB;border-right-color:transparent;border-radius:50%;animation:r .8s linear infinite;vertical-align:-2px;margin-right:8px}}@keyframes r{{to{{transform:rotate(360deg)}}}}</style></head>\
<body><div class=\"c\"><h1><span class=\"s\"></span>{t} пока не отвечает</h1><p>Приложение ещё запускается или его контейнер упал (код {{http.error.status_code}}). Страница обновится сама через 5 секунд.</p>\
<p>Если не поднимается: откройте <a href=\"https://{h}/#app={n}\">карточку приложения</a> в AgentVerse OS, там состояние стека и кнопка «Логи». Обычные причины: приложение ждёт обязательную настройку, образ ещё скачивается, не хватило прав на каталог данных.</p></div></body></html>",
        t = esc(title), h = esc(host), n = esc(name)
    );
    json!({ "routes": [ { "handle": [ { "handler": "headers", "response": { "set": { "Cache-Control": ["no-store"], "Content-Type": ["text/html; charset=utf-8"] } } },
        { "handler": "static_response", "status_code": "{http.error.status_code}", "body": body } ] } ] })
}
