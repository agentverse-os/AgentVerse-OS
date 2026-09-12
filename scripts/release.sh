#!/usr/bin/env bash
# Пакет обновления AgentVerse OS (docs/updates.md): dist/agentverse-os-<версия>-<arch>.tar.gz и dist/stable.json (манифест канала).
# Внутри пакета: manifest.json (версия, сборка, sha256 бинаря), cloudd (с встроенным Desktop), store/ — рукописные манифесты
# (origin: manual) и recommended.yaml, bootstrap/ (установщик, update.sh, compose компонентов) и templates/ (шаблон Coder) —
# пакет самодостаточен для установки на чистую машину: tar xzf agentverse-os-*.tar.gz -C agentverse && sudo agentverse/bootstrap/install.sh Ядро принимает пакет через Desktop → Обновления (загрузка файлом) или скачивает сам
# по адресу канала: положите tar.gz и stable.json на любой статический хостинг и укажите URL stable.json в Обновлениях.
#
#   scripts/release.sh                                   # собрать Desktop и cloudd, упаковать
#   scripts/release.sh --no-build                        # только упаковать уже собранный cloudd/target/release/cloudd
#   scripts/release.sh --notes "что нового" --url-base https://example.com/agentverse   # ссылки в stable.json
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD=1; NOTES=""; URL_BASE=""
while [[ $# -gt 0 ]]; do case "$1" in
  --no-build) BUILD=0 ;; --notes) NOTES="$2"; shift ;; --url-base) URL_BASE="${2%/}"; shift ;; *) echo "неизвестный аргумент $1"; exit 1 ;;
esac; shift; done
VERSION="$(grep -m1 '^version' "$ROOT/cloudd/Cargo.toml" | cut -d'"' -f2)"
BUILD_ID="$(date -u +%Y%m%d)-$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo nogit)"
ARCH="$(uname -m)"
DIST="$ROOT/dist"; PKG="$DIST/pkg"
if [[ $BUILD -eq 1 ]]; then
  (cd "$ROOT/desktop" && npm run build >/dev/null) && echo "Desktop собран"
  (cd "$ROOT/cloudd" && CLOUDD_BUILD="$BUILD_ID" cargo build --release 2>&1 | grep -E "^error" -A8 || true) && echo "cloudd собран ($BUILD_ID)"
fi
[[ -x "$ROOT/cloudd/target/release/cloudd" ]] || { echo "нет cloudd/target/release/cloudd"; exit 1; }
rm -rf "$PKG"; mkdir -p "$PKG/store"
cp "$ROOT/cloudd/target/release/cloudd" "$PKG/cloudd"; chmod 0755 "$PKG/cloudd"
n=0
for d in "$ROOT"/store/*/; do
  [[ -f "$d/manifest.yaml" ]] && grep -q '^origin: manual' "$d/manifest.yaml" || continue
  name="$(basename "$d")"; mkdir -p "$PKG/store/$name"
  for f in "$d"/*; do [[ -f "$f" && "$(basename "$f")" != "compose.override.yaml" && "$(basename "$f")" != "login.yaml" ]] && cp "$f" "$PKG/store/$name/"; done
  n=$((n + 1))
done
[[ -f "$ROOT/store/recommended.yaml" ]] && cp "$ROOT/store/recommended.yaml" "$PKG/store/"
SHA_BIN="$(sha256sum "$PKG/cloudd" | cut -d' ' -f1)"
python3 - "$PKG/manifest.json" "$VERSION" "$BUILD_ID" "$ARCH" "$SHA_BIN" "$NOTES" <<'PY'
import json, sys
p, ver, build, arch, sha, notes = sys.argv[1:7]
json.dump({"name": "agentverse-os", "version": ver, "build": build, "arch": f"{arch}-unknown-linux-gnu", "cloudd_sha256": sha, "min_version": "0.1.0", "notes": notes or None}, open(p, "w"), ensure_ascii=False, indent=1)
PY
# bootstrap/ и templates/ — чтобы установка на новую машину шла из одного пакета: tar xzf … && sudo bootstrap/install.sh
rsync -a --exclude '*.tfstate*' --exclude .terraform "$ROOT/bootstrap" "$ROOT/templates" "$PKG/"
FILE="agentverse-os-$VERSION-$ARCH.tar.gz"
tar -C "$PKG" -czf "$DIST/$FILE" manifest.json cloudd store bootstrap templates
SHA_PKG="$(sha256sum "$DIST/$FILE" | cut -d' ' -f1)"; SIZE="$(stat -c %s "$DIST/$FILE")"
python3 - "$DIST/stable.json" "$VERSION" "$ARCH" "$URL_BASE" "$FILE" "$SHA_PKG" "$SIZE" "$NOTES" <<'PY'
import json, sys, datetime
p, ver, arch, base, file, sha, size, notes = sys.argv[1:9]
url = f"{base}/{file}" if base else file
json.dump({"version": ver, "published": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "notes": notes or None,
           "assets": {f"{arch}-unknown-linux-gnu": {"url": url, "sha256": sha, "size": int(size)}}}, open(p, "w"), ensure_ascii=False, indent=1)
PY
rm -rf "$PKG"
echo "пакет: $DIST/$FILE ($((SIZE / 1048576)) МБ, sha256 $SHA_PKG), манифестов Store: $n; канал: $DIST/stable.json"
