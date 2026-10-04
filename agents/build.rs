// Веб-приложение «Агенты» (agents/ui → npm run build → ui/dist) встраивается rust-embed при компиляции. Если статика не собрана,
// вместо неё встраивается заглушка — сервис и API всё равно работают, а страница честно говорит, что интерфейс не собран.
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
