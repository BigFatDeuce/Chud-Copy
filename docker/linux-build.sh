#!/bin/sh
# Lance les tests du cœur sous Linux puis construit l AppImage, le RPM et le DEB. S'exécute dans l'image de compilation.
set -eu

npm ci --no-audit --no-fund
node scripts/fetch-rclone.mjs x86_64-unknown-linux-gnu

echo "=== Tests du coeur sous Linux (root) ==="
cargo test -p chud-core --manifest-path src-tauri/Cargo.toml > /tmp/tests.log 2>&1 || { tail -60 /tmp/tests.log; exit 1; }
grep -E '^test result|FAILED|panicked' /tmp/tests.log

echo "=== Tests du moteur en utilisateur normal ==="
mkdir -p /tmp/autrui && echo contenu > /tmp/autrui/autre-proprietaire.txt && chmod 644 /tmp/autrui/autre-proprietaire.txt
# Les versions récentes de Cargo rangent les tests ailleurs que dans deps : on les cherche partout.
BIN=$(find src-tauri/target/debug -type f -name 'engine-*' ! -name '*.d' -perm -u+x -exec ls -t {} + | head -1)
CHUD_FOREIGN_DIR=/tmp/autrui TMPDIR=/tmp HOME=/tmp setpriv --reuid=65534 --regid=65534 --clear-groups "$BIN" > /tmp/tests-user.log 2>&1 || { tail -40 /tmp/tests-user.log; exit 1; }
grep -E '^test result|FAILED|panicked' /tmp/tests-user.log

echo "=== Construction de l AppImage, du RPM et du DEB ==="
npx tauri build --bundles appimage,rpm,deb

mkdir -p release
cp "src-tauri/target/release/bundle/appimage/"*.AppImage release/
cp "src-tauri/target/release/bundle/rpm/"*.rpm release/
cp "src-tauri/target/release/bundle/deb/"*.deb release/
ls -la release/
