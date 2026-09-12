//! Контракты (раздел 2 архитектуры): `project.yaml`, манифест приложения, состояние ядра.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ---------- project.yaml (2.7) ----------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProjectSpec {
    #[serde(default = "one")]
    pub schema: u32,
    pub project: String,
    #[serde(default)]
    pub workspace: WorkspaceSpec,
    #[serde(default)]
    pub agents: Vec<String>,
    /// Имена capabilities; в срезе 1 capability резолвится в установленное приложение, которое её `provides`,
    /// и превращается в маршрут `http://<capability>.gate` на gate проекта.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceSpec {
    #[serde(default = "default_runtime")]
    pub runtime: Runtime,
    #[serde(default = "default_cpu")]
    pub cpu: u32,
    /// В GiB; в YAML допускается `8GiB`/`8`.
    #[serde(default = "default_memory", deserialize_with = "de_gib", serialize_with = "ser_gib")]
    pub memory: u32,
    #[serde(default = "default_home", deserialize_with = "de_gib", serialize_with = "ser_gib")]
    pub home: u32,
    #[serde(default)]
    pub image: Option<String>,
}

impl Default for WorkspaceSpec {
    fn default() -> Self {
        Self { runtime: default_runtime(), cpu: default_cpu(), memory: default_memory(), home: default_home(), image: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Runtime {
    Incus,
    IncusNesting,
    IncusVm,
}
impl Runtime {
    pub fn as_param(&self) -> &'static str {
        match self {
            Runtime::Incus => "incus",
            Runtime::IncusNesting => "incus-nesting",
            Runtime::IncusVm => "incus-vm",
        }
    }
}

fn one() -> u32 { 1 }
fn default_runtime() -> Runtime { Runtime::IncusNesting }
fn default_cpu() -> u32 { 2 }
fn default_memory() -> u32 { 4 }
fn default_home() -> u32 { 50 }

fn de_gib<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V { N(u32), S(String) }
    match V::deserialize(d)? {
        V::N(n) => Ok(n),
        V::S(s) => {
            let t = s.trim().trim_end_matches("GiB").trim_end_matches("Gi").trim_end_matches('G').trim();
            t.parse().map_err(|_| serde::de::Error::custom(format!("ожидалось число GiB, получено {s:?}")))
        }
    }
}
fn ser_gib<S: serde::Serializer>(v: &u32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("{v}GiB"))
}

// ---------- манифест приложения (2.6) ----------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppManifest {
    #[serde(default = "one")]
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Переводы текстов манифеста по языку интерфейса Desktop (ключи en/ru/uk/es): показывается `i18n.<lang>.description`,
    /// без перевода — `description`. У импортированных манифестов пусто: их описания английские, из upstream.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub i18n: std::collections::BTreeMap<String, I18nText>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: AppKind,
    /// Путь к compose относительно каталога манифеста.
    pub compose: String,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    pub route: RouteSpec,
    #[serde(default)]
    pub auth: Option<String>,
    /// Генерируемые ядром переменные окружения → секрет-стор → `.env` стека.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, EnvSpec>,
    /// Что снапшотить и бэкапить (`tank/apps/<name>`); ядро создаёт каталог `APP_DATA_DIR`.
    #[serde(default)]
    pub data: Vec<String>,
    /// URL healthcheck внутри сети приложения (`http://<service>:<port>/path`).
    #[serde(default)]
    pub health: Option<String>,
    /// Куда ведут маршруты: сервис и порт внутри `<app>-net` (для edge и gate).
    pub endpoint: Endpoint,
    #[serde(default)]
    pub hooks: Hooks,
    /// `manual` (рукописный) | `runtipi` (импортёр); импортёр не трогает manual.
    #[serde(default = "manual")]
    pub origin: String,
    /// Метаданные upstream-каталога: source, author, version, categories.
    #[serde(default)]
    pub upstream: Option<serde_json::Value>,
    /// Настройки приложения из каталога (form_fields) — для UI; значения живут в `env`.
    #[serde(default)]
    pub settings: Vec<serde_json::Value>,
    /// Монтирует docker.sock хоста — показать предупреждение (уровень доверия платформы).
    #[serde(default)]
    pub host_docker_socket: bool,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Скриншоты из каталога (URL); показываются в карточке приложения.
    #[serde(default)]
    pub gallery: Vec<String>,
    /// Длинное описание (markdown) — путь к файлу рядом с манифестом (`./description.md`).
    #[serde(default)]
    pub readme: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Данные для входа (раздел «Доступ» в Desktop): что ядро знает о логине и пароле приложения.
    #[serde(default)]
    pub login: LoginSpec,
    /// Кураторский список «Рекомендуем» (store/recommended.yaml): место на полке и причина. Подмешивается при чтении манифеста.
    #[serde(default)]
    pub recommended: Option<Recommended>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Recommended {
    pub rank: u32,
    pub reason: String,
    /// Переводы причины по языку интерфейса (`i18n.<lang>.reason`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub i18n: std::collections::BTreeMap<String, I18nText>,
}

/// Переводимые тексты манифеста и списка «Рекомендуем» на одном языке.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct I18nText {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn manual() -> String { "manual".into() }

/// Как войти в приложение. `user`/`password` — имя переменной из `env` (значение берётся из секрет-стора или настроек)
/// либо литерал. Заполняется импортёром (см. `importer::derive_login`), уточняется файлом `login.yaml` рядом с манифестом.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct LoginSpec {
    #[serde(default)]
    pub mode: LoginMode,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// Путь страницы входа относительно URL приложения.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum LoginMode {
    /// Логин и пароль знает ядро: сгенерированы при установке или заданы в настройках.
    Generated,
    /// Пароль по умолчанию из образа приложения (литерал) — сменить после первого входа.
    Default,
    /// Вход настраивается в самом приложении (мастер первого запуска) или его нет — по каталогу не отличить.
    #[default]
    App,
    /// Входа нет: доступ ограничен сетью Cloud OS.
    None,
    /// Вход через внешний сервис (OAuth и т.п.).
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AppKind {
    #[default]
    App,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Endpoint {
    pub service: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RouteSpec {
    pub mode: RouteMode,
    /// Для `path`: префикс `/apps/<name>/`; для `port`: желаемый порт (иначе выдаст ядро); для `host` — поддомен.
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub open: OpenMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    Path,
    Port,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OpenMode {
    #[default]
    Iframe,
    Newtab,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EnvSpec {
    Literal(String),
    Generate { generate: GenerateKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum GenerateKind {
    /// 32 hex-символа (16 байт)
    Token,
    /// 24 символа [A-Za-z0-9] без похожих
    Password,
    /// 64 hex-символа (32 байта) — rpc_secret Garage и подобные
    Hex64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Hooks {
    #[serde(default)]
    pub on_grant: Option<String>,
    #[serde(default)]
    pub on_revoke: Option<String>,
}

// ---------- состояние (что ядро создало) ----------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProjectState {
    pub name: String,
    /// Третий октет подсети проекта: `10.77.<n>.0/24`.
    pub subnet_index: u8,
    pub coder_workspace_id: Option<uuid::Uuid>,
    pub spec: ProjectSpec,
    pub created_at: String,
}

impl ProjectState {
    pub fn incus_network(&self) -> String { format!("net-{}", self.name) }
    pub fn macvlan_network(&self) -> String { format!("mv-net-{}", self.name) }
    pub fn gate_name(&self) -> String { format!("gate-{}", self.name) }
    pub fn subnet(&self, base: &str) -> String { format!("{base}.{}.0/24", self.subnet_index) }
    pub fn gateway(&self, base: &str) -> String { format!("{base}.{}.1", self.subnet_index) }
    pub fn gate_ip(&self, base: &str) -> String { format!("{base}.{}.250", self.subnet_index) }
    pub fn gate_ip_range(&self, base: &str) -> String { format!("{base}.{}.240/28", self.subnet_index) }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppState {
    pub name: String,
    pub manifest: AppManifest,
    /// Выданный порт для `route: port`.
    pub port: Option<u16>,
    pub komodo_stack_id: Option<String>,
    pub installed_at: String,
}

impl AppState {
    pub fn network(&self) -> String { format!("{}-net", self.name) }
    pub fn stack_name(&self) -> String { self.name.clone() }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Grant {
    pub project: String,
    pub capability: String,
    pub app: String,
    pub granted_at: String,
}

/// Связь приложений: потребитель подключён к сети провайдера и получает его адрес (и ключи из хуков) в env.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppLink {
    pub consumer: String,
    pub provider: String,
    pub created_at: String,
}

// ---------- сводки для API/Desktop ----------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProjectView {
    pub name: String,
    pub spec: ProjectSpec,
    pub network: String,
    pub subnet: String,
    pub gate: ComponentHealth,
    pub workspace: Option<WorkspaceView>,
    pub grants: Vec<Grant>,
    pub missing_capabilities: Vec<String>,
    /// Имена переменных env-контракта по capability (значения — секреты, наружу не отдаются).
    #[serde(default)]
    pub grant_env: std::collections::BTreeMap<String, Vec<String>>,
    /// Capabilities, которые можно выдать (есть установленный provider).
    #[serde(default)]
    pub available_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceView {
    pub id: uuid::Uuid,
    pub name: String,
    /// `running | stopped | starting | stopping | pending | failed | deleting | deleted | unknown`
    pub status: String,
    pub agent_status: Option<String>,
    pub instance: Option<String>,
    pub ip: Option<String>,
    pub apps: Vec<WorkspaceApp>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceApp {
    pub slug: String,
    pub display_name: String,
    pub url: String,
    pub health: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppView {
    pub name: String,
    pub manifest: AppManifest,
    pub installed: bool,
    pub port: Option<u16>,
    /// Публичный URL через edge (если установлено).
    pub url: Option<String>,
    pub state: Option<String>,
    pub granted_to: Vec<String>,
    /// Текущие значения настраиваемых env (секретные типы — замаскированы).
    #[serde(default)]
    pub settings_values: std::collections::BTreeMap<String, String>,
    /// Связи: к каким приложениям подключено (провайдеры) и кто подключён к нему (потребители).
    #[serde(default)]
    pub links_to: Vec<String>,
    #[serde(default)]
    pub links_from: Vec<String>,
    /// В общей сети приложений (имя `<app>.apps`).
    #[serde(default)]
    pub shared_net: bool,
    /// Адрес внутри Cloud OS: `http://<endpoint.service>:<port>` — для подключённых приложений и общей сети.
    #[serde(default)]
    pub internal_url: Option<String>,
    /// Переменные, которые приложение получает от связей (секреты замаскированы).
    #[serde(default)]
    pub link_env: std::collections::BTreeMap<String, String>,
    /// Есть заметка пользователя (шифруется как секрет; текст — отдельным запросом).
    #[serde(default)]
    pub has_note: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ComponentHealth {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SystemStatus {
    pub edge_host: String,
    /// Дополнительные адреса Entry Point (IP для устройств без MagicDNS).
    #[serde(default)]
    pub edge_alt_hosts: Vec<String>,
    pub coder_url: String,
    /// Логин администратора Coder и есть ли у ядра его пароль (показ — /api/system/credentials/coder/reveal).
    #[serde(default)]
    pub coder_user: String,
    #[serde(default)]
    pub coder_password_set: bool,
    pub components: Vec<ComponentHealth>,
    pub version: String,
    /// Идентификатор сборки (CLOUDD_BUILD при сборке; dev).
    #[serde(default)]
    pub build: String,
    /// Мастер первого запуска пройден (kv setup.done).
    #[serde(default)]
    pub setup_done: bool,
    /// По последней проверке канала: есть обновление системы; сколько приложений можно обновить из каталога.
    #[serde(default)]
    pub update_available: bool,
    #[serde(default)]
    pub app_updates: u32,
}
