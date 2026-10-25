// The "Agents" web app (agents/ui → npm run build → ui/dist) is embedded with rust-embed at compile time. If the statics are not
// built, a stub is embedded instead — the service and the API still work, and the page honestly says the UI is not built.
fn main() {
    let dist = std::path::Path::new("ui/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).unwrap();
        std::fs::write(dist.join("index.html"), "<!doctype html><meta charset=\"utf-8\"><title>Агенты</title><p style=\"font:14px system-ui;padding:24px\">Интерфейс не собран: <code>cd agents/ui && npm run build</code></p>").unwrap();
        println!("cargo:warning=agents/ui/dist не собран — встроена заглушка");
    }
    println!("cargo:rerun-if-changed=ui/dist");
    for e in walk("ui/dist") {
        println!("cargo:rerun-if-changed={e}");
    }
}
fn walk(dir: &str) -> Vec<String> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { out.extend(walk(p.to_str().unwrap())); } else { out.push(p.to_string_lossy().to_string()); }
        }
    }
    out
}
