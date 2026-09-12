#!/usr/bin/env bash
# Spike 0 · ZFS layout (раздел 2.8 документа, ADR: datasets фиксируются при установке, политика снапшотов — настройка).
#
# Использование:
#   DISKS="/dev/disk/by-id/nvme-A /dev/disk/by-id/nvme-B" sudo ./zfs-layout.sh create   # создать пул (зеркало из двух дисков)
#   DISKS="/dev/disk/by-id/nvme-A"                        sudo ./zfs-layout.sh create   # один диск, без зеркала
#   sudo ./zfs-layout.sh datasets                                                      # только datasets на существующем пуле tank
#   sudo ./zfs-layout.sh verify                                                        # проверка
#
# Ничего не делает с диском без явного DISKS и подкоманды create. Скрипт идемпотентен для datasets.
set -euo pipefail

POOL="${POOL:-tank}"
ARC_MAX_GB="${ARC_MAX_GB:-6}"

need() { command -v "$1" >/dev/null 2>&1 || { echo "нет команды: $1 (apt install zfsutils-linux)"; exit 1; }; }
need zpool; need zfs

create_pool() {
  [[ -n "${DISKS:-}" ]] || { echo "DISKS не задан — отказываюсь трогать диски"; exit 1; }
  if zpool list "$POOL" >/dev/null 2>&1; then echo "пул $POOL уже существует, пропускаю create"; return; fi
  read -r -a arr <<< "$DISKS"
  local vdev=("${arr[@]}")
  if (( ${#arr[@]} >= 2 )); then vdev=(mirror "${arr[@]}"); fi
  echo "создаю пул $POOL: ${vdev[*]}"
  zpool create -f \
    -o ashift=12 -o autotrim=on \
    -O compression=zstd -O atime=off -O xattr=sa -O acltype=posixacl -O dnodesize=auto \
    -O normalization=formD -O mountpoint=none \
    "$POOL" "${vdev[@]}"
}

ds() { # ds <name> <mountpoint|none> [zfs props...]
  local name="$1" mp="$2"; shift 2
  if ! zfs list "$POOL/$name" >/dev/null 2>&1; then
    zfs create -o mountpoint="$mp" "$@" "$POOL/$name"
    echo "создан $POOL/$name → $mp"
  else
    # менять mountpoint только при расхождении: zfs set пытается перемонтировать занятый датасет (cloudd, docker)
    [[ "$(zfs get -H -o value mountpoint "$POOL/$name")" == "$mp" ]] || zfs set mountpoint="$mp" "$POOL/$name"
    for p in "$@"; do [[ "$p" == "-o" ]] && continue; zfs set "$p" "$POOL/$name"; done
    echo "есть $POOL/$name (свойства проверены)"
  fi
}

create_datasets() {
  # tank/core — состояние cloudd, секрет-стор, репозиторий манифестов; маленький, бэкапится первым
  ds core        /var/lib/cloudos    -o recordsize=128K
  # tank/postgres — PostgreSQL для Coder и cloudd; 16K под страницы Postgres
  ds postgres    /srv/postgres       -o recordsize=16K -o logbias=throughput
  # tank/apps — данные приложений Store, по child-dataset на приложение (bind-mount, не named volume)
  ds apps        /srv/apps           -o recordsize=128K
  # tank/workspaces — storage pool Incus; mountpoint не нужен, Incus управляет сам
  ds workspaces  none
  # tank/docker — data-root Docker: образы и overlay, одноразовое; снапшоты выключены
  ds docker      /var/lib/docker     -o recordsize=128K -o com.sun:auto-snapshot=false
  # tank/backups — локальный restic-репозиторий; отдельно от данных
  ds backups     /srv/backups        -o recordsize=1M   -o com.sun:auto-snapshot=false

  # Пример child-dataset приложения (cloudd будет делать это при install):
  #   zfs create tank/apps/whoami   → /srv/apps/whoami

  # Лимит ARC (память под кэш ZFS), применяется сразу и после перезагрузки
  local bytes=$(( ARC_MAX_GB * 1024 * 1024 * 1024 ))
  echo "options zfs zfs_arc_max=${bytes}" > /etc/modprobe.d/zfs-arc.conf
  echo "$bytes" > /sys/module/zfs/parameters/zfs_arc_max || true
  echo "zfs_arc_max=${ARC_MAX_GB}G"
}

verify() {
  echo "== пул"; zpool status "$POOL" | sed -n '1,20p'
  echo "== datasets"; zfs list -o name,used,avail,mountpoint,recordsize,compression -r "$POOL"
  echo "== ARC max: $(( $(cat /sys/module/zfs/parameters/zfs_arc_max) / 1024 / 1024 / 1024 )) G"
  for d in core postgres apps workspaces docker backups; do
    zfs list "$POOL/$d" >/dev/null 2>&1 && echo "PASS dataset $POOL/$d" || echo "FAIL dataset $POOL/$d"
  done
  echo "Incus pool на $POOL/workspaces создаётся в 00-host/install-host.sh: incus storage create default zfs source=$POOL/workspaces"
}

case "${1:-}" in
  create)   create_pool; create_datasets; verify ;;
  datasets) create_datasets; verify ;;
  verify)   verify ;;
  *) echo "usage: $0 {create|datasets|verify}"; exit 1 ;;
esac
