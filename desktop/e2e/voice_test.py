# e2e Pipecat Voice: встроенный клиент Pipecat открывается через edge, WebRTC-соединение с ботом устанавливается (фейковый микрофон Chromium),
# и то же самое внутри окна Desktop (iframe с allow=microphone).
import os, re, time, json, urllib.request, ssl
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
app = json.load(urllib.request.urlopen(f"https://{HOST}/api/apps/pipecat-voice", context=ctx_ssl)); URL = app["url"]
ok("api: pipecat-voice installed & running", app["installed"] and app["state"] == "running", str(app.get("state")))
def wait_connected(scope, label):
    # журнал событий клиента Pipecat: Transport state changed: … → ready означает установленный WebRTC и открытый канал данных
    t0 = time.time(); state = ""
    for _ in range(60):
        time.sleep(1); m = re.findall(r"Transport state changed: (\w+)", scope.locator("body").inner_text())
        if m and m[-1] in ("ready", "connected"): state = m[-1]; break
        if m and m[-1] == "error": state = "error"; break
    ok(f"{label}: webrtc transport ready", state in ("ready", "connected"), f"{time.time() - t0:.0f}s state={state or 'timeout'}")
with sync_playwright() as p:
    b = p.chromium.launch(args=["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream", "--autoplay-policy=no-user-gesture-required"])
    ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}, permissions=["microphone"]); pg = ctx.new_page()
    pg.goto(URL + "client/", wait_until="domcontentloaded"); time.sleep(2)
    btn = pg.get_by_role("button", name=re.compile("^connect", re.I)).first
    ok("direct: prebuilt client loads via edge", btn.count() >= 1, pg.locator("body").inner_text()[:80].replace("\n", " | "))
    btn.click(); wait_connected(pg, "direct"); pg.screenshot(path="shots/vc-direct.png"); pg.close()
    pg = ctx.new_page(); pg.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999'); localStorage.setItem('cloudos.access.shown.pipecat-voice', '1')")
    pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(1.5)
    pg.fill("input.search", "pipecat"); time.sleep(0.8)
    pg.locator(".card.app", has_text="Pipecat Voice").first.locator("button", has_text="Открыть").click(); time.sleep(3)
    fr = pg.frame_locator(".win iframe[src*='" + URL.split("//")[1].split("/")[0] + "']")
    try:
        fbtn = fr.get_by_role("button", name=re.compile("^connect", re.I)).first; fbtn.wait_for(timeout=30000); ok("desktop window: client rendered in iframe", True)
        fbtn.click(); wait_connected(fr, "desktop window")
    except Exception as e: ok("desktop window: client rendered in iframe", False, str(e)[:120])
    pg.screenshot(path="shots/vc-desktop.png")
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
