#!/usr/bin/env python3
"""Демо-прокси для скриншотов README: сидит перед настоящим cloudd и подменяет ответы про состояние системы
(проекты, установленные приложения, монитор, события, статус, погода, лента новостей) на показательные данные.
Всё остальное — каталог Store, иконки, статика Desktop, настройки — уходит в настоящее ядро.
Запуск: python3 demo_proxy.py <порт> <адрес cloudd> [--tls cert.pem key.pem]   (например 7300 http://127.0.0.1:7299)
--tls — отдавать HTTPS с указанным (самоподписанным) сертификатом: так e2e-тесты Desktop (они ходят по https и игнорируют ошибки
сертификата) можно прогнать локально на демо-данных, без стенда: EDGE_HOST=host.docker.internal:7443."""
import json, ssl, sys, time, urllib.request, urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 7300
UPSTREAM = sys.argv[2] if len(sys.argv) > 2 else "http://127.0.0.1:7299"
NOW = "2026-09-11T09:41:00Z"

INSTALLED = {  # имя → (порт, состояние)
    "gitea": (8456, "running"), "n8n-coolify": (8460, "running"), "hermes-agent": (8462, "running"),
    "uptime-kuma": (8453, "running"), "ntfy": (8455, "running"), "it-tools": (8454, "running"),
    "linkding-coolify": (8461, "running"), "nextcloud": (8463, "running"), "jellyfin": (8464, "running"),
    "immich": (8465, "running"), "vaultwarden": (8466, "running"), "paperless-ngx": (8467, "running"),
    "litellm-coolify": (8458, "running"), "garage": (8452, "running"), "dozzle": (8450, "running"),
}
HOST = "agentverse.tail0fe52f.ts.net"

def ws(name, running=True, ip="10.77.1.10"):
    return {"id": "00000000-0000-0000-0000-00000000000%d" % (1 if name == "alpha" else 2), "name": f"{name}.main",
            "status": "running" if running else "stopped", "agent_status": "connected" if running else None,
            "instance": f"ws-{name}", "ip": ip if running else None,
            "apps": [{"slug": "code-server", "display_name": "VS Code", "url": f"https://{HOST}:8444/@admin/{name}.main/apps/code-server/", "health": "healthy"}] if running else [],
            "url": f"https://{HOST}:8444/@admin/{name}.main"}

PROJECTS = [
    {"name": "alpha", "spec": {"schema": 1, "project": "alpha", "workspace": {"runtime": "incus-nesting", "cpu": 4, "memory": 8, "home": 50}, "agents": ["claude-code", "codex"], "capabilities": ["llm", "storage.s3"]},
     "network": "net-alpha", "subnet": "10.77.1.0/24", "gate": {"name": "gate-alpha", "status": "ok", "detail": "caddy"}, "workspace": ws("alpha"),
     "grants": [{"project": "alpha", "capability": "llm", "app": "litellm-coolify", "granted_at": NOW}, {"project": "alpha", "capability": "storage.s3", "app": "garage", "granted_at": NOW}],
     "missing_capabilities": [], "grant_env": {"llm": ["LLM_URL", "LLM_API_KEY"], "storage.s3": ["S3_ENDPOINT", "S3_BUCKET", "S3_ACCESS_KEY", "S3_SECRET_KEY"]}, "available_capabilities": ["llm", "storage.s3", "notify"]},
    {"name": "beta", "spec": {"schema": 1, "project": "beta", "workspace": {"runtime": "incus-nesting", "cpu": 2, "memory": 4, "home": 50}, "agents": ["claude-code"], "capabilities": ["notify"]},
     "network": "net-beta", "subnet": "10.77.2.0/24", "gate": {"name": "gate-beta", "status": "ok", "detail": "caddy"}, "workspace": ws("beta", running=False),
     "grants": [{"project": "beta", "capability": "notify", "app": "ntfy", "granted_at": NOW}], "missing_capabilities": [], "grant_env": {"notify": ["NTFY_URL", "NTFY_TOPIC", "NTFY_TOKEN"]}, "available_capabilities": ["llm", "storage.s3", "notify"]},
    {"name": "research", "spec": {"schema": 1, "project": "research", "workspace": {"runtime": "incus-nesting", "cpu": 2, "memory": 4, "home": 30}, "agents": ["claude-code", "codex"], "capabilities": ["llm"]},
     "network": "net-research", "subnet": "10.77.3.0/24", "gate": {"name": "gate-research", "status": "ok", "detail": "caddy"}, "workspace": ws("research", ip="10.77.3.10"),
     "grants": [{"project": "research", "capability": "llm", "app": "litellm-coolify", "granted_at": NOW}], "missing_capabilities": [], "grant_env": {"llm": ["LLM_URL", "LLM_API_KEY"]}, "available_capabilities": ["llm", "storage.s3", "notify"]},
]

EVENTS_RU = [
    ("update", "gitea", "обновлено: 1.27.2 → 1.27.3"), ("app", "nextcloud", "маршрут https://%s:8463" % HOST),
    ("project", "alpha", "grant storage.s3 → garage: gate подключён к сети garage-net"), ("backup", "daily", "снимки tank/apps, tank/core, tank/postgres; restic 1.2 ГиБ"),
    ("app", "immich", "health OK"), ("project", "research", "workspace research.main запущен, агент connected"),
    ("app", "hermes-agent", "связь с litellm: OPENAI_BASE_URL=http://litellm:4000/v1"), ("update", "system", "обновление до 0.2.0 применено"),
    ("app", "paperless-ngx", "каталог данных paperless-ngx/data → 1000:1000"), ("project", "beta", "workspace beta.main остановлен"),
    ("setup", "wizard", "первый запуск завершён"), ("app", "vaultwarden", "стек running: 1 сервисов"),
]
EVENTS_EN = [
    ("update", "gitea", "updated: 1.27.2 → 1.27.3"), ("app", "nextcloud", "route https://%s:8463" % HOST),
    ("project", "alpha", "grant storage.s3 → garage: gate joined network garage-net"), ("backup", "daily", "snapshots tank/apps, tank/core, tank/postgres; restic 1.2 GiB"),
    ("app", "immich", "health OK"), ("project", "research", "workspace research.main started, agent connected"),
    ("app", "hermes-agent", "linked to litellm: OPENAI_BASE_URL=http://litellm:4000/v1"), ("update", "system", "update to 0.2.0 applied"),
    ("app", "paperless-ngx", "data directory paperless-ngx/data → 1000:1000"), ("project", "beta", "workspace beta.main stopped"),
    ("setup", "wizard", "first run completed"), ("app", "vaultwarden", "stack running: 1 service"),
]

def monitor():
    import math
    hist_cpu = [round(9 + 6 * math.sin(i / 7) + (i % 5) * 0.8, 1) for i in range(60)]
    hist_mem = [int(9.1e9 + 2.5e8 * math.sin(i / 9)) for i in range(60)]
    return {"host": {"at": NOW, "cpu_pct": 11.0, "mem_used": int(9.3e9), "mem_total": int(32e9), "load1": 0.42, "load5": 0.51, "load15": 0.47, "uptime_s": 19 * 86400 + 5 * 3600},
            "disks": [{"mount": "/", "total": int(120e9), "avail": int(71e9), "fs": "ext4"}, {"mount": "/srv/apps", "total": int(1.8e12), "avail": int(1.31e12), "fs": "zfs"}, {"mount": "/srv/backups", "total": int(1.8e12), "avail": int(1.31e12), "fs": "zfs"}],
            "containers": [{"name": "nextcloud-app-1", "app": "nextcloud", "cpu_pct": 1.4, "mem_used": int(1.1e9), "mem_limit": 0, "state": "running"}, {"name": "immich-server-1", "app": "immich", "cpu_pct": 2.1, "mem_used": int(0.9e9), "mem_limit": 0, "state": "running"}, {"name": "jellyfin-1", "app": "jellyfin", "cpu_pct": 0.6, "mem_used": int(0.5e9), "mem_limit": 0, "state": "running"}, {"name": "gitea-server-1", "app": "gitea", "cpu_pct": 0.3, "mem_used": int(0.4e9), "mem_limit": 0, "state": "running"}, {"name": "n8n-1", "app": "n8n-coolify", "cpu_pct": 0.5, "mem_used": int(0.35e9), "mem_limit": 0, "state": "running"}],
            "instances": [{"name": "ws-alpha", "cpu_pct": 3.2, "mem_used": int(2.4e9)}, {"name": "ws-research", "cpu_pct": 1.1, "mem_used": int(1.3e9)}],
            "history_cpu": hist_cpu, "history_mem": hist_mem, "cores": 12}

STATUS_COMPONENTS = [{"name": n, "status": "ok", "detail": d} for n, d in [("docker", "28.3.2"), ("incus", "6.14"), ("coder", "ok"), ("komodo", "2.3.3"), ("caddy", "admin api")]]

NEWS_RU = {"url": "demo", "title": "AI", "fetched_at": NOW, "error": None, "items": [
    {"title": "Anthropic представила Claude Fable 5.1: рассуждение с инструментами в одном проходе", "link": "https://example.com/1", "published": "2026-09-11T08:10:00Z", "source": "TechCrunch", "summary": "Модель Mythos-класса стала доступна всем разработчикам через API и Claude Code.", "image": None, "content": None},
    {"title": "Локальные агенты вместо облачных VM: почему закрылись Operator и Mariner", "link": "https://example.com/2", "published": "2026-09-11T07:30:00Z", "source": "The Verge", "summary": "Индустрия переходит к агентам в браузере пользователя и структурированным входам вроде MCP.", "image": None, "content": None},
    {"title": "Voxtral Realtime: потоковое распознавание речи на 13 языках без GPU", "link": "https://example.com/3", "published": "2026-09-10T21:15:00Z", "source": "Хабр", "summary": "Mistral выложила открытую модель с задержкой ниже 200 мс, русский в списке.", "image": None, "content": None},
    {"title": "WebMCP выходит в origin trial: сайты начинают публиковать инструменты для агентов", "link": "https://example.com/4", "published": "2026-09-10T16:40:00Z", "source": "Ars Technica", "summary": "Chrome 149 получил navigator.modelContext, Expedia и Shopify тестируют первые интеграции.", "image": None, "content": None},
    {"title": "Home Assistant Assist: голосовое управление домом без облака стало нормой", "link": "https://example.com/5", "published": "2026-09-10T12:00:00Z", "source": "Хабр", "summary": "Whisper, Piper и локальная LLM в каждой десятой установке.", "image": None, "content": None},
]}
NEWS_EN = {"url": "demo", "title": "AI", "fetched_at": NOW, "error": None, "items": [
    {"title": "Anthropic ships Claude Fable 5.1: tool-using reasoning in a single pass", "link": "https://example.com/1", "published": "2026-09-11T08:10:00Z", "source": "TechCrunch", "summary": "The Mythos-class model is now available to every developer through the API and Claude Code.", "image": None, "content": None},
    {"title": "Local agents instead of cloud VMs: why Operator and Mariner shut down", "link": "https://example.com/2", "published": "2026-09-11T07:30:00Z", "source": "The Verge", "summary": "The industry is moving to agents inside the user's browser and structured inputs like MCP.", "image": None, "content": None},
    {"title": "Voxtral Realtime: streaming speech recognition in 13 languages without a GPU", "link": "https://example.com/3", "published": "2026-09-10T21:15:00Z", "source": "Ars Technica", "summary": "Mistral released an open model with sub-200 ms latency.", "image": None, "content": None},
    {"title": "WebMCP enters origin trial: sites start publishing tools for agents", "link": "https://example.com/4", "published": "2026-09-10T16:40:00Z", "source": "Ars Technica", "summary": "Chrome 149 gets navigator.modelContext; Expedia and Shopify test the first integrations.", "image": None, "content": None},
    {"title": "Home Assistant Assist: voice control of the home without the cloud becomes the norm", "link": "https://example.com/5", "published": "2026-09-10T12:00:00Z", "source": "Hacker News", "summary": "Whisper, Piper and a local LLM in one install out of ten.", "image": None, "content": None},
]}
def by_lang(h):
    """События и новости — на языке браузера (Accept-Language): русский как прежде, иначе английский."""
    ru = (h.headers.get("Accept-Language") or "").lower().startswith("ru")
    return (EVENTS_RU, NEWS_RU) if ru else (EVENTS_EN, NEWS_EN)

WEATHER = {"current": {"temperature_2m": 21.4, "weather_code": 2, "wind_speed_10m": 3.1, "relative_humidity_2m": 54, "apparent_temperature": 21.0},
           "daily": {"time": ["2026-09-11", "2026-09-12", "2026-09-13", "2026-09-14", "2026-09-15"], "weather_code": [2, 3, 61, 1, 0], "temperature_2m_max": [24.0, 22.5, 19.8, 23.1, 25.6], "temperature_2m_min": [14.2, 13.9, 12.5, 12.8, 14.0]}}

def fetch(path, method="GET", body=None, headers=None):
    req = urllib.request.Request(UPSTREAM + path, data=body, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send_json(self, obj, code=200):
        data = json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
    def do_GET(self):
        u = urlparse(self.path); p = u.path
        if p == "/api/status":
            self.send_json({"edge_host": HOST, "edge_alt_hosts": ["100.69.48.34"], "coder_url": f"https://{HOST}:8444", "coder_user": "admin@agentverse.local", "coder_password_set": True,
                            "components": STATUS_COMPONENTS, "version": "0.2.0", "build": "20260911-demo", "setup_done": True, "update_available": False, "app_updates": 2}); return
        if p == "/api/projects": self.send_json(PROJECTS); return
        if p.startswith("/api/projects/") and p.count("/") == 3:
            name = p.rsplit("/", 1)[1]; pr = next((x for x in PROJECTS if x["name"] == name), None)
            if pr: self.send_json(pr); return
        if p == "/api/monitor": self.send_json(monitor()); return
        if p == "/api/events":
            t = time.mktime(time.strptime(NOW, "%Y-%m-%dT%H:%M:%SZ"))
            self.send_json([{"at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t - i * 1370)), "kind": k, "subject": s, "message": m} for i, (k, s, m) in enumerate(by_lang(self)[0])]); return
        if p == "/api/widgets/feed": self.send_json(by_lang(self)[1]); return
        if p == "/api/widgets/weather": self.send_json(WEATHER); return
        if p == "/api/apps" or (p.startswith("/api/apps/") and p.count("/") == 3):
            code, hdr, body = fetch(p)
            if code == 200:
                data = json.loads(body); many = isinstance(data, list)
                for a in (data if many else [data]):
                    inst = INSTALLED.get(a["name"])
                    a["installed"] = bool(inst)
                    if inst:
                        port, state = inst
                        a["port"] = port; a["url"] = f"https://{HOST}:{port}/"; a["state"] = state; a["internal_url"] = f"http://{a['manifest']['endpoint']['service']}:{a['manifest']['endpoint']['port']}"
                        if a["name"] == "hermes-agent": a["links_to"] = ["litellm-coolify"]
                        if a["name"] == "litellm-coolify": a["links_from"] = ["hermes-agent", "n8n-coolify"]; a["granted_to"] = ["alpha", "research"]
                        if a["name"] == "garage": a["granted_to"] = ["alpha"]
                        if a["name"] == "ntfy": a["granted_to"] = ["beta"]
                        if a["name"] == "n8n-coolify": a["links_to"] = ["litellm-coolify"]
                    else:
                        a["port"] = None; a["url"] = None; a["state"] = None
                self.send_json(data); return
        code, hdr, body = fetch(p)
        self.send_response(code)
        for k, v in hdr.items():
            if k.lower() in ("content-type", "cache-control", "etag", "content-length"): self.send_header(k, v)
        self.end_headers(); self.wfile.write(body)
    def do_PUT(self): self._pass()
    def do_POST(self): self._pass()
    def do_DELETE(self): self._pass()
    def _pass(self):
        n = int(self.headers.get("Content-Length") or 0); body = self.rfile.read(n) if n else None
        code, hdr, out = fetch(self.path, self.command, body, {"Content-Type": self.headers.get("Content-Type", "application/json")})
        self.send_response(code)
        for k, v in hdr.items():
            if k.lower() in ("content-type", "content-length"): self.send_header(k, v)
        self.end_headers(); self.wfile.write(out)

if __name__ == "__main__":
    srv = ThreadingHTTPServer(("0.0.0.0", PORT), H)
    if "--tls" in sys.argv:
        i = sys.argv.index("--tls"); ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(sys.argv[i + 1], sys.argv[i + 2])
        srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    print(f"demo proxy :{PORT} → {UPSTREAM}" + (" (https)" if "--tls" in sys.argv else ""), flush=True)
    srv.serve_forever()
