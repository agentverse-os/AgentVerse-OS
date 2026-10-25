//! Edge Caddy: `cloudd` owns the whole configuration and loads it through the Admin API (`POST /load`, idempotent; 3.1). One
//! hostname, the `path` and `port` modes (2.2). TLS — Caddy's internal CA (NetBird) or a certificate from tailscaled
//! (Tailscale).

use crate::model::{AppState, RouteMode};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct EdgeCaddy {
    admin: String,
    http: reqwest::Client,
}

/// Lifetime of the internal CA's leaf certificates. By default Caddy issues them for 12 hours and reissues every ~8: a browser
/// without the root installed (the "accept the risk" exception, Firefox) loses the exception on every reissue — the shell opens from
/// the cache while API requests are rejected. 90 days and signing by the root (without the intermediate that lives 7 days) make the
/// exception stable.
pub const LEAF_LIFETIME: &str = "2160h";

/// Preview for the render: project name, dev server port, edge port, upstream `ip:port` (None — the workspace is not running: only the
/// error page)
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewRoute { pub project: String, pub port: u16, pub edge_port: u16, pub upstream: Option<String> }

pub struct EdgeInput<'a> {
    pub host: &'a str,
    /// Additional names/IPs of the same Entry Point: the same routes and certificate (the internal CA can
    /// do IP SANs).
    pub alt_hosts: &'a [String],
    pub cloudd_upstream: &'a str,
    /// The "Agents" service on the host: `/agents/*` goes to it,
    /// bypassing the core.
    pub agents_upstream: &'a str,
    pub coder_upstream: &'a str,
    pub coder_port: u16,
    /// "Agents" as a separate app (§9.8): its own server on this port, the same service, `/` → `/agents/`
    pub agents_port: u16,
    pub apps: &'a [AppState],
    /// Dev server previews (§4.9): a server on the edge port → the workspace IP and the dev
    /// server port
    pub previews: &'a [PreviewRoute],
    /// `internal` — the internal CA; `auto` — regular ACME/tailscale (no explicit
    /// issuer).
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

    /// Does the live edge configuration have our server (after the container is recreated Caddy starts with the bootstrap config
    /// without it)
    pub async fn has_server(&self, name: &str) -> Result<bool> {
        let r = self.http.get(format!("{}/config/apps/http/servers/{name}", self.admin)).send().await.context("caddy admin")?;
        if r.status().is_success() { let body = r.text().await.unwrap_or_default(); return Ok(body.trim() != "null" && !body.trim().is_empty()) }
        if r.status().as_u16() == 404 { return Ok(false) }
        Err(anyhow!("caddy admin: HTTP {}", r.status()))
    }

    pub async fn load(&self, config: &Value) -> Result<()> {
        let r = self.http.post(format!("{}/load", self.admin)).json(config).send().await.context("caddy /load")?;
        if !r.status().is_success() {
            return Err(anyhow!("caddy /load: HTTP {}: {}", r.status(), r.text().await.unwrap_or_default()));
        }
        Ok(())
    }

    /// Full JSON configuration of the edge.
    pub fn render(input: &EdgeInput<'_>) -> Value {
        let host = input.host;
        let mut hosts: Vec<String> = vec![host.to_string()];
        hosts.extend(input.alt_hosts.iter().filter(|h| !h.is_empty() && h.as_str() != host).cloned());
        // origins allowed to frame previews, apps and Coder: Desktop (:443) and "Agents" on its own port (§9.8)
        let origins = hosts.iter().flat_map(|h| [format!("https://{h}"), format!("https://{h}:{}", input.agents_port)]).collect::<Vec<_>>().join(" ");
        let mut servers = serde_json::Map::new();

        // ---- Desktop + cloudd API (:443), plus path apps under /apps/<name>/ ----
        let mut desktop_routes = Vec::new();
        // the "Agents" app is a separate service on the same host; the prefix is not stripped, the service itself
        // lives under /agents/
        desktop_routes.push(json!({
            "match": [{ "path": ["/agents", "/agents/*"] }],
            "handle": [ { "handler": "encode", "encodings": { "gzip": {}, "zstd": {} }, "prefer": ["zstd", "gzip"], "minimum_length": 1024 }, reverse_proxy(input.agents_upstream) ]
        }));
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
                // the Store catalog is 2 MB of JSON; over WireGuard with MTU 1200 without
                // compression that is seconds
                { "handler": "encode", "encodings": { "gzip": {}, "zstd": {} }, "prefer": ["zstd", "gzip"], "minimum_length": 1024 },
                reverse_proxy(input.cloudd_upstream)
            ],
            "terminal": true
        }));
        servers.insert("desktop".into(), json!({ "listen": [":443"], "routes": desktop_routes }));

        // ---- Coder: route: port. Coder sets `frame-ancestors 'self'` (origin :8444) — extend it to the Desktop origin so
        // VS Code Web and the terminal open in Desktop windows (3.12: a targeted rewrite only to the Desktop origin).
        servers.insert(
            "coder".into(),
            json!({ "listen": [format!(":{}", input.coder_port)], "routes": [ { "match": [{ "host": hosts.clone() }], "handle": [
                { "handler": "headers", "response": { "deferred": true, "delete": ["X-Frame-Options"],
                    "replace": { "Content-Security-Policy": [ { "search_regexp": "frame-ancestors 'self'", "replace": format!("frame-ancestors 'self' {origins}") } ] } } },
                reverse_proxy(input.coder_upstream)
            ], "terminal": true } ] }),
        );

        // ---- "Agents" as a separate app (§9.8): its own origin on a port so Chrome installs it separately from Desktop ----
        servers.insert(
            "agents".into(),
            json!({ "listen": [format!(":{}", input.agents_port)], "routes": [
                { "match": [{ "host": hosts.clone(), "path": ["/"] }], "handle": [ { "handler": "static_response", "status_code": 302, "headers": { "Location": ["/agents/"] } } ], "terminal": true },
                { "match": [{ "host": hosts.clone() }], "handle": [
                    { "handler": "encode", "encodings": { "gzip": {}, "zstd": {} }, "prefer": ["zstd", "gzip"], "minimum_length": 1024 },
                    reverse_proxy(input.agents_upstream)
                ], "terminal": true }
            ], "errors": error_page(host, "agents", "Агенты") }),
        );

        // ---- route: port apps — a separate origin on a port ----
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

        // ---- dev server previews (§4.9): like route: port apps, the upstream is the workspace IP
        // ----
        for pv in input.previews {
            let name = format!("preview-{}-{}", pv.project, pv.port);
            let title = format!("превью {} :{}", pv.project, pv.port);
            let mut handle = vec![frame_ancestors_rewrite(&origins)];
            handle.push(reverse_proxy(pv.upstream.as_deref().unwrap_or("127.0.0.1:9")));
            servers.insert(name.clone(), json!({ "listen": [format!(":{}", pv.edge_port)], "routes": [ { "match": [{ "host": hosts.clone() }], "handle": handle, "terminal": true } ],
                "errors": error_page(host, &name, &title) }));
        }

        // A client without SNI (a browser by IP address) gets the IP certificate: default_sni on all servers. Otherwise Caddy answers with alert
        // internal error.
        if let Some(ip) = hosts.iter().find(|h| h.parse::<std::net::IpAddr>().is_ok()) {
            for (_, srv) in servers.iter_mut() {
                srv["tls_connection_policies"] = json!([{ "default_sni": ip }]);
            }
        }
        // h1/h2 only: the edge container publishes only TCP, and an Alt-Svc h3 header makes browsers (especially Firefox) try QUIC and wait
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
            // Tailscale: for the machine name (<box>.<tailnet>.ts.net) Caddy itself takes a Let's Encrypt certificate from tailscaled
            // via /var/run/tailscale/tailscaled.sock — no policy is set for it. Fallback names and IPs get no public certificate: the
            // internal CA remains for them (with a browser exception).
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

/// An app with `open: iframe` is framed by Desktop: lift the framing ban only for the Desktop origin
/// (3.12).
fn frame_ancestors_rewrite(origins: &str) -> Value {
    json!({ "handler": "headers", "response": { "deferred": true, "delete": ["X-Frame-Options"],
        "set": { "Content-Security-Policy": [format!("frame-ancestors 'self' {origins}")] } } })
}

/// Instead of a bare 502/503/504 from reverse_proxy — an AgentVerse OS page: the app is still starting or has crashed, a link to the
/// card and the logs, auto-refresh every 5 seconds. The user works only from the browser, and this is all they will see.
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
