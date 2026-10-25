# Spike 0 · Coder template: workspace = an Incus system container (or VM), persistent, stop/start without
# recreation.
#
# Verifies ADR-001 (the Claude Code/Codex auth state survives stop/start), the runtime modes from
# section 3.4, Docker inside via security.nesting, VS Code Web via the code-server module.
#
# Key mechanics:
# * incus_instance.running = start_count > 0 → stop/start toggle the state, the instance is not recreated;
# * /home/coder is a separate custom storage volume named by the immutable workspace id: survives recreation
# of the instance and renaming of the user/workspace (the instance name stays human-readable);
# * the Coder agent starts as a systemd unit on every boot, reading the fresh token and
# init script
# from the instance's user.* configuration via the Incus guest API (/dev/incus/sock) — cloud-init runs only on the first boot.
#
# Requirements on the Coder server side: access to the Incus socket (see coder-compose.yml) and the
# lxc/incus provider.

terraform {
  required_providers {
    coder = { source = "coder/coder" }
    incus = { source = "lxc/incus", version = ">= 1.0.0" }
  }
}

# The local Incus socket (the Coder server has /var/lib/incus/unix.socket mounted and is in the incus-admin
# group). If the provider does not pick the socket up automatically — see README (INCUS_SOCKET / remote).
provider "incus" {}

data "coder_workspace" "me" {}
data "coder_workspace_owner" "me" {}

# ---------- parameters (in the product cloudd fills them from project.yaml)
# ----------

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
  default      = 2 # stand: 8 vCPU on the host; the target value is 4
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
  default      = 4 # stand: 14 GB on the host; the target value is 8
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

# ---------- Coder agent ----------

locals {
  nesting = data.coder_parameter.runtime.value == "incus-nesting"
  is_vm   = data.coder_parameter.runtime.value == "incus-vm"
  ws_name = lower("ws-${data.coder_workspace_owner.me.name}-${data.coder_workspace.me.name}")

  # versions of the agent CLIs. In the spike — "latest"; in the product specific versions go here,
  # updated deliberately, not by auto-update (threat model): DISABLE_AUTOUPDATER below already forbids
  # Claude Code self-updates.
  claude_code_version = "latest"
  codex_version       = "latest"
  gemini_cli_version  = "latest"
  pi_version          = "latest"
}

resource "coder_agent" "main" {
  arch = "amd64"
  os   = "linux"

  env = {
    DISABLE_AUTOUPDATER = "1" # Claude Code: do not self-update
    GIT_AUTHOR_NAME     = data.coder_workspace_owner.me.name
    GIT_AUTHOR_EMAIL    = data.coder_workspace_owner.me.email
  }

  # run by the agent at every workspace start (not only the first): idempotent
  startup_script = <<-EOT
    #!/bin/bash
    set -euo pipefail
    export PATH="$HOME/.npm-global/bin:/usr/local/bin:$PATH"

    # Node.js for the agent CLIs (once; survives stop/start together with the rootfs
    # and home)
    if ! command -v node >/dev/null 2>&1; then
      curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
      sudo apt-get install -y -q nodejs
    fi
    # bubblewrap for the Codex ≥ 0.160 sandbox: in a container it works only with the setuid bit
    # (the host's apparmor_restrict_unprivileged_userns forbids a user namespace to the coder user)
    command -v bwrap >/dev/null 2>&1 || sudo apt-get install -y -q bubblewrap
    sudo chmod u+s /usr/bin/bwrap
    mkdir -p "$HOME/.npm-global"
    npm config set prefix "$HOME/.npm-global" >/dev/null
    grep -q npm-global "$HOME/.profile" || echo 'export PATH="$HOME/.npm-global/bin:$PATH"' >> "$HOME/.profile"
    # code-server terminals are an interactive non-login bash (.profile is not read): the PATH to the agent CLIs goes
    # into .bashrc too
    grep -q npm-global "$HOME/.bashrc" || echo 'export PATH="$HOME/.npm-global/bin:$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc"

    # Claude Code and Codex — regular subscription sign-in inside the workspace
    # (ADR-001); the auth state lives in ~/.claude and ~/.codex, i.e. on the persistent
    # home volume.
    command -v claude >/dev/null 2>&1 || npm install -g "@anthropic-ai/claude-code@${local.claude_code_version}"
    command -v codex  >/dev/null 2>&1 || npm install -g "@openai/codex@${local.codex_version}"
    # Gemini CLI and pi are ACP engines (docs/agents-ux.md §4.3.8); pi-acp is the ACP adapter on top of pi
    # --mode rpc
    command -v gemini >/dev/null 2>&1 || npm install -g "@google/gemini-cli@${local.gemini_cli_version}"
    command -v pi     >/dev/null 2>&1 || npm install -g --ignore-scripts "@earendil-works/pi-coding-agent@${local.pi_version}"
    command -v pi-acp >/dev/null 2>&1 || npm install -g "pi-acp@latest"

    # Docker inside (incus-nesting only): the daemon is installed by cloud-init on the first boot; here —
    # a check
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

# VS Code Web (code-server) as a path-based coder_app — subdomain apps are not needed (no
# wildcard)
module "code-server" {
  source   = "registry.coder.com/coder/code-server/coder"
  version  = "~> 1.0"
  agent_id = coder_agent.main.id
  folder   = "/home/coder"
  # Agent extensions from Open VSX at every start: Claude Code and Codex (openai.chatgpt; sign-in with a ChatGPT subscription,
  # the state in ~/.codex on the persistent home). What is installed is not re-downloaded — use_cached_extensions, code-server
  # itself updates afterwards. Without internet from the workspace the install fails and the module does not bring code-server
  # up. The capabilities environment (ANTHROPIC_BASE_URL and keys) the extensions take from the code-server process — see
  # agent_runner below.
  extensions            = ["anthropic.claude-code", "openai.chatgpt"]
  use_cached_extensions = true
}

# An alternative to installing via startup_script — the official modules (v5+, without Tasks):
# module "claude-code" {
#   source   = "registry.coder.com/coder/claude-code/coder"
#   version  = "~> 5.0"
#   agent_id = coder_agent.main.id
#   workdir  = "/home/coder"
# }

# ---------- cloud-init: the instance's first boot ----------
# cloud-config is assembled as an HCL object and serialized to JSON (valid YAML) — no fragile indentation in a heredoc.

locals {
  # Outside Coder init_script is empty; Incus removes a user.* key with an empty value, and adding the key conditionally
  # makes the whole config map unknown at plan time (provider 1.2.0 + OpenTofu 1.12 → "Provider produced invalid plan",
  # the stand). So the key is always set, and an empty script is replaced with a no-op.
  init_b64 = base64encode(coalesce(coder_agent.main.init_script, "#!/bin/sh\nexit 0\n"))

  agent_runner = <<-EOT
    #!/bin/sh
    # Starting the Coder agent on every boot. The token and init script are taken from the instance's user.* configuration via
    # the Incus guest API — Terraform updates them on every build. setpriv instead of `su -`: su opens a PAM session, and
    # logind moves the agent's children (code-server, extensions, terminals) into a session scope outside the unit's cgroup —
    # `systemctl restart coder-agent` does not kill them, the new code-server fails with EADDRINUSE. Without PAM the whole
    # tree stays in the unit's cgroup and restarts as a whole (cloudd does that when the env capabilities change). `bash -l`
    # is a login shell: /etc/profile reads /etc/profile.d/cloudos-env.sh, which cloudd places together with
    # /etc/cloudos/env.sh, so the agent, code-server, extensions and terminals inherit the project's env capabilities.
    set -e
    SOCK=/dev/incus/sock
    for i in $(seq 1 60); do [ -S "$SOCK" ] && break; sleep 1; done
    TOKEN=$(curl -s --unix-socket "$SOCK" http://incus/1.0/config/user.coder_agent_token)
    mkdir -p /opt/coder
    curl -s --unix-socket "$SOCK" http://incus/1.0/config/user.coder_init_script | base64 -d > /opt/coder/init.sh
    chmod 0755 /opt/coder/init.sh
    exec setpriv --reuid=coder --regid=coder --init-groups --reset-env env CODER_AGENT_TOKEN="$TOKEN" bash -lc 'cd "$HOME" && exec /opt/coder/init.sh'
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

  # outside the VM mode --dangerously-skip-permissions is forbidden (threat model); in a VM
  # there are no restrictions
  managed_settings = local.is_vm ? jsonencode({}) : jsonencode({
    permissions = { disableBypassPermissionsMode = "disable" }
  })

  cloud_config = {
    hostname = local.ws_name
    users = [{
      name        = "coder"
      groups      = ["sudo"] # the docker group appears after Docker is installed in runcmd (usermod there too); it must not be listed in users
      shell       = "/bin/bash"
      sudo        = ["ALL=(ALL) NOPASSWD:ALL"]
      lock_passwd = true
    }]
    package_update = true
    packages       = ["curl", "ca-certificates", "git", "jq", "sudo", "openssh-client", "bubblewrap"]
    # The NetBird stand: the edge Caddy with an internal CA → the workspace must trust its root, otherwise the agent cannot download
    # init from CODER_ACCESS_URL. The ca.crt file is placed next to the template (03-edge: caddy-root.crt); with Tailscale/a public CA
    # there is no file — the block is empty.
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

# ---------- persistent home: a separate volume, survives instance recreation ----------

resource "incus_storage_volume" "home" {
  # The name is by the immutable workspace id, not by the username/name: a rename must not
  # recreate the volume (Coder warns about this separately for persistent resources).
  name = "ws-home-${data.coder_workspace.me.id}"
  pool = "default"
  type = "custom"
  # home_gb = 0 → no quota (needed for pools without size support, e.g. dir on a stand without
  # ZFS)
  config = data.coder_parameter.home_gb.value > 0 ? { size = "${data.coder_parameter.home_gb.value}GiB" } : {}
}

# ---------- instance ----------

resource "incus_instance" "workspace" {
  name    = local.ws_name # a human-readable name; a workspace rename recreates the instance, but the home volume (by id) is kept
  image   = data.coder_parameter.image.value
  type    = local.is_vm ? "virtual-machine" : "container"
  running = data.coder_workspace.me.start_count > 0 # stop = running=false, the instance is kept

  config = merge(
    {
      "limits.cpu"             = tostring(data.coder_parameter.cpu.value)
      "limits.memory"          = "${data.coder_parameter.memory_gb.value}GiB"
      "cloud-init.user-data"   = local.cloud_init
      "user.coder_agent_token" = nonsensitive(coder_agent.main.token) # changes on every build, read by the unit at boot; nonsensitive — otherwise the Incus provider fails with "inconsistent values for sensitive attribute" (the stand)
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
    ignore_changes = [image, config["cloud-init.user-data"]] # cloud-init only on the first boot; an image change = a new workspace
  }
}

# coder_metadata is not used: the lxc/incus provider resources have no exported `id` that Coder binds metadata to. The runtime,
# network and instance name are visible in the workspace parameters and in `incus list`.
