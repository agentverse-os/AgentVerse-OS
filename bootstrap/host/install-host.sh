#!/usr/bin/env bash
# Spike 0 · host: Ubuntu 24.04 → Docker Engine + Incus (LTS, Zabbly) + Tailscale + sysctl + firewall compatibility.
# Run after 04-zfs/zfs-layout.sh (/var/lib/docker and tank/workspaces are needed).
#
#   sudo ./install-host.sh all
#   sudo ./install-host.sh docker | incus | tailscale | firewall | verify
set -euo pipefail

INCUS_CHANNEL="${INCUS_CHANNEL:-lts-7.0}"     # check the current channel name at https://pkgs.zabbly.com/incus/
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
  # Initialization via preseed: the default pool on a ZFS dataset, no default network (networks are per project,
  # 02-networking). We do not use `incus admin init --minimal`: it creates the `default` pool on the dir driver and
  # the incusbr0 bridge itself, after which `incus storage create default zfs …` fails with "already exists".
  if ! incus storage show default >/dev/null 2>&1; then
    # The pool on a ZFS dataset (the block mode gives ext4 inside → overlay2 for Docker in the workspace works
    # without caveats); without the tank pool (an install without DISKS) — the dir driver in /var/lib/incus.
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
# Allow Caddy (in a container it is root → uid 0 is already allowed; for Caddy on the host under its own user — give its
# uid)
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
# br_netfilter is enabled by Docker; left as is — 02-networking takes it into
# account
EOF
  sysctl --system >/dev/null
}

configure_modules() {
  # A socket() call for a protocol family makes the host kernel load that family's module — from any container too:
  # the workspaces and the app containers share the host kernel. Nothing in the stack uses these protocols, and each of
  # them has been a way to root (the SCTP local root of November 2026 came without a fix). Ubuntu itself turns off the
  # autoload of ax25, netrom, rose, x25, decnet and rds (blacklist-rare-network.conf); `install … /bin/false` also
  # refuses an explicit modprobe. A module that is already loaded stays until it is removed or the host reboots.
  cat > /etc/modprobe.d/cloudos-hardening.conf <<'EOF'
# AgentVerse OS: network protocols and line disciplines nothing here uses (bootstrap/host/install-host.sh).
# To allow one again, delete its line.
install sctp /bin/false
install dccp /bin/false
install tipc /bin/false
install rds /bin/false
install appletalk /bin/false
install ax25 /bin/false
install netrom /bin/false
install rose /bin/false
install x25 /bin/false
install atm /bin/false
install can /bin/false
install n_hdlc /bin/false
EOF
  local m; for m in sctp dccp tipc rds appletalk ax25 netrom rose x25 atm can n_hdlc; do
    if grep -q "^$m " /proc/modules 2>/dev/null; then modprobe -r "$m" 2>/dev/null || echo "модуль $m уже загружен и занят — исчезнет после перезагрузки"; fi
  done
}

configure_firewall() {
  # A known conflict: Docker sets the FORWARD policy to DROP and breaks NAT/forwarding for Incus bridge networks. The
  # official Incus recommendation: pass the Incus bridge traffic through DOCKER-USER. The project bridges are named
  # net-*.
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
  # Docker after Incus: the project gates live on macvlan networks whose parents are the Incus bridges (net-*). When
  # Docker starts first, the bridges do not exist yet, the macvlan driver does not load those networks and the gates
  # stay down after a boot (the stand, 2026-11-04: three projects without their LLM route). cloudd repairs such gates
  # itself (Service::ensure_gate_running), the start order removes the cause.
  mkdir -p /etc/systemd/system/docker.service.d
  printf '[Unit]\nAfter=incus.service\nWants=incus.service\n' > /etc/systemd/system/docker.service.d/10-after-incus.conf
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
  firewall)  configure_sysctl; configure_modules; configure_firewall ;;
  modules)   configure_modules ;;
  verify)    verify ;;
  all)       apt-get update -q; apt-get install -y -q curl ca-certificates gnupg zfsutils-linux
             install_docker; install_incus; configure_sysctl; configure_modules; configure_firewall; install_tailscale; verify ;;
  *) echo "usage: $0 {all|docker|incus|tailscale|firewall|modules|verify}"; exit 1 ;;
esac
