#!/usr/bin/env bash
# Spike 0 · ZFS layout (section 2.8 of the document, ADR: the datasets are fixed at install, the snapshot policy is a
# setting).
#
# Usage:
#   DISKS="/dev/disk/by-id/nvme-A /dev/disk/by-id/nvme-B" sudo ./zfs-layout.sh create   # create the pool (a mirror of two disks)
#   DISKS="/dev/disk/by-id/nvme-A"                        sudo ./zfs-layout.sh create   # one disk, no mirror
#   sudo ./zfs-layout.sh datasets                                                      # only the datasets on the existing tank pool
#   sudo ./zfs-layout.sh verify                                                        # check
#
# Does nothing to a disk without an explicit DISKS and the create subcommand. The script is
# idempotent for datasets.
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
    # change the mountpoint only on a mismatch: zfs set tries to remount a busy dataset (cloudd, docker)
    [[ "$(zfs get -H -o value mountpoint "$POOL/$name")" == "$mp" ]] || zfs set mountpoint="$mp" "$POOL/$name"
    for p in "$@"; do [[ "$p" == "-o" ]] && continue; zfs set "$p" "$POOL/$name"; done
    echo "есть $POOL/$name (свойства проверены)"
  fi
}

create_datasets() {
  # tank/core — cloudd state, the secret store, the manifest repository; small, backed up first
  ds core        /var/lib/cloudos    -o recordsize=128K
  # tank/postgres — PostgreSQL for Coder and cloudd; 16K for Postgres pages
  ds postgres    /srv/postgres       -o recordsize=16K -o logbias=throughput
  # tank/apps — Store app data, a child dataset per app (bind mount, not a named volume)
  ds apps        /srv/apps           -o recordsize=128K
  # tank/workspaces — the Incus storage pool; no mountpoint needed, Incus manages
  # it itself
  ds workspaces  none
  # tank/docker — the Docker data-root: images and overlay, disposable; snapshots off
  ds docker      /var/lib/docker     -o recordsize=128K -o com.sun:auto-snapshot=false
  # tank/backups — the local restic repository; separate from the
  # data
  ds backups     /srv/backups        -o recordsize=1M   -o com.sun:auto-snapshot=false

  # Example of an app child dataset (cloudd will do this at install):
  #   zfs create tank/apps/whoami   → /srv/apps/whoami

  # ARC limit (memory for the ZFS cache), applied at once and after a
  # reboot
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
