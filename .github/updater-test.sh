#!/bin/sh
# CI check for deploy/update/update-linux.sh: build an old install from the v0.1.0 release, update
# it, and check that only the program changed and every other file came through byte for byte.
# Needs sudo (GitHub's Ubuntu runners have it). Uses --no-self-update so it tests this checkout's
# script, not the one in the latest release.
set -eu
REPO="${REPO:-Hailey-Ross/hails.Lighthouse}"
DATA=$(mktemp -d)/lighthouse
mkdir -p "$DATA/bin" "$DATA/custom"

curl -fsSL "https://github.com/$REPO/releases/download/v0.1.0/lighthouse-x86_64-unknown-linux-gnu" -o "$DATA/bin/lighthouse"
chmod 755 "$DATA/bin/lighthouse"
cat > "$DATA/lighthouse.toml" <<'EOF'
# my own settings
mode = "mega"
poll_interval_secs = 45

[[servers]]
id = "test"
game = "7dtd"
host = "203.0.113.10"
game_port = 26900

[updates]
check = false
EOF
head -c 200000 /dev/urandom > "$DATA/lighthouse.db"
head -c 5000 /dev/urandom > "$DATA/lighthouse.db-wal"
echo '{"messages":{}}' > "$DATA/lighthouse.state.json"
echo "keep me" > "$DATA/custom/notes.txt"
before=$(cd "$DATA" && find . -type f ! -path './bin/*' -exec sha256sum {} + | sort -k2)
old_version=$("$DATA/bin/lighthouse" --version)

sudo sh deploy/update/update-linux.sh --binary "$DATA/bin/lighthouse" --data "$DATA" --no-self-update

new_version=$("$DATA/bin/lighthouse" --version)
after=$(cd "$DATA" && sudo find . -path ./backups -prune -o -type f ! -path './bin/*' -exec sha256sum {} + | sort -k2)
echo "old: $old_version"
echo "new: $new_version"
[ "$old_version" != "$new_version" ] || { echo "FAIL: program not replaced"; exit 1; }
if [ "$before" != "$after" ]; then
    echo "FAIL: install files changed"
    echo "$before" > /tmp/before.txt
    echo "$after" > /tmp/after.txt
    diff /tmp/before.txt /tmp/after.txt || true
    exit 1
fi
grep -q '^check = false' "$DATA/lighthouse.toml" || { echo "FAIL: settings were changed"; exit 1; }
sudo test -f "$DATA/bin/lighthouse.old" || { echo "FAIL: no lighthouse.old"; exit 1; }
sudo sh -c "ls -d '$DATA'/backups/update-*/SHA256SUMS.txt" >/dev/null || { echo "FAIL: no backup"; exit 1; }
echo "ok: program replaced, $(echo "$after" | wc -l) files unchanged, settings untouched, backup made"

# The opt-in: only [updates] changes, and the result is a valid config.
sudo sh deploy/update/update-linux.sh --binary "$DATA/bin/lighthouse" --data "$DATA" --no-self-update --enable-auto-updates
grep -q '^check = true' "$DATA/lighthouse.toml" && grep -q '^auto_install = true' "$DATA/lighthouse.toml" \
    && grep -q '^channel = "stable"' "$DATA/lighthouse.toml" && grep -q '^poll_interval_secs = 45' "$DATA/lighthouse.toml" \
    || { echo "FAIL: --enable-auto-updates"; cat "$DATA/lighthouse.toml"; exit 1; }
"$DATA/bin/lighthouse" --config "$DATA/lighthouse.toml" check-config
echo "ok: --enable-auto-updates"
