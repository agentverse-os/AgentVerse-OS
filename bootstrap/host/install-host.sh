#!/usr/bin/env bash
# Spike 0 · хост: Ubuntu 24.04 → Docker Engine + Incus (LTS, Zabbly) + Tailscale + sysctl + firewall-совместимость.
# Запускать после 04-zfs/zfs-layout.sh (нужны /var/lib/docker и tank/workspaces).
#
#   sudo ./install-host.sh all
#   sudo ./install-host.sh docker | incus | tailscale | firewall | verify
set -euo pipefail

INCUS_CHANNEL="${INCUS_CHANNEL:-lts-7.0}"     # проверьте актуальное имя канала на https://pkgs.zabbly.com/incus/
POOL="${POOL:-tank}"
CODENAME="$(. /etc/os-release && echo "$VERSION_CODENAME")"

install_docker() {
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu ${CODENAME} stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -q
  apt-get install -y -q docker-ce docker-ce-cli containerd.io docker-compose-plugin
  install -m 0644 "$(dirname "$0")/docker-daemon.json" /etc/docker/daemon.json
  systemctl restart docker
  docker info --format 'Docker {{.ServerVersion}} · storage {{.Driver}} · data-root {{.DockerRootDir}} · iptables backend: см. docker info | grep -i firewall'
}

install_incus() {
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://pkgs.zabbly.com/key.asc -o /etc/apt/keyrings/zabbly.asc
  cat > /etc/apt/sources.list.d/zabbly-incus-${INCUS_CHANNEL}.sources <<EOF
Enabled: yes
Types: deb
URIs: https://pkgs.zabbly.com/incus/${INCUS_CHANNEL}
Suites: ${CODENAME}
Components: main
Architectures: $(dpkg --print-architecture)
Signed-By: /etc/apt/keyrings/zabbly.asc
EOF
  apt-get update -q
  apt-get install -y -q incus incus-client
  usermod -aG incus-admin "${SUDO_USER:-root}" || true
  # Инициализация через preseed: пул default — на ZFS dataset, дефолтной сети нет (сети — по проектам, 02-networking).
  # Не используем `incus admin init --minimal`: он сам создаёт пул `default` на драйвере dir и мост incusbr0,
  # после чего `incus storage create default zfs …` падает с «already exists».
  if ! incus storage show default >/dev/null 2>&1; then
    # Пул на ZFS dataset (блочный режим даёт ext4 внутри → overlay2 для Docker в workspace работает без оговорок);
    # без пула tank (установка без DISKS) — драйвер dir в /var/lib/incus.
    local storage
    if command -v zfs >/dev/null 2>&1 && zfs list "${POOL}/workspaces" >/dev/null 2>&1; then
      storage=$'  driver: zfs\n  config:\n    source: '"${POOL}"$'/workspaces\n    volume.zfs.block_mode: "true"'
    else
      echo "нет dataset ${POOL}/workspaces — пул Incus default на драйвере dir (без снапшотов ZFS для workspaces)"
      storage=$'  driver: dir'
    fi
    incus admin init --preseed <<EOF
config: {}
networks: []
storage_pools:
- name: default
${storage}
profiles:
- name: default
  devices:
    root:
      type: disk
      path: /
      pool: default
EOF
  fi
  incus storage show default | grep -E 'driver|source|block_mode'
  incus version
}

install_tailscale() {
  curl -fsSL https://tailscale.com/install.sh | sh
  cat > /etc/default/tailscaled <<'EOF'
# Разрешить Caddy (в контейнере он root → uid 0 уже разрешён; для Caddy на хосте под своим пользователем — укажите его uid)
PORT="41641"
FLAGS=""
TS_PERMIT_CERT_UID=""
EOF
  echo "дальше: sudo tailscale up --hostname=<box>  (в админке tailnet: MagicDNS и HTTPS Certificates → Enable HTTPS)"
  echo "если на хосте ещё работает другой VPN из диапазона 100.64.0.0/10 (NetBird): sudo tailscale up --hostname=<box> --netfilter-mode=off,"
  echo "иначе правило ts-input DROP отрежет доступ по нему (Tailscale CGNAT interoperability)"
}

configure_sysctl() {
  cat > /etc/sysctl.d/90-cloudos.conf <<'EOF'
net.ipv4.ip_forward = 1
net.ipv6.conf.all.forwarding = 1
# br_netfilter включает Docker; оставляем как есть — это учитывается в 02-networking
EOF
  sysctl --system >/dev/null
}

configure_firewall() {
  # Известный конфликт: Docker ставит FORWARD policy DROP и ломает NAT/forwarding для bridge-сетей Incus.
  # Официальная рекомендация Incus: пропускать трафик Incus-мостов через DOCKER-USER. Мосты проектов называются net-*.
  install -m 0755 "$(dirname "$0")/docker-incus-firewall.sh" /usr/local/sbin/docker-incus-firewall.sh
  cat > /etc/systemd/system/docker-incus-firewall.service <<'EOF'
[Unit]
Description=Allow Incus project bridges through Docker's FORWARD chain
After=docker.service incus.service
Requires=docker.service

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/docker-incus-firewall.sh
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable --now docker-incus-firewall.service
}

verify() {
  echo "== docker";  docker run --rm alpine:3.20 sh -c 'wget -qO- https://1.1.1.1 >/dev/null && echo "PASS docker egress"' || echo "FAIL docker egress"
  echo "== incus";   incus storage list; incus network list
  echo "== firewall"; iptables -S DOCKER-USER 2>/dev/null | head -5 || nft list chain ip filter DOCKER-USER 2>/dev/null | head
  echo "== tailscale"; tailscale status --self --peers=false 2>/dev/null || echo "tailscale не поднят: sudo tailscale up"
  echo "Проверка Incus egress делается в 02-networking/spike-net.sh (после создания сети проекта)."
}

case "${1:-all}" in
  docker)    install_docker ;;
  incus)     install_incus ;;
  tailscale) install_tailscale ;;
  firewall)  configure_sysctl; configure_firewall ;;
  verify)    verify ;;
  all)       apt-get update -q; apt-get install -y -q curl ca-certificates gnupg zfsutils-linux
             install_docker; install_incus; configure_sysctl; configure_firewall; install_tailscale; verify ;;
  *) echo "usage: $0 {all|docker|incus|tailscale|firewall|verify}"; exit 1 ;;
esac
