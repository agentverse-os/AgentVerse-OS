// Статика Desktop встраивается rust-embed при компиляции: без этого cargo не замечает новую сборку в static/.
// Бинарь cloudos-agentd (крейт ../agentd, ADR-006) встраивается тоже: cloudd сам кладёт его в workspace'ы, поэтому один
// артефакт релиза = ядро + адаптер. Если agentd не собран, встраивается пустой файл, и выкладка молча пропускается.
fn main() {
    println!("cargo:rerun-if-changed=static");
    for e in walk("static") {
        println!("cargo:rerun-if-changed={e}");
    }
    embed("../agentd/target/release/cloudos-agentd", "cloudos-agentd", "agentd не собран: cloudd не сможет выкладывать cloudos-agentd в workspace");
    // сервис «Агенты» (крейт ../agents): ядро ставит его на хост как systemd-юнит — один пакет обновления на всё
    embed("../agents/target/release/cloudos-agents", "cloudos-agents", "сервис «Агенты» не собран: cloudd не поставит cloudos-agents на хост");
}
fn embed(src: &str, name: &str, warn: &str) {
    let src = std::path::Path::new(src);
    let dst = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(name);
    println!("cargo:rerun-if-changed={}", src.display());
    match std::fs::read(src) {
        Ok(bytes) => std::fs::write(&dst, bytes).unwrap(),
        Err(_) => {
            println!("cargo:warning={warn} ({})", src.display());
            std::fs::write(&dst, []).unwrap();
        }
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
