//! Gate проекта (2.5, ADR-002/005): Caddy-контейнер на macvlan в `net-<project>`; отдаёт ровно выданные маршруты
//! `http://<capability>.gate` → `<service>:<port>` в сети provider'а; всё остальное — 403 «no grant».

use crate::model::{AppState, Grant};
use anyhow::Result;
use std::path::Path;

pub struct GateRoute<'a> {
    pub capability: &'a str,
    pub upstream: String,
}

pub fn render_caddyfile(project: &str, routes: &[GateRoute<'_>]) -> String {
    let mut out = format!(
        "# gate проекта {project} — рендерит cloudd из grants; правки руками перезапишутся\n{{\n  auto_https off\n  admin localhost:2019\n}}\n\n"
    );
    for r in routes {
        out.push_str(&format!("http://{cap}.gate, http://{cap}.gate:80 {{\n  reverse_proxy {up}\n}}\n\n", cap = r.capability, up = r.upstream));
    }
    out.push_str("http:// {\n  respond \"no grant\" 403\n}\n");
    out
}

/// Маршруты из grants: capability → приложение, которое её provides → endpoint в `<app>-net`.
pub fn routes_for<'a>(grants: &'a [Grant], apps: &'a [AppState]) -> Vec<GateRoute<'a>> {
    grants
        .iter()
        .filter_map(|g| {
            let app = apps.iter().find(|a| a.name == g.app)?;
            Some(GateRoute { capability: &g.capability, upstream: format!("{}:{}", app.manifest.endpoint.service, app.manifest.endpoint.port) })
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
