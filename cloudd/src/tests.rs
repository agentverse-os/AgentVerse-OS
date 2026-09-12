//! Модульные тесты ядра: контракты (парсинг), рендеры (gate, edge), хранилище (SQLite + age).
//! Интеграция с живыми Incus/Docker/Coder/Komodo — `tests/e2e-stand.sh` на стенде.

use crate::caddy::{EdgeCaddy, EdgeInput};
use crate::gate;
use crate::model::*;
use crate::store::{self, Store};

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn manifest(name: &str, provides: &[&str], mode: RouteMode, port: Option<u16>) -> AppManifest {
    AppManifest {
        login: LoginSpec::default(),
        recommended: None,
        schema: 1,
        name: name.into(),
        title: None,
        description: None,
        i18n: Default::default(),
        icon: None,
        kind: AppKind::App,
        compose: "./compose.yaml".into(),
        provides: provides.iter().map(|s| s.to_string()).collect(),
        requires: vec![],
        route: RouteSpec { mode, prefix: None, port, open: OpenMode::Iframe },
        auth: None,
        env: Default::default(),
        data: vec![],
        health: None,
        endpoint: Endpoint { service: name.into(), port: 80 },
        hooks: Hooks::default(),
        origin: "manual".into(),
        upstream: None,
        settings: vec![],
        host_docker_socket: false,
        notes: vec![],
        gallery: vec![],
        readme: None,
        website: None,
        tags: vec![],
    }
}

fn app_state(m: AppManifest, port: Option<u16>) -> AppState {
    AppState { name: m.name.clone(), manifest: m, port, komodo_stack_id: None, installed_at: store::now() }
}

// ---------- project.yaml ----------

#[test]
fn project_yaml_parses_gib_and_defaults() {
    let y = "schema: 1\nproject: alpha\nworkspace:\n  runtime: incus-nesting\n  cpu: 4\n  memory: 8GiB\n  home: 50GiB\nagents: [claude-code, codex]\ncapabilities: [storage.s3, llm]\n";
    let p: ProjectSpec = serde_yaml_ng::from_str(y).unwrap();
    assert_eq!(p.project, "alpha");
    assert_eq!(p.workspace.runtime, Runtime::IncusNesting);
    assert_eq!(p.workspace.memory, 8);
    assert_eq!(p.workspace.home, 50);
    assert_eq!(p.capabilities, vec!["storage.s3", "llm"]);
    // числа без суффикса тоже принимаются, а обратно сериализуется с GiB
    let p2: ProjectSpec = serde_yaml_ng::from_str("project: b\nworkspace: { memory: 4, home: 10 }\n").unwrap();
    assert_eq!(p2.workspace.memory, 4);
    assert_eq!(p2.workspace.runtime, Runtime::IncusNesting, "runtime по умолчанию — incus-nesting (3.4)");
    let out = serde_yaml_ng::to_string(&p2).unwrap();
    assert!(out.contains("memory: 4GiB"), "{out}");
}

#[test]
fn project_yaml_rejects_bad_runtime() {
    assert!(serde_yaml_ng::from_str::<ProjectSpec>("project: x\nworkspace: { runtime: docker }\n").is_err());
}

#[test]
fn project_state_addressing() {
    let st = ProjectState { name: "alpha".into(), subnet_index: 7, coder_workspace_id: None, spec: serde_yaml_ng::from_str("project: alpha").unwrap(), created_at: store::now() };
    assert_eq!(st.incus_network(), "net-alpha");
    assert_eq!(st.macvlan_network(), "mv-net-alpha");
    assert_eq!(st.gate_name(), "gate-alpha");
    assert_eq!(st.subnet("10.77"), "10.77.7.0/24");
    assert_eq!(st.gateway("10.77"), "10.77.7.1");
    assert_eq!(st.gate_ip("10.77"), "10.77.7.250");
    assert_eq!(st.gate_ip_range("10.77"), "10.77.7.240/28");
}

#[test]
fn validate_names() {
    use crate::service::validate_name;
    assert!(validate_name("alpha").is_ok());
    assert!(validate_name("my-app-2").is_ok());
    assert!(validate_name("Alpha").is_err());
    assert!(validate_name("-x").is_err());
    assert!(validate_name("").is_err());
    assert!(validate_name("a/b").is_err());
    assert!(validate_name(&"a".repeat(49)).is_err());
}

// ---------- манифест ----------

#[test]
fn manifest_parses_store_whoami() {
    let y = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../store/whoami/manifest.yaml")).unwrap();
    let m: AppManifest = serde_yaml_ng::from_str(&y).unwrap();
    assert_eq!(m.name, "whoami");
    assert_eq!(m.provides, vec!["demo.http"]);
    assert_eq!(m.route.mode, RouteMode::Port);
    assert_eq!(m.endpoint.port, 80);
}

#[test]
fn manifest_env_generate_and_literal() {
    let y = "name: garage\ncompose: ./c.yaml\nroute: { mode: path, prefix: /apps/garage/ }\nendpoint: { service: garage, port: 3900 }\nenv:\n  GARAGE_ADMIN_TOKEN: { generate: token }\n  RPC_SECRET: { generate: password }\n  GARAGE_REGION: garage\n";
    let m: AppManifest = serde_yaml_ng::from_str(y).unwrap();
    assert!(matches!(m.env["GARAGE_ADMIN_TOKEN"], EnvSpec::Generate { generate: GenerateKind::Token }));
    assert!(matches!(m.env["GARAGE_REGION"], EnvSpec::Literal(ref s) if s == "garage"));
    assert_eq!(m.kind, AppKind::App);
}

#[test]
fn generate_secrets_have_expected_shape() {
    let t = store::generate(GenerateKind::Token);
    assert_eq!(t.len(), 32);
    assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    let p = store::generate(GenerateKind::Password);
    assert_eq!(p.len(), 24);
    assert_ne!(store::generate(GenerateKind::Token), t);
}

// ---------- gate ----------

#[test]
fn gate_caddyfile_routes_and_deny_all() {
    let apps = vec![app_state(manifest("whoami", &["demo.http"], RouteMode::Port, None), Some(8450)), app_state(manifest("garage", &["storage.s3"], RouteMode::Path, None), None)];
    let grants = vec![
        Grant { project: "alpha".into(), capability: "demo.http".into(), app: "whoami".into(), granted_at: store::now() },
        Grant { project: "alpha".into(), capability: "storage.s3".into(), app: "garage".into(), granted_at: store::now() },
        Grant { project: "alpha".into(), capability: "llm".into(), app: "bifrost".into(), granted_at: store::now() }, // приложение не установлено → маршрута нет
    ];
    let routes = gate::routes_for(&grants, &apps);
    assert_eq!(routes.len(), 2);
    let text = gate::render_caddyfile("alpha", &routes);
    assert!(text.contains("http://demo.http.gate, http://demo.http.gate:80 {\n  reverse_proxy whoami:80"));
    assert!(text.contains("http://storage.s3.gate"));
    assert!(text.contains("reverse_proxy garage:80"));
    assert!(text.contains("admin localhost:2019"), "reload через admin на loopback");
    assert!(text.trim_end().ends_with("respond \"no grant\" 403\n}"), "последний блок — отказ по умолчанию");
    assert!(text.find("no grant").unwrap() > text.find("demo.http.gate").unwrap());
}

#[test]
fn gate_caddyfile_write_reports_change() {
    let d = tmp();
    assert!(gate::write_caddyfile(d.path(), "a").unwrap());
    assert!(!gate::write_caddyfile(d.path(), "a").unwrap(), "без изменений — без reload");
    assert!(gate::write_caddyfile(d.path(), "b").unwrap());
}

// ---------- edge ----------

#[test]
fn edge_config_servers_and_tls() {
    let apps = vec![
        app_state(manifest("whoami", &[], RouteMode::Port, None), Some(8450)),
        app_state(manifest("grafana", &[], RouteMode::Path, None), None),
        app_state(manifest("noport", &[], RouteMode::Port, None), None), // без выданного порта — пропускается
    ];
    let cfg = EdgeCaddy::render(&EdgeInput { host: "box.example", alt_hosts: &[], cloudd_upstream: "host.docker.internal:7100", coder_upstream: "coder:7080", coder_port: 8444, apps: &apps, tls_internal: true });
    let servers = cfg["apps"]["http"]["servers"].as_object().unwrap();
    let mut names: Vec<_> = servers.keys().cloned().collect();
    names.sort();
    assert_eq!(names, vec!["app-whoami", "coder", "desktop"]);
    assert_eq!(servers["coder"]["listen"][0], ":8444");
    assert_eq!(servers["coder"]["routes"][0]["handle"][0]["response"]["replace"]["Content-Security-Policy"][0]["replace"], "frame-ancestors 'self' https://box.example");
    assert_eq!(servers["coder"]["routes"][0]["handle"][1]["upstreams"][0]["dial"], "coder:7080");
    assert_eq!(servers["app-whoami"]["listen"][0], ":8450");
    // path-приложение — маршрут внутри desktop-сервера, раньше catch-all
    let routes = servers["desktop"]["routes"].as_array().unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0]["match"][0]["path"][0], "/apps/grafana/*");
    assert_eq!(routes[1]["handle"][1]["handler"], "encode"); // сжатие API/оболочки перед проксированием
    assert_eq!(routes[1]["handle"][2]["upstreams"][0]["dial"], "host.docker.internal:7100");
    // Desktop нельзя фреймить; приложение с open: iframe можно только с origin Desktop
    assert_eq!(routes[1]["handle"][0]["response"]["set"]["X-Frame-Options"][0], "DENY");
    assert_eq!(servers["app-whoami"]["routes"][0]["handle"][0]["response"]["set"]["Content-Security-Policy"][0], "frame-ancestors 'self' https://box.example");
    assert_eq!(cfg["apps"]["tls"]["automation"]["policies"][0]["issuers"][0]["module"], "internal");
    let cfg2 = EdgeCaddy::render(&EdgeInput { host: "box.example", alt_hosts: &[], cloudd_upstream: "x", coder_upstream: "y", coder_port: 8444, apps: &[], tls_internal: false });
    assert!(cfg2["apps"].get("tls").is_none(), "без internal CA — обычная автоматизация Caddy");
}

// ---------- store ----------

#[test]
fn store_roundtrip_projects_apps_grants() {
    let d = tmp();
    let s = Store::open(&d.path().join("db"), &d.path().join("age.key")).unwrap();
    assert_eq!(s.next_subnet_index().unwrap(), 1);
    let spec: ProjectSpec = serde_yaml_ng::from_str("project: alpha\ncapabilities: [demo.http]").unwrap();
    s.upsert_project(&ProjectState { name: "alpha".into(), subnet_index: 1, coder_workspace_id: None, spec, created_at: store::now() }).unwrap();
    assert_eq!(s.next_subnet_index().unwrap(), 2);
    let a = app_state(manifest("whoami", &["demo.http"], RouteMode::Port, None), Some(8450));
    s.upsert_app(&a).unwrap();
    assert_eq!(s.used_ports().unwrap(), vec![8450]);
    s.put_grant(&Grant { project: "alpha".into(), capability: "demo.http".into(), app: "whoami".into(), granted_at: store::now() }).unwrap();
    assert_eq!(s.list_grants(Some("alpha")).unwrap().len(), 1);
    let p = s.get_project("alpha").unwrap().unwrap();
    assert_eq!(p.spec.capabilities, vec!["demo.http"]);
    let mut p2 = p.clone();
    p2.coder_workspace_id = Some(uuid::Uuid::new_v4());
    s.upsert_project(&p2).unwrap();
    assert_eq!(s.get_project("alpha").unwrap().unwrap().coder_workspace_id, p2.coder_workspace_id);
    s.delete_app("whoami").unwrap();
    assert!(s.list_grants(None).unwrap().is_empty(), "удаление приложения снимает его grants");
    s.delete_project("alpha").unwrap();
    assert!(s.list_projects().unwrap().is_empty());
}

#[test]
fn store_secrets_encrypted_at_rest_and_key_reused() {
    let d = tmp();
    let db = d.path().join("db");
    let key = d.path().join("age.key");
    {
        let s = Store::open(&db, &key).unwrap();
        s.put_secret("app:garage", "GARAGE_ADMIN_TOKEN", "s3cr3t-value").unwrap();
        assert_eq!(s.get_secret("app:garage", "GARAGE_ADMIN_TOKEN").unwrap().as_deref(), Some("s3cr3t-value"));
        assert_eq!(s.get_secret("app:garage", "NOPE").unwrap(), None);
        assert_eq!(s.list_secret_keys("app:garage").unwrap(), vec!["GARAGE_ADMIN_TOKEN"]);
    }
    // в файле БД открытого текста нет
    let raw = std::fs::read(&db).unwrap();
    assert!(!raw.windows(12).any(|w| w == b"s3cr3t-value"), "секрет лежит в SQLite открытым текстом");
    // повторное открытие — тот же ключ, секрет читается
    let s = Store::open(&db, &key).unwrap();
    assert_eq!(s.get_secret("app:garage", "GARAGE_ADMIN_TOKEN").unwrap().as_deref(), Some("s3cr3t-value"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn store_events_recent_first() {
    let d = tmp();
    let s = Store::open(&d.path().join("db"), &d.path().join("k")).unwrap();
    s.event("project", "alpha", "one").unwrap();
    s.event("grant", "alpha", "two").unwrap();
    let ev = s.recent_events(10).unwrap();
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[0].message, "two");
}

// ---------- coder статус ----------

#[test]
fn coder_status_mapping() {
    let ws = |status: &str| crate::coder::Workspace {
        id: uuid::Uuid::nil(),
        name: "a".into(),
        owner_name: "admin".into(),
        latest_build: crate::coder::Build { id: uuid::Uuid::nil(), transition: "start".into(), status: status.into(), job: Default::default(), resources: vec![] },
    };
    assert_eq!(ws("running").simple_status(), "running");
    assert_eq!(ws("stopped").simple_status(), "stopped");
    assert_eq!(ws("canceled").simple_status(), "failed");
    assert_eq!(ws("starting").simple_status(), "starting");
}

// ---------- хуки ----------

#[test]
fn hook_output_parsing_and_env_prefix() {
    use crate::service::{env_prefix, parse_env};
    let out = "# комментарий\nS3_ENDPOINT=http://storage.s3.gate\nS3_SECRET_KEY=\"abc=def\"\nnot a pair\nBAD-KEY=1\n\n";
    let env = parse_env(out);
    assert_eq!(env, vec![("S3_ENDPOINT".to_string(), "http://storage.s3.gate".to_string()), ("S3_SECRET_KEY".to_string(), "abc=def".to_string())]);
    assert_eq!(env_prefix("storage.s3"), "STORAGE_S3");
    assert_eq!(env_prefix("llm"), "LLM");
}

#[test]
fn garage_manifest_has_hooks_and_generated_secrets() {
    let y = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../store/garage/manifest.yaml")).unwrap();
    let m: AppManifest = serde_yaml_ng::from_str(&y).unwrap();
    assert_eq!(m.provides, vec!["storage.s3"]);
    assert_eq!(m.kind, AppKind::System);
    assert_eq!(m.hooks.on_grant.as_deref(), Some("./hooks/grant.sh"));
    assert!(matches!(m.env["GARAGE_ADMIN_TOKEN"], EnvSpec::Generate { generate: GenerateKind::Token }));
    assert_eq!(m.endpoint.port, 3900);
}

#[test]
fn all_store_manifests_parse_and_are_consistent() {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../store"));
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let mp = p.join("manifest.yaml");
        if !mp.exists() { continue; }
        let m: AppManifest = serde_yaml_ng::from_str(&std::fs::read_to_string(&mp).unwrap()).unwrap_or_else(|err| panic!("{}: {err}", mp.display()));
        assert_eq!(Some(m.name.as_str()), p.file_name().and_then(|f| f.to_str()), "{}", mp.display());
        assert!(p.join(&m.compose).exists(), "{}: compose {} отсутствует", m.name, m.compose);
        assert!(m.endpoint.port > 0, "{}: endpoint.port", m.name);
        if let Some(icon) = &m.icon { if icon.starts_with("./") { assert!(p.join(icon).exists(), "{}: иконка {icon}", m.name); } }
        for h in [&m.hooks.on_grant, &m.hooks.on_revoke].into_iter().flatten() { assert!(p.join(h).exists(), "{}: хук {h}", m.name); }
        // login: при mode=generated пароль — переменная из env; логин — переменная из env или литерал
        if m.login.mode == LoginMode::Generated {
            let pw = m.login.password.as_deref().unwrap_or_else(|| panic!("{}: login.mode=generated без password", m.name));
            assert!(m.env.contains_key(pw), "{}: login.password {pw} не объявлен в env", m.name);
        }
        n += 1;
    }
    assert!(n >= 200, "в store ожидается импортированный каталог, найдено {n}");
}

#[test]
fn data_subdirs_from_compose() {
    let c = "volumes:\n  - ${APP_DATA_DIR}/data/gitea:/data\n  - ${APP_DATA_DIR}/torrc:/etc/tor/torrc:ro\n  - ${APP_DATA_DIR}/config.yaml:/c.yaml\nvolumes:\n  x:\n    driver_opts: { device: \"${APP_DATA_DIR}/hermes-home\" }\n";
    assert_eq!(crate::service::data_subdirs(c), vec!["data/gitea", "hermes-home", "torrc"]);
}

#[test]
fn derive_login_coolify_pairs_and_infra_excluded() {
    use serde_json::{json, Map};
    let mut env = Map::new();
    env.insert("SERVICE_USER_LINKDING".into(), json!({ "generate": "password" }));
    env.insert("SERVICE_PASSWORD_LINKDING".into(), json!({ "generate": "password" }));
    env.insert("SERVICE_PASSWORD_POSTGRES".into(), json!({ "generate": "password" }));
    env.insert("SERVICE_URL_LINKDING_9090".into(), json!("https://${APP_DOMAIN}"));
    let l = crate::importer::derive_login(&env, None, None, "linkding", "LD_SUPERUSER_NAME=${SERVICE_USER_LINKDING}\nLD_SUPERUSER_PASSWORD=${SERVICE_PASSWORD_LINKDING}\nPOSTGRES_PASSWORD: ${SERVICE_PASSWORD_POSTGRES}");
    assert_eq!(l["mode"], "generated");
    assert_eq!(l["user"], "SERVICE_USER_LINKDING");
    assert_eq!(l["password"], "SERVICE_PASSWORD_LINKDING");
    // только инфраструктурные секреты — вход настраивается в приложении
    let mut env = Map::new();
    env.insert("SERVICE_PASSWORD_JWTSECRET".into(), json!({ "generate": "password" }));
    env.insert("SERVICE_PASSWORD_POSTGRES".into(), json!({ "generate": "password" }));
    env.insert("SERVICE_PASSWORD_64_MASTERKEY".into(), json!({ "generate": "hex64" }));
    let l = crate::importer::derive_login(&env, None, None, "app", "");
    assert_eq!(l["mode"], "app");
    assert!(l["password"].is_null());
    // пароль без пары: логин из литеральной переменной каталога (GOTIFY_USERNAME=admin)
    let mut env = Map::new();
    env.insert("SERVICE_PASSWORD_GOTIFY".into(), json!({ "generate": "password" }));
    env.insert("GOTIFY_USERNAME".into(), json!("admin"));
    env.insert("GITLAB_SMTP_USER_NAME".into(), json!("x"));
    let l = crate::importer::derive_login(&env, None, None, "gotify", "GOTIFY_DEFAULTUSER_NAME=${GOTIFY_USERNAME:-admin}\nGOTIFY_DEFAULTUSER_PASS=${SERVICE_PASSWORD_GOTIFY}");
    assert_eq!(l["password"], "SERVICE_PASSWORD_GOTIFY");
    assert_eq!(l["user"], "GOTIFY_USERNAME");
}

#[test]
fn derive_login_umbrel_defaults_and_runtipi_fields() {
    use serde_json::{json, Map};
    let mut env = Map::new();
    env.insert("APP_PASSWORD".into(), json!({ "generate": "password" }));
    let l = crate::importer::derive_login(&env, Some("admin".into()), None, "web", "FLATNOTES_PASSWORD: ${APP_PASSWORD}");
    assert_eq!(l["mode"], "generated");
    assert_eq!(l["user"], "admin");
    assert_eq!(l["password"], "APP_PASSWORD");
    let l = crate::importer::derive_login(&Map::new(), Some("umbrel@umbrel.local".into()), Some("changeme".into()), "web", "");
    assert_eq!(l["mode"], "default");
    assert_eq!(l["password"], "changeme");
    assert!(l["note"].as_str().unwrap().contains("по умолчанию"));
    let mut env = Map::new();
    env.insert("DAILYTXT_ADMIN_PASSWORD".into(), json!({ "generate": "password" }));
    env.insert("DB_PASSWORD".into(), json!({ "generate": "token" }));
    env.insert("SMTP_PASSWORD".into(), json!(""));
    let l = crate::importer::derive_login(&env, None, None, "dailytxt", "");
    assert_eq!(l["password"], "DAILYTXT_ADMIN_PASSWORD");
    // пустой литерал — пароль, который пользователь задаст в настройках: ядро его узнает, режим generated
    let mut env = Map::new();
    env.insert("ADMIN_PASSWORD".into(), json!(""));
    let l = crate::importer::derive_login(&env, None, None, "x", "");
    assert_eq!(l["mode"], "generated");
    assert_eq!(l["password"], "ADMIN_PASSWORD");
    // использование в compose важнее имени: SERVICE_PASSWORD_N8N идёт в *_AUTH_TOKEN → не пароль входа; GRAFANA → GF_SECURITY_ADMIN_PASSWORD → пароль, логин — литерал соседней переменной
    let mut env = Map::new();
    env.insert("SERVICE_PASSWORD_N8N".into(), json!({ "generate": "password" }));
    assert_eq!(crate::importer::derive_login(&env, None, None, "n8n", "N8N_RUNNERS_AUTH_TOKEN=${SERVICE_PASSWORD_N8N}")["mode"], "app");
    let mut env = Map::new();
    env.insert("SERVICE_PASSWORD_GRAFANA".into(), json!({ "generate": "password" }));
    let l = crate::importer::derive_login(&env, None, None, "grafana", "      - GF_SECURITY_ADMIN_USER=admin\n      - GF_SECURITY_ADMIN_PASSWORD=${SERVICE_PASSWORD_GRAFANA}\n");
    assert_eq!(l["password"], "SERVICE_PASSWORD_GRAFANA");
    assert_eq!(l["user"], "admin");
    let mut env = Map::new();
    env.insert("SERVICE_PASSWORD_POSTGRES".into(), json!({ "generate": "password" }));
    env.insert("SERVICE_USER_POSTGRES".into(), json!({ "generate": "password" }));
    assert_eq!(crate::importer::derive_login(&env, None, None, "immich", "POSTGRES_PASSWORD: ${SERVICE_PASSWORD_POSTGRES}\nDB_PASSWORD=$SERVICE_PASSWORD_POSTGRES")["mode"], "app");
    // manifest с login.yaml-совместимым разделом парсится
    let m: AppManifest = serde_yaml_ng::from_str("name: x\ncompose: ./c.yaml\nroute: { mode: port }\nendpoint: { service: web, port: 80 }\nlogin: { mode: none }\n").unwrap();
    assert_eq!(m.login.mode, LoginMode::None);
}

#[test]
fn recommended_list_names_exist_in_store() {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../store"));
    let l: crate::service::RecommendedList = serde_yaml_ng::from_str(&std::fs::read_to_string(dir.join("recommended.yaml")).unwrap()).unwrap();
    assert!(l.items.len() >= 5);
    for it in &l.items {
        assert!(dir.join(&it.app).join("manifest.yaml").exists(), "recommended.yaml: нет приложения {}", it.app);
        assert!(!it.reason.trim().is_empty(), "recommended.yaml: {} без причины", it.app);
    }
    assert_eq!(l.items[0].app, "pipecat-voice");
}

#[test]
fn garage_manifest_provides_storage_s3_with_hooks() {
    let y = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../store/garage/manifest.yaml")).unwrap();
    let m: AppManifest = serde_yaml_ng::from_str(&y).unwrap();
    assert_eq!(m.provides, vec!["storage.s3"]);
    assert!(m.hooks.on_grant.is_some() && m.hooks.on_revoke.is_some());
    assert_eq!(m.login.mode, LoginMode::None);
    assert!(matches!(m.env.get("GARAGE_ADMIN_TOKEN"), Some(EnvSpec::Generate { .. })));
}

#[test]
fn edge_alt_hosts_share_routes_and_certificate() {
    let cfg = EdgeCaddy::render(&EdgeInput { host: "box.example", alt_hosts: &["100.64.0.9".to_string()], cloudd_upstream: "x", coder_upstream: "y", coder_port: 8444, apps: &[], tls_internal: true });
    let text = serde_json::to_string(&cfg).unwrap();
    assert!(text.contains(r#"["box.example","100.64.0.9"]"#), "host-матчеры и subjects должны включать IP: {text}");
    assert!(text.contains("frame-ancestors 'self' https://box.example https://100.64.0.9"));
}

#[test]
fn inject_links_networks_env_and_shared_alias() {
    let compose = "services:\n  app:\n    image: x\n    networks: [appnet]\n    environment:\n      - A=1\n      - OPENAI_API_KEY=${OPENAI_API_KEY}\n  db:\n    image: pg\n    networks:\n      appnet:\n        aliases: [database]\n    environment:\n      PGDATA: /x\n  host:\n    image: h\n    network_mode: host\nnetworks:\n  appnet:\n    external: true\n    name: ${APP_NET}\n";
    let mut v: serde_json::Value = serde_yaml_ng::from_str(compose).unwrap();
    crate::service::inject_links(&mut v, "hermes", "app", &[("litellm".into(), "litellm-net".into())], true, &["LITELLM_URL".into(), "OPENAI_API_KEY".into(), "OPENAI_BASE_URL".into()]);
    // сети: провайдер и общая — external
    assert_eq!(v["networks"]["link_litellm"]["name"], "litellm-net");
    assert_eq!(v["networks"]["cloudos_shared"]["name"], crate::service::SHARED_NET);
    // список сетей превращён в отображение, alias <app>.apps — только у главного сервиса
    assert!(v["services"]["app"]["networks"]["appnet"].is_object());
    assert!(v["services"]["app"]["networks"]["link_litellm"].is_object());
    assert_eq!(v["services"]["app"]["networks"]["cloudos_shared"]["aliases"][0], "hermes.apps");
    assert!(v["services"]["db"]["networks"]["cloudos_shared"]["aliases"].is_null());
    assert_eq!(v["services"]["db"]["networks"]["appnet"]["aliases"][0], "database");
    // environment: список — добавлены только отсутствующие, как ${VAR}; отображение — тоже
    let env: Vec<&str> = v["services"]["app"]["environment"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert!(env.contains(&"LITELLM_URL=${LITELLM_URL}") && env.contains(&"OPENAI_BASE_URL=${OPENAI_BASE_URL}"));
    assert_eq!(env.iter().filter(|e| e.starts_with("OPENAI_API_KEY=")).count(), 1);
    assert_eq!(v["services"]["db"]["environment"]["OPENAI_BASE_URL"], "${OPENAI_BASE_URL}");
    // сервис в сети хоста не тронут
    assert!(v["services"]["host"].get("networks").is_none());
    assert!(v["services"]["host"].get("environment").is_none());
}

#[test]
fn store_delete_app_keeps_secrets_for_reinstall() {
    let dir = tmp();
    let st = crate::store::Store::open(&dir.path().join("db.sqlite"), &dir.path().join("age.key")).unwrap();
    st.upsert_app(&app_state(manifest("x", &[], RouteMode::Port, None), Some(8500))).unwrap();
    st.put_secret("app:x", "PASSWORD", "p1").unwrap();
    st.delete_app("x").unwrap();
    assert!(st.get_app("x").unwrap().is_none());
    assert_eq!(st.get_secret("app:x", "PASSWORD").unwrap().as_deref(), Some("p1"), "обычное удаление должно сохранять пароли");
    st.delete_secrets("app:x").unwrap();
    assert!(st.get_secret("app:x", "PASSWORD").unwrap().is_none());
}

#[test]
fn generate_rotation_yields_new_value_of_same_shape() {
    for k in [GenerateKind::Token, GenerateKind::Password, GenerateKind::Hex64] {
        let a = crate::store::generate(k);
        let b = crate::store::generate(k);
        assert_ne!(a, b);
        assert_eq!(a.len(), b.len());
    }
}

#[test]
fn edge_no_h3_and_compresses_api() {
    let cfg = EdgeCaddy::render(&EdgeInput { host: "box.example", alt_hosts: &[], cloudd_upstream: "x", coder_upstream: "y", coder_port: 8444, apps: &[], tls_internal: true });
    let text = serde_json::to_string(&cfg).unwrap();
    assert!(text.contains(r#""protocols":["h1","h2"]"#));
    assert!(text.contains(r#""handler":"encode""#));
}

#[test]
fn edge_app_servers_have_friendly_error_page() {
    let apps = vec![app_state(manifest("whoami", &[], RouteMode::Port, Some(8450)), Some(8450))];
    let cfg = EdgeCaddy::render(&EdgeInput { host: "box.example", alt_hosts: &[], cloudd_upstream: "x", coder_upstream: "y", coder_port: 8444, apps: &apps, tls_internal: true });
    let errs = &cfg["apps"]["http"]["servers"]["app-whoami"]["errors"]["routes"][0]["handle"];
    assert_eq!(errs[1]["handler"], "static_response");
    assert!(errs[1]["body"].as_str().unwrap().contains("#app=whoami"));
}

#[test]
fn appdata_mounts_expand_and_owner_from_compose() {
    use crate::appdata::*;
    use std::collections::HashMap;
    let c: serde_json::Value = serde_yaml_ng::from_str(
        "services:\n  app:\n    image: x\n    user: \"${PUID:-927}:927\"\n    volumes:\n      - ${APP_DATA_DIR}/data:/data\n      - ${APP_DATA_DIR}/data/sub:/home/sub\n      - type: bind\n        source: ${APP_DATA_DIR}/cfg\n        target: /cfg\n      - ${APP_DATA_DIR}/cfg/app.json:/app.json:ro\n      - named:/var/lib/x\n  db:\n    image: y\n    user: 1000\n    environment:\n      - PUID=999\n      - PGID=${PGID:-998}\n    volumes: [\"${APP_DATA_DIR}:/var/lib/db\"]\n  web:\n    image: z\n    environment:\n      PUID: 1500\n    volumes: [\"${APP_DATA_DIR}/web:/w\"]\nvolumes:\n  named: {driver: local, driver_opts: {type: none, o: bind, device: \"${APP_DATA_DIR}/named\"}}\n").unwrap();
    let m = data_mounts(&c);
    assert_eq!(m.iter().map(|x| x.subdir.as_str()).collect::<Vec<_>>(), vec!["", "cfg", "web", "data", "data/sub"], "bind-монтирования без файлов и named volumes, родители раньше детей");
    assert_eq!(m[3].service, "app"); assert_eq!(m[3].target, "/data");
    let env: HashMap<String, String> = HashMap::new();
    let o = owner_from_compose(&c["services"]["app"], &env).unwrap();
    assert_eq!((o.uid, o.gid, o.strong), (927, 927, true), "{}", o.why);
    let o = owner_from_compose(&c["services"]["db"], &env).unwrap();
    assert_eq!((o.uid, o.gid), (1000, 1000), "user: число важнее PUID");
    let o = owner_from_compose(&c["services"]["web"], &env).unwrap();
    assert_eq!((o.uid, o.gid), (1500, 1500));
    assert!(owner_from_compose(&serde_json::json!({"image": "q"}), &env).is_none());
    let e: HashMap<String, String> = HashMap::from([("B".to_string(), "7".to_string())]);
    assert_eq!(expand("${A:-5}:${B}", &e), Some("5:7".into()));
    assert_eq!(expand("${A}", &e), None);
    assert_eq!(expand("$B-x", &e), Some("7-x".into()));
    assert_eq!(parse_ids("\"1000:1000\""), Some((1000, Some(1000))));
    assert_eq!(parse_ids("node"), None);
    assert_eq!(resolve_name("nginx:nginx", "root:x:0:0:root:/root:/bin/sh\nnginx:x:101:101:nginx:/nonexistent:/bin/false\n"), Some((101, 101)));
    assert_eq!(resolve_name("1500:0", ""), Some((1500, 0)));
    assert_eq!(service_of("openclaw-umbrel", "openclaw-umbrel-openclaw-1"), "openclaw");
    assert!(perm_error("Error: EACCES: permission denied, mkdir '/data/.openclaw'"));
    assert!(!perm_error("Setup server listening on port 18789"));
}

#[test]
fn edge_tailscale_mode_leaves_ts_net_host_to_caddy_and_keeps_internal_ca_for_alt_hosts() {
    let apps = vec![app_state(manifest("whoami", &[], RouteMode::Port, Some(8450)), Some(8450))];
    let alts = vec!["100.101.102.103".to_string()];
    let cfg = EdgeCaddy::render(&EdgeInput { host: "aios.tail1234.ts.net", alt_hosts: &alts, cloudd_upstream: "x", coder_upstream: "y", coder_port: 8444, apps: &apps, tls_internal: false });
    let policies = cfg["apps"]["tls"]["automation"]["policies"].as_array().unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0]["subjects"], serde_json::json!(["100.101.102.103"]));
    assert_eq!(policies[0]["issuers"][0]["module"], "internal");
    let no_alts = EdgeCaddy::render(&EdgeInput { host: "aios.tail1234.ts.net", alt_hosts: &[], cloudd_upstream: "x", coder_upstream: "y", coder_port: 8444, apps: &apps, tls_internal: false });
    assert!(no_alts["apps"].get("tls").is_none(), "без запасных имён политики TLS не нужны — Caddy сам обслужит .ts.net");
}

#[test]
fn backup_snapshot_names_and_prune_plan() {
    use crate::backup::*;
    assert_eq!(kind_of("cloudos-hourly-20260906-0300").as_deref(), Some("hourly"));
    assert_eq!(kind_of("cloudos-pre-remove-n8n-coolify-20260906-0300").as_deref(), Some("pre"));
    assert_eq!(kind_of("readonly"), None);
    let t = time::OffsetDateTime::from_unix_timestamp(1_788_670_800).unwrap(); // 2026-09-06 05:00 UTC
    assert_eq!(stamp(t), "20260906-050000");
    let mk = |ds: &str, kind: &str, i: i64| SnapshotInfo { dataset: ds.into(), name: format!("cloudos-{kind}-{i}"), kind: kind.into(), created: i, used: 0 };
    let mut snaps: Vec<SnapshotInfo> = (0..30).map(|i| mk("tank/apps", "hourly", i)).collect();
    snaps.extend((0..12).map(|i| mk("tank/apps", "pre", i)));
    snaps.extend((0..3).map(|i| mk("tank/apps", "manual", i)));
    snaps.extend((0..5).map(|i| mk("tank/workspaces/containers/ws-a", "hourly", i))); // дочерний датасет — не планируется отдельно
    let cfg = BackupConfig { hourly: 24, ..Default::default() };
    let plan = prune_plan(&snaps, &cfg);
    let hourly: Vec<&(String, String)> = plan.iter().filter(|(_, n)| n.contains("-hourly-")).collect();
    assert_eq!(hourly.len(), 6, "30 hourly − 24 = 6 старейших");
    assert!(hourly.iter().all(|(ds, n)| ds == "tank/apps" && (0..6).any(|i| *n == format!("cloudos-hourly-{i}"))));
    assert_eq!(plan.iter().filter(|(_, n)| n.contains("-pre-")).count(), 2, "12 pre − 10");
    assert!(plan.iter().all(|(_, n)| !n.contains("-manual-")), "manual не удаляются");
}

// ---------------------------------------------------------------------------------------------------------------------
// первый запуск и обновления (setup.rs, updates.rs)
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn tailscale_status_running_parsed() {
    let j = r#"{"Version":"1.102.3-t9329c3677-ga522f65e9","BackendState":"Running","AuthURL":"","MagicDNSSuffix":"tail0fe52f.ts.net",
        "CertDomains":["aios.tail0fe52f.ts.net"],"Health":[],"Self":{"HostName":"aios","DNSName":"aios.tail0fe52f.ts.net.","TailscaleIPs":["100.69.48.34","fd7a:115c:a1e0::ea2e:3023"],"Online":true},
        "CurrentTailnet":{"Name":"x.github","MagicDNSSuffix":"tail0fe52f.ts.net","MagicDNSEnabled":true}}"#;
    let t = crate::setup::parse_status(j);
    assert_eq!(t.state, "Running");
    assert_eq!(t.dns_name.as_deref(), Some("aios.tail0fe52f.ts.net"));
    assert_eq!(t.ips, vec!["100.69.48.34", "fd7a:115c:a1e0::ea2e:3023"]);
    assert!(t.https_certs && t.magic_dns && t.installed);
    assert_eq!(t.version.as_deref(), Some("1.102.3"));
    assert_eq!(t.tailnet.as_deref(), Some("tail0fe52f.ts.net"));
    assert!(t.auth_url.is_none());
}

#[test]
fn tailscale_status_needs_login_and_garbage() {
    let t = crate::setup::parse_status(r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.tailscale.com/a/abc","Self":{"DNSName":""}}"#);
    assert_eq!(t.state, "NeedsLogin");
    assert_eq!(t.auth_url.as_deref(), Some("https://login.tailscale.com/a/abc"));
    assert!(t.dns_name.is_none() && !t.https_certs);
    let e = crate::setup::parse_status("not json");
    assert_eq!(e.state, "Error");
    assert!(e.error.is_some());
}

#[test]
fn env_file_set_replaces_and_appends() {
    let src = "# комментарий\nCLOUDD_EDGE_HOST=10.0.0.5\nCLOUDD_TLS_INTERNAL=true\nCLOUDOS_ADMIN_EMAIL=a@b\n";
    let out = crate::setup::env_file_set(src, &[("CLOUDD_EDGE_HOST", "box.tail.ts.net"), ("CLOUDD_TLS_INTERNAL", "false"), ("CLOUDD_EDGE_ALT_HOSTS", "10.0.0.5,100.1.2.3")]);
    assert_eq!(out, "# комментарий\nCLOUDD_EDGE_HOST=box.tail.ts.net\nCLOUDD_TLS_INTERNAL=false\nCLOUDOS_ADMIN_EMAIL=a@b\nCLOUDD_EDGE_ALT_HOSTS=10.0.0.5,100.1.2.3\n");
    // ключ-префикс другого ключа не задевается
    let out2 = crate::setup::env_file_set("CLOUDD_EDGE_HOSTS_X=1\n", &[("CLOUDD_EDGE_HOST", "h")]);
    assert_eq!(out2, "CLOUDD_EDGE_HOSTS_X=1\nCLOUDD_EDGE_HOST=h\n");
}

#[test]
fn version_cmp_orders_semver() {
    use crate::updates::version_cmp;
    use std::cmp::Ordering::*;
    assert_eq!(version_cmp("0.2.0", "0.1.9"), Greater);
    assert_eq!(version_cmp("0.10.0", "0.9.1"), Greater);
    assert_eq!(version_cmp("1.0.0", "1.0"), Equal);
    assert_eq!(version_cmp("v1.2.3", "1.2.3"), Equal);
    assert_eq!(version_cmp("1.2.3-rc1", "1.2.3"), Less);
    assert_eq!(version_cmp("1.2.3+build5", "1.2.3"), Less); // метаданные сборки считаем «предварительными»
    assert_eq!(version_cmp("2", "1.99.99"), Greater);
}

#[test]
fn channel_manifest_picks_arch_asset() {
    let j = r#"{"version":"0.3.0","published":"2026-09-07","notes":"n","assets":{"x86_64-unknown-linux-gnu":{"url":"https://u/x.tar.gz","sha256":"ABCD","size":10},"aarch64":{"url":"https://u/a.tar.gz"}}}"#;
    let x = crate::updates::parse_channel(j, "x86_64").unwrap();
    assert_eq!((x.version.as_str(), x.url.as_deref(), x.sha256.as_deref(), x.size), ("0.3.0", Some("https://u/x.tar.gz"), Some("abcd"), Some(10)));
    let a = crate::updates::parse_channel(j, "aarch64").unwrap();
    assert_eq!(a.url.as_deref(), Some("https://u/a.tar.gz"));
    let r = crate::updates::parse_channel(j, "riscv64").unwrap();
    assert!(r.url.is_none(), "нет ассета — версия есть, ссылки нет");
    assert!(crate::updates::parse_channel(r#"{"notes":"x"}"#, "x86_64").is_err());
}

#[test]
fn floating_tags_from_compose() {
    let c = "services:\n  a:\n    image: traefik/whoami:latest\n  b:\n    image: ghcr.io/x/y:1.2.3\n  c:\n    image: postgres\n  d:\n    image: registry:5000/img:main\n  e:\n    image: ${IMG}\n  f:\n    image: nginx:1.25@sha256:abc\n";
    let f = crate::updates::floating_tags(c);
    assert_eq!(f, vec!["traefik/whoami:latest", "postgres", "registry:5000/img:main"]);
    assert!(crate::updates::floating_tags("nonsense: [").is_empty());
}

#[test]
fn component_titles_and_file_names() {
    use crate::updates::{component_title, safe_file_name};
    assert_eq!(component_title("03-edge", "cloudos-edge"), "Edge (Caddy)");
    assert_eq!(component_title("01-coder-incus", "01-coder-incus-postgres-1"), "Coder + PostgreSQL");
    assert_eq!(component_title("komodo", "komodo-ferretdb-1"), "Komodo (App Runtime)");
    assert_eq!(component_title("cloudos-docker-proxy", "cloudos-docker-ro"), "Docker-прокси (read-only)");
    assert_eq!(component_title("mything", "mything-web-1"), "mything");
    assert!(safe_file_name("../x.tar.gz").is_err() && safe_file_name("a/b").is_err() && safe_file_name(".hidden").is_err());
    assert_eq!(safe_file_name(" agentverse-os-0.2.0-x86_64.tar.gz ").unwrap(), "agentverse-os-0.2.0-x86_64.tar.gz");
}

fn make_package(dir: &std::path::Path, version: &str, sha_ok: bool) -> std::path::PathBuf {
    use sha2::Digest;
    let bin = b"#!/bin/sh\necho cloudd 9.9.9\n".to_vec();
    let sha = format!("{:x}", sha2::Sha256::digest(&bin));
    let manifest = serde_json::json!({ "name": "agentverse-os", "version": version, "build": "20260907-abc", "arch": "x86_64-unknown-linux-gnu", "cloudd_sha256": if sha_ok { sha } else { "00".repeat(32) } }).to_string();
    let path = dir.join(format!("agentverse-os-{version}-x86_64.tar.gz"));
    let f = std::fs::File::create(&path).unwrap();
    let enc = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
    let mut ar = tar::Builder::new(enc);
    let mut h = tar::Header::new_gnu();
    for (name, data) in [("manifest.json", manifest.as_bytes().to_vec()), ("cloudd", bin), ("store/hermes-agent/manifest.yaml", b"name: hermes-agent\n".to_vec()), ("store/hermes-agent/compose.yaml", b"services: {}\n".to_vec()), ("store/recommended.yaml", b"[]\n".to_vec())] {
        h.set_size(data.len() as u64); h.set_mode(0o644); h.set_cksum();
        ar.append_data(&mut h, name, &data[..]).unwrap();
    }
    ar.into_inner().unwrap().finish().unwrap();
    path
}

#[test]
fn package_inspect_verifies_sha_and_lists_store() {
    let d = tmp();
    let ok = make_package(d.path(), "0.3.0", true);
    let (m, apps, size, file_sha) = crate::updates::inspect_package(&ok).unwrap();
    assert_eq!((m.version.as_str(), m.arch.as_deref(), apps.as_slice()), ("0.3.0", Some("x86_64-unknown-linux-gnu"), &["hermes-agent".to_string()][..]));
    assert!(size > 0 && file_sha.len() == 64);
    let bad = make_package(d.path(), "0.3.1", false);
    let e = crate::updates::inspect_package(&bad).unwrap_err().to_string();
    assert!(e.contains("sha256"), "{e}");
}

#[test]
fn compose_differs_is_semantic() {
    use crate::updates::compose_differs;
    assert!(!compose_differs("# a\nservices:\n  x:\n    image: i:1\n", "# другой комментарий\nservices:\n  x:\n    image:   i:1\n\n"));
    assert!(compose_differs("services:\n  x:\n    image: i:1\n", "services:\n  x:\n    image: i:2\n"));
    assert!(compose_differs("services:\n  x:\n    networks:\n    - appnet\n", "services:\n  x:\n    networks:\n      appnet:\n        aliases: [a]\n"));
    assert!(!compose_differs("plain text", "plain text\n"));
}
