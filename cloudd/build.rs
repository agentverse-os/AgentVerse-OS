// Статика Desktop встраивается rust-embed при компиляции: без этого cargo не замечает новую сборку в static/.
fn main() {
    println!("cargo:rerun-if-changed=static");
    for e in walk("static") {
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
