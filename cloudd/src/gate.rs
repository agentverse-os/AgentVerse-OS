//! Gate проекта (2.5, ADR-002/005): Caddy-контейнер на macvlan в `net-<project>`; отдаёт ровно выданные маршруты
//! `http://<capability>.gate` → `<service>:<port>` в сети provider'а, а для внешних провайдеров (`type: external`) —
//! на их https-адрес с подстановкой ключа; всё остальное — 403 «no grant».

use crate::model::{AppState, Grant};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub enum Upstream {
    /// `<service>:<port>` в сети приложения.
    Internal(String),
    /// Внешний адрес `https://host[:port]`; ключ подставляется в заголовки, Host — хост upstream.
    External { url: String, key: Option<String> },
}

pub struct GateRoute<'a> {
    pub capability: &'a str,
    pub upstream: Upstream,
}

fn caddy_quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn render_caddyfile(project: &str, routes: &[GateRoute<'_>]) -> String {
    let mut out = format!(
        "# gate проекта {project} — рендерит cloudd из grants; правки руками перезапишутся\n{{\n  auto_https off\n  admin localhost:2019\n}}\n\n"
    );
    for r in routes {
        match &r.upstream {
            Upstream::Internal(up) => out.push_str(&format!("http://{cap}.gate, http://{cap}.gate:80 {{\n  reverse_proxy {up}\n}}\n\n", cap = r.capability)),
            Upstream::External { url, key } => {
                out.push_str(&format!("http://{cap}.gate, http://{cap}.gate:80 {{\n  reverse_proxy {url} {{\n    header_up Host {{upstream_hostport}}\n", cap = r.capability));
                if let Some(k) = key {
                    out.push_str(&format!(
                        "    header_up Authorization {bearer}\n    header_up x-api-key {k}\n    header_up x-goog-api-key {k}\n",
                        bearer = caddy_quote(&format!("Bearer {k}")),
                        k = caddy_quote(k)
                    ));
                }
                out.push_str("  }\n}\n\n");
            }
        }
    }
    out.push_str("http:// {\n  respond \"no grant\" 403\n}\n");
    out
}

/// Маршруты из grants: capability → приложение, которое её provides → endpoint в `<app>-net`, либо внешний адрес
/// (`externals`: имя приложения → (url, ключ) для `type: external`).
pub fn routes_for<'a>(grants: &'a [Grant], apps: &'a [AppState], externals: &HashMap<String, (String, Option<String>)>) -> Vec<GateRoute<'a>> {
    grants
        .iter()
        .filter_map(|g| {
            let app = apps.iter().find(|a| a.name == g.app)?;
            let upstream = match externals.get(&g.app) {
                Some((url, key)) => Upstream::External { url: url.clone(), key: key.clone() },
                None => Upstream::Internal(format!("{}:{}", app.manifest.endpoint.service, app.manifest.endpoint.port)),
            };
            Some(GateRoute { capability: &g.capability, upstream })
        })
        .collect()
}

pub fn write_caddyfile(dir: &Path, content: &str) -> Result<bool> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("Caddyfile");
    let changed = std::fs::read_to_string(&path).map(|old| old != content).unwrap_or(true);
    if changed {
        std::fs::write(&path, content)?;
    }
    Ok(changed)
}
