#!/usr/bin/env bash
# AgentVerse OS update package (docs/updates.md): dist/agentverse-os-<version>-<arch>.tar.gz and dist/stable.json (the channel manifest). Inside the package: manifest.json (version, build, binary sha256), cloudd (with the
# embedded Desktop), store/ — hand-written manifests (origin: manual) and recommended.yaml, bootstrap/ (the installer, update.sh, component composes) and templates/ (the Coder template) — the package is self-sufficient for
# an install on a clean machine: tar xzf agentverse-os-*.tar.gz -C agentverse && sudo agentverse/bootstrap/install.sh. The core accepts the package through Desktop → Updates (file upload) or downloads it itself from the
# channel address: put the tar.gz and stable.json on any static hosting and set the stable.json URL in Updates.
#
#   scripts/release.sh                                   # build Desktop and cloudd, pack
#   scripts/release.sh --no-build                        # only pack the already built cloudd/target/release/cloudd
#   scripts/release.sh --notes "what's new" --url-base https://example.com/agentverse   # links in stable.json
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
  # Reproducible paths in the binaries: without a remap the paths of the build machine (/home/<user>/.cargo/registry/…, the repository
  # directory) get into cloudd/agentd/agents — through debuginfo and the Location::caller() of panics. We rewrite them to /cargo and
  # /src and strip the symbols; a RUSTFLAGS change rebuilds the dependencies, so the first release after this takes longer than usual.
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$ROOT=/src"
  export CARGO_PROFILE_RELEASE_STRIP=true
  (cd "$ROOT/desktop" && npm run build >/dev/null) && echo "Desktop собран"
  # agentd before cloudd: cloudd embeds its binary (build.rs). An error in any build stops the release — otherwise an old binary would go into
  # the package.
  (cd "$ROOT/agentd" && cargo build --release >/tmp/agentd-build.log 2>&1 || { grep -E "^error" -A8 /tmp/agentd-build.log; echo "сборка agentd не удалась"; exit 1; }) && echo "agentd собран" || exit 1
  # the "Agents" service (ADR-006, stage 2): the agents/ui web app → embedded into cloudos-agents → embedded into
  # cloudd
  (cd "$ROOT/agents/ui" && npm run build >/tmp/agents-ui-build.log 2>&1 || { tail -20 /tmp/agents-ui-build.log; echo "сборка agents/ui не удалась"; exit 1; }) && echo "agents/ui собран" || exit 1
  (cd "$ROOT/agents" && cargo build --release >/tmp/agents-build.log 2>&1 || { grep -E "^error" -A8 /tmp/agents-build.log; echo "сборка cloudos-agents не удалась"; exit 1; }) && echo "cloudos-agents собран" || exit 1
  (cd "$ROOT/cloudd" && CLOUDD_BUILD="$BUILD_ID" cargo build --release >/tmp/cloudd-build.log 2>&1 || { grep -E "^error" -A8 /tmp/cloudd-build.log; echo "сборка cloudd не удалась"; exit 1; }) && echo "cloudd собран ($BUILD_ID)" || exit 1
fi
[[ -x "$ROOT/cloudd/target/release/cloudd" ]] || { echo "нет cloudd/target/release/cloudd"; exit 1; }
rm -rf "$PKG"; mkdir -p "$PKG/store"
cp "$ROOT/cloudd/target/release/cloudd" "$PKG/cloudd"; chmod 0755 "$PKG/cloudd"
n=0
for d in "$ROOT"/store/*/; do
  [[ -f "$d/manifest.yaml" ]] && grep -q '^origin: manual' "$d/manifest.yaml" || continue
  name="$(basename "$d")"; mkdir -p "$PKG/store/$name"
  # the whole directory, including hooks/ and other subdirectories; without the install files (overrides, login.yaml,
  # user.env)
  for f in "$d"/*; do case "$(basename "$f")" in compose.override.yaml|login.yaml|user.env) continue ;; esac; cp -r "$f" "$PKG/store/$name/"; done
  n=$((n + 1))
done
[[ -f "$ROOT/store/recommended.yaml" ]] && cp "$ROOT/store/recommended.yaml" "$PKG/store/"
SHA_BIN="$(sha256sum "$PKG/cloudd" | cut -d' ' -f1)"
python3 - "$PKG/manifest.json" "$VERSION" "$BUILD_ID" "$ARCH" "$SHA_BIN" "$NOTES" <<'PY'
import json, sys
p, ver, build, arch, sha, notes = sys.argv[1:7]
json.dump({"name": "agentverse-os", "version": ver, "build": build, "arch": f"{arch}-unknown-linux-gnu", "cloudd_sha256": sha, "min_version": "0.1.0", "notes": notes or None}, open(p, "w"), ensure_ascii=False, indent=1)
PY
# bootstrap/ and templates/ — so an install on a new machine goes from one package: tar xzf … && sudo
# bootstrap/install.sh
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
