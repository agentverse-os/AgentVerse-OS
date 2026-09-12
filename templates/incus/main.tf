# Spike 0 · Coder template: workspace = Incus system container (или VM), persistent, stop/start без пересоздания.
#
# Проверяет ADR-001 (auth-state Claude Code/Codex переживает stop/start), режимы runtime из раздела 3.4,
# Docker внутри через security.nesting, VS Code Web через модуль code-server.
#
# Ключевая механика:
#   * incus_instance.running = start_count > 0 → stop/start переключают состояние, инстанс не пересоздаётся;
#   * /home/coder — отдельный custom storage volume с именем по immutable workspace id: переживает пересоздание
#     инстанса и переименование пользователя/workspace'а (имя инстанса остаётся человеческим);
#   * агент Coder стартует systemd-юнитом на каждом boot, читая свежий токен и init-скрипт
#     из user.* конфигурации инстанса через guest API Incus (/dev/incus/sock) — cloud-init выполняется только при первом boot.
#
# Требования на стороне Coder server: доступ к сокету Incus (см. coder-compose.yml) и провайдер lxc/incus.

terraform {
  required_providers {
    coder = { source = "coder/coder" }
    incus = { source = "lxc/incus", version = ">= 1.0.0" }
  }
}

# Локальный сокет Incus (Coder server смонтирован в /var/lib/incus/unix.socket и состоит в группе incus-admin).
# Если провайдер не подхватывает сокет автоматически — см. README (INCUS_SOCKET / remote).
provider "incus" {}

data "coder_workspace" "me" {}
data "coder_workspace_owner" "me" {}

# ---------- параметры (в продукте их заполняет cloudd из project.yaml) ----------

data "coder_parameter" "runtime" {
  name         = "runtime"
  display_name = "Runtime"
  description  = "incus — контейнер без Docker внутри; incus-nesting — контейнер с Docker внутри; incus-vm — виртуальная машина"
  type         = "string"
  default      = "incus-nesting"
  mutable      = false
  option {
    name  = "incus (контейнер)"
    value = "incus"
  }
  option {
    name  = "incus-nesting (Docker внутри)"
    value = "incus-nesting"
  }
  option {
    name  = "incus-vm (VM)"
    value = "incus-vm"
  }
}

data "coder_parameter" "network" {
  name         = "network"
  display_name = "Сеть проекта"
  description  = "Управляемая сеть Incus проекта (net-<project>), создаётся заранее — см. 02-networking"
  type         = "string"
  default      = "net-alpha"
  mutable      = false
}

data "coder_parameter" "cpu" {
  name         = "cpu"
  display_name = "CPU"
  type         = "number"
  default      = 2 # стенд: 8 vCPU на хосте; целевое значение 4
  mutable      = true
  validation {
    min = 1
    max = 32
  }
}

data "coder_parameter" "memory_gb" {
  name         = "memory_gb"
  display_name = "RAM, GiB"
  type         = "number"
  default      = 4 # стенд: 14 GB на хосте; целевое значение 8
  mutable      = true
  validation {
    min = 2
    max = 128
  }
}

data "coder_parameter" "home_gb" {
  name         = "home_gb"
  display_name = "Home, GiB"
  type         = "number"
  default      = 50
  mutable      = false
  validation {
    min = 0
    max = 2000
  }
}

data "coder_parameter" "image" {
  name         = "image"
  display_name = "Образ"
  description  = "cloud-вариант обязателен (cloud-init). В продукте — собственный образ из пайплайна."
  type         = "string"
  default      = "images:ubuntu/24.04/cloud"
  mutable      = false
}

# ---------- агент Coder ----------

locals {
  nesting = data.coder_parameter.runtime.value == "incus-nesting"
  is_vm   = data.coder_parameter.runtime.value == "incus-vm"
  ws_name = lower("ws-${data.coder_workspace_owner.me.name}-${data.coder_workspace.me.name}")

  # версии CLI агентов. В спайке — "latest"; в продукте здесь конкретные версии, обновляемые осознанно,
  # не автоапдейтом (threat model): DISABLE_AUTOUPDATER ниже уже запрещает самообновление Claude Code.
  claude_code_version = "latest"
  codex_version       = "latest"
}

resource "coder_agent" "main" {
  arch = "amd64"
  os   = "linux"

  env = {
    DISABLE_AUTOUPDATER = "1" # Claude Code: не обновляться сам
    GIT_AUTHOR_NAME     = data.coder_workspace_owner.me.name
    GIT_AUTHOR_EMAIL    = data.coder_workspace_owner.me.email
  }

  # выполняется агентом при каждом старте workspace'а (не только при первом): идемпотентно
  startup_script = <<-EOT
    #!/bin/bash
    set -euo pipefail
    export PATH="$HOME/.npm-global/bin:/usr/local/bin:$PATH"

    # Node.js для CLI агентов (один раз; переживает stop/start вместе с rootfs и home)
    if ! command -v node >/dev/null 2>&1; then
      curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
      sudo apt-get install -y -q nodejs
    fi
    mkdir -p "$HOME/.npm-global"
    npm config set prefix "$HOME/.npm-global" >/dev/null
    grep -q npm-global "$HOME/.profile" || echo 'export PATH="$HOME/.npm-global/bin:$PATH"' >> "$HOME/.profile"

    # Claude Code и Codex — штатная авторизация по подписке внутри workspace (ADR-001);
    # auth-state лежит в ~/.claude и ~/.codex, то есть на persistent home volume.
    command -v claude >/dev/null 2>&1 || npm install -g "@anthropic-ai/claude-code@${local.claude_code_version}"
    command -v codex  >/dev/null 2>&1 || npm install -g "@openai/codex@${local.codex_version}"

    # Docker внутри (только incus-nesting): демон ставится cloud-init'ом при первом boot; здесь — проверка
    if [ "${local.nesting}" = "true" ]; then
      docker info >/dev/null 2>&1 && echo "docker inside: OK ($(docker info -f '{{.Driver}}'))" || echo "docker inside: NOT READY"
    fi

    echo "workspace ready: $(hostname)"
  EOT

  metadata {
    display_name = "CPU"
    key          = "cpu"
    script       = "coder stat cpu"
    interval     = 10
    timeout      = 1
  }
  metadata {
    display_name = "RAM"
    key          = "mem"
    script       = "coder stat mem"
    interval     = 10
    timeout      = 1
  }
  metadata {
    display_name = "Home"
    key          = "disk"
    script       = "coder stat disk --path /home/coder"
    interval     = 60
    timeout      = 1
  }
}

# VS Code Web (code-server) как path-based coder_app — subdomain-apps не нужны (нет wildcard)
module "code-server" {
  source   = "registry.coder.com/coder/code-server/coder"
  version  = "~> 1.0"
  agent_id = coder_agent.main.id
  folder   = "/home/coder"
}

# Альтернатива установке через startup_script — официальные модули (v5+, без Tasks):
# module "claude-code" {
#   source   = "registry.coder.com/coder/claude-code/coder"
#   version  = "~> 5.0"
#   agent_id = coder_agent.main.id
#   workdir  = "/home/coder"
# }

# ---------- cloud-init: первый boot инстанса ----------
# cloud-config собирается как HCL-объект и сериализуется в JSON (валидный YAML) — без хрупких отступов в heredoc.

locals {
  # Вне Coder init_script пуст; Incus удаляет user.*-ключ с пустым значением, а условное добавление ключа делает всю map
  # config неизвестной на этапе плана (провайдер 1.2.0 + OpenTofu 1.12 → «Provider produced invalid plan», стенд).
  # Поэтому ключ ставится всегда, а пустой скрипт заменяется на no-op.
  init_b64 = base64encode(coalesce(coder_agent.main.init_script, "#!/bin/sh\nexit 0\n"))

  agent_runner = <<-EOT
    #!/bin/sh
    # Запуск агента Coder на каждом boot. Токен и init-скрипт берутся из user.* конфигурации
    # инстанса через guest API Incus — Terraform обновляет их при каждом build.
    set -e
    SOCK=/dev/incus/sock
    for i in $(seq 1 60); do [ -S "$SOCK" ] && break; sleep 1; done
    TOKEN=$(curl -s --unix-socket "$SOCK" http://incus/1.0/config/user.coder_agent_token)
    mkdir -p /opt/coder
    curl -s --unix-socket "$SOCK" http://incus/1.0/config/user.coder_init_script | base64 -d > /opt/coder/init.sh
    chmod 0755 /opt/coder/init.sh
    exec su - coder -c "CODER_AGENT_TOKEN='$TOKEN' /opt/coder/init.sh"
  EOT

  agent_unit = <<-EOT
    [Unit]
    Description=Coder agent
    After=network-online.target${local.nesting ? " docker.service" : ""}
    Wants=network-online.target

    [Service]
    Type=simple
    ExecStart=/usr/local/sbin/coder-agent-run.sh
    Restart=always
    RestartSec=3

    [Install]
    WantedBy=multi-user.target
  EOT

  # вне VM-режима запрещаем --dangerously-skip-permissions (threat model); в VM ограничений нет
  managed_settings = local.is_vm ? jsonencode({}) : jsonencode({
    permissions = { disableBypassPermissionsMode = "disable" }
  })

  cloud_config = {
    hostname = local.ws_name
    users = [{
      name        = "coder"
      groups      = ["sudo"] # docker-группа появится после установки Docker в runcmd (usermod там же); в users её указывать нельзя
      shell       = "/bin/bash"
      sudo        = ["ALL=(ALL) NOPASSWD:ALL"]
      lock_passwd = true
    }]
    package_update = true
    packages       = ["curl", "ca-certificates", "git", "jq", "sudo", "openssh-client"]
    # Стенд NetBird: edge-Caddy с внутренним CA → workspace должен доверять его корню, иначе агент не скачает init по CODER_ACCESS_URL.
    # Файл ca.crt кладётся рядом с шаблоном (03-edge: caddy-root.crt); при Tailscale/публичном CA файла нет — блок пуст.
    ca_certs = fileexists("${path.module}/ca.crt") ? { trusted = [file("${path.module}/ca.crt")] } : {}
    write_files = [
      { path = "/etc/claude-code/managed-settings.json", permissions = "0644", content = local.managed_settings },
      { path = "/usr/local/sbin/coder-agent-run.sh", permissions = "0755", content = local.agent_runner },
      { path = "/etc/systemd/system/coder-agent.service", permissions = "0644", content = local.agent_unit },
    ]
    runcmd = concat(
      [["sh", "-c", "chown -R coder:coder /home/coder"]],
      local.nesting ? [["sh", "-c", "curl -fsSL https://get.docker.com | sh"], ["sh", "-c", "usermod -aG docker coder"]] : [],
      [["systemctl", "daemon-reload"], ["systemctl", "enable", "--now", "coder-agent.service"]]
    )
  }

  cloud_init = "#cloud-config\n${jsonencode(local.cloud_config)}"
}

# ---------- persistent home: отдельный volume, переживает пересоздание инстанса ----------

resource "incus_storage_volume" "home" {
  # Имя — по immutable id workspace'а, не по username/имени: rename не должен пересоздавать volume
  # (Coder отдельно предупреждает об этом для persistent-ресурсов).
  name = "ws-home-${data.coder_workspace.me.id}"
  pool = "default"
  type = "custom"
  # home_gb = 0 → без квоты (нужно для пулов без поддержки size, например dir на стенде без ZFS)
  config = data.coder_parameter.home_gb.value > 0 ? { size = "${data.coder_parameter.home_gb.value}GiB" } : {}
}

# ---------- инстанс ----------

resource "incus_instance" "workspace" {
  name    = local.ws_name # человеческое имя; rename workspace'а пересоздаст инстанс, но home volume (по id) сохранится
  image   = data.coder_parameter.image.value
  type    = local.is_vm ? "virtual-machine" : "container"
  running = data.coder_workspace.me.start_count > 0 # stop = running=false, инстанс сохраняется

  config = merge(
    {
      "limits.cpu"             = tostring(data.coder_parameter.cpu.value)
      "limits.memory"          = "${data.coder_parameter.memory_gb.value}GiB"
      "cloud-init.user-data"   = local.cloud_init
      "user.coder_agent_token" = nonsensitive(coder_agent.main.token) # меняется каждый build, читается юнитом при boot; nonsensitive — иначе провайдер Incus падает с «inconsistent values for sensitive attribute» (стенд)
      "user.coder_init_script" = local.init_b64
      "user.cloudos.project"   = data.coder_parameter.network.value
    },
    local.nesting ? {
      "security.nesting"                     = "true"
      "security.syscalls.intercept.mknod"    = "true"
      "security.syscalls.intercept.setxattr" = "true"
    } : {},
    local.is_vm ? {
      "limits.memory.hugepages" = "false"
    } : {}
  )

  device {
    name = "eth0"
    type = "nic"
    properties = {
      network = data.coder_parameter.network.value
      name    = "eth0"
    }
  }

  device {
    name = "home"
    type = "disk"
    properties = {
      path   = "/home/coder"
      source = incus_storage_volume.home.name
      pool   = "default"
    }
  }

  lifecycle {
    ignore_changes = [image, config["cloud-init.user-data"]] # cloud-init только при первом boot; смена образа = новый workspace
  }
}

# coder_metadata не используется: у ресурсов провайдера lxc/incus нет экспортируемого `id`, к которому Coder привязывает метаданные.
# Runtime, сеть и имя инстанса видны в параметрах workspace'а и в `incus list`.
