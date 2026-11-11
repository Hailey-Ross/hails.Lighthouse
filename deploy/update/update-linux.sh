#!/bin/sh
# Updates an existing Lighthouse install on Linux to the latest release, for installs that can't
# update themselves (an older build, or update checks turned off).
#
#   sudo sh update-linux.sh                    # the systemd install in /var/lib/lighthouse
#   sudo sh update-linux.sh --binary /opt/lighthouse/lighthouse --data /opt/lighthouse
#   sudo sh update-linux.sh --channel bleeding-edge
#   sudo sh update-linux.sh --enable-auto-updates
#
# Only the program is replaced. Everything else that belongs to the install (lighthouse.toml, the
# database and its journal files, the saved post ids, any other file in the data folder) is kept
# exactly as it is:
#   1. Lighthouse is stopped, so the database is consistent.
#   2. Every file in the data folder is copied to <data>/backups/update-<time>/ with a checksum list.
#   3. The program is replaced (after checking the download against the release's SHA256SUMS).
#   4. Every file is checked against the checksum list; anything missing or changed is put back
#      from the backup, and the update is reported as failed.
#   5. Lighthouse is started again. The backup stays (the newest 3 are kept).
# Settings are not changed, unless you pass --enable-auto-updates.
#
# Before anything else the updater updates itself: it fetches the current update-linux.sh from the
# same release (checked against the release's SHA256SUMS) and, if it differs from this copy, runs
# that one instead with the same options.
set -eu
ORIGINAL_ARGS="$*"

REPO="Hailey-Ross/hails.Lighthouse"
CHANNEL="stable"
BINARY="/var/lib/lighthouse/bin/lighthouse"
DATA="/var/lib/lighthouse"
AUTO=0
SELF_UPDATE=1
KEEP_BACKUPS=3

while [ $# -gt 0 ]; do
    case "$1" in
        --channel) CHANNEL="$2"; shift 2 ;;
        --binary) BINARY="$2"; shift 2 ;;
        --data) DATA="$2"; shift 2 ;;
        --repo) REPO="$2"; shift 2 ;;
        --enable-auto-updates) AUTO=1; shift ;;
        --no-self-update) SELF_UPDATE=0; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
case "$CHANNEL" in stable|bleeding-edge) ;; *) echo "--channel is stable or bleeding-edge" >&2; exit 2 ;; esac
if [ "$(id -u)" -ne 0 ]; then echo "Please run with sudo." >&2; exit 1; fi
if [ ! -x "$BINARY" ]; then echo "No Lighthouse at $BINARY. Pass --binary <path to the program>." >&2; exit 1; fi
if [ ! -d "$DATA" ]; then echo "No data folder at $DATA. Pass --data <folder with lighthouse.toml>." >&2; exit 1; fi
DATA=$(cd "$DATA" && pwd)

case "$(uname -m)" in
    x86_64|amd64) ASSET="lighthouse-x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) ASSET="lighthouse-aarch64-unknown-linux-gnu" ;;
    *) echo "No Linux build for $(uname -m)." >&2; exit 1 ;;
esac
if command -v curl >/dev/null 2>&1; then fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then fetch() { wget -q "$1" -O "$2"; }
else echo "Needs curl or wget." >&2; exit 1; fi

# ---------- first, make sure this updater is the current one ----------
if [ "$SELF_UPDATE" -eq 1 ] && [ -f "$0" ]; then
    self_tmp=$(mktemp -d)
    base="https://github.com/$REPO/releases/download/$CHANNEL"
    if fetch "$base/update-linux.sh" "$self_tmp/update-linux.sh" 2>/dev/null && fetch "$base/SHA256SUMS" "$self_tmp/SHA256SUMS" 2>/dev/null; then
        want=$(awk '$2 == "update-linux.sh" || $2 == "*update-linux.sh" { print $1; exit }' "$self_tmp/SHA256SUMS")
        got=$(sha256sum "$self_tmp/update-linux.sh" | cut -d' ' -f1)
        if [ -n "$want" ] && [ "$want" = "$got" ]; then
            if [ "$got" != "$(sha256sum "$0" | cut -d' ' -f1)" ]; then
                echo "-> A newer version of this updater is available; using it"
                # shellcheck disable=SC2086
                code=0
                sh "$self_tmp/update-linux.sh" $ORIGINAL_ARGS --no-self-update || code=$?
                rm -rf "$self_tmp"
                exit $code
            fi
            echo "   updater is current"
        else
            echo "   could not verify a newer updater; continuing with this one"
        fi
    else
        echo "   could not check for a newer updater; continuing with this one"
    fi
    rm -rf "$self_tmp"
fi

echo "-> Updating $BINARY (data in $DATA)"
echo "   now: $("$BINARY" --version 2>/dev/null || echo unknown)"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# ---------- download and verify (before touching anything) ----------
base="https://github.com/$REPO/releases/download/$CHANNEL"
echo "-> Downloading the $CHANNEL build"
fetch "$base/$ASSET" "$tmp/$ASSET"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"
want=$(awk -v a="$ASSET" '$2 == a || $2 == "*"a { print $1; exit }' "$tmp/SHA256SUMS")
got=$(sha256sum "$tmp/$ASSET" | cut -d' ' -f1)
if [ -z "$want" ] || [ "$want" != "$got" ]; then
    echo "The download doesn't match the release checksum; nothing was changed." >&2
    exit 1
fi
echo "   checksum ok"

# ---------- stop ----------
service=0
if command -v systemctl >/dev/null 2>&1 && systemctl cat lighthouse.service >/dev/null 2>&1; then service=1; fi
echo "-> Stopping Lighthouse"
if [ "$service" -eq 1 ]; then systemctl stop lighthouse; fi

# ---------- back up the install's files ----------
# Everything in the data folder except the program itself and earlier backups.
list_files() {
    (cd "$DATA" && find . -type f ! -path './backups/*' -print | sed 's#^\./##') |
        while IFS= read -r f; do
            [ "$DATA/$f" = "$BINARY" ] || [ "$DATA/$f" = "$BINARY.old" ] || printf '%s\n' "$f"
        done
}
stamp=$(date +%Y%m%d-%H%M%S)
backup="$DATA/backups/update-$stamp"
echo "-> Backing up settings and data to backups/update-$stamp"
mkdir -p "$backup"
list_files > "$tmp/files"
(cd "$DATA" && while IFS= read -r f; do
    mkdir -p "$backup/$(dirname "$f")"
    cp -p "$f" "$backup/$f"
done < "$tmp/files")
(cd "$DATA" && while IFS= read -r f; do sha256sum "$f"; done < "$tmp/files") > "$backup/SHA256SUMS.txt"
cp -p "$BINARY" "$backup/lighthouse.program"
chown -R --reference="$DATA" "$backup" 2>/dev/null || true
chmod -R go-rwx "$backup"
echo "   $(wc -l < "$tmp/files" | tr -d ' ') file(s) backed up"

# ---------- swap the program ----------
echo "-> Replacing the program"
owner=$(stat -c '%U:%G' "$BINARY")
cp -p "$BINARY" "$BINARY.old"
install -m 755 "$tmp/$ASSET" "$BINARY"
chown "$owner" "$BINARY"

# ---------- check every file came through ----------
echo "-> Checking settings and data"
if ! (cd "$DATA" && sha256sum -c --quiet "$backup/SHA256SUMS.txt") >/dev/null 2>&1; then
    (cd "$DATA" && sha256sum -c "$backup/SHA256SUMS.txt" 2>/dev/null | grep -v ': OK$' | sed 's/: FAILED.*//') > "$tmp/bad" || true
    while IFS= read -r f; do
        mkdir -p "$DATA/$(dirname "$f")"
        cp -p "$backup/$f" "$DATA/$f"
    done < "$tmp/bad"
    echo "These files changed during the update and were put back from the backup:" >&2
    cat "$tmp/bad" >&2
    echo "Check them before starting Lighthouse." >&2
    exit 1
fi
echo "   all $(wc -l < "$tmp/files" | tr -d ' ') file(s) unchanged"

# Keep the newest backups only.
ls -1d "$DATA"/backups/update-* 2>/dev/null | sort -r | tail -n +$((KEEP_BACKUPS + 1)) | while IFS= read -r old; do rm -rf "$old"; done

# ---------- optional: let it update itself from now on ----------
CONFIG="$DATA/lighthouse.toml"
if [ "$AUTO" -eq 1 ] && [ -f "$CONFIG" ]; then
    awk -v ch="$CHANNEL" '
        function flush() {
            if (!c) print "check = true"; if (!a) print "auto_install = true"; if (!h) print "channel = \"" ch "\""
        }
        /^[[:space:]]*\[/ { if (inside) flush(); inside = ($0 ~ /^[[:space:]]*\[updates\][[:space:]]*$/); if (inside) { seen = 1; c = a = h = 0 } print; next }
        inside && /^[[:space:]]*check[[:space:]]*=/        { print "check = true"; c = 1; next }
        inside && /^[[:space:]]*auto_install[[:space:]]*=/ { print "auto_install = true"; a = 1; next }
        inside && /^[[:space:]]*channel[[:space:]]*=/      { print "channel = \"" ch "\""; h = 1; next }
        { print }
        END { if (inside) flush(); if (!seen) { print ""; print "[updates]"; c = a = h = 0; flush() } }
    ' "$CONFIG" > "$tmp/lighthouse.toml"
    cat "$tmp/lighthouse.toml" > "$CONFIG"   # keeps the file's owner and permissions
    echo "   automatic updates turned on in lighthouse.toml ($CHANNEL); the original is in the backup"
fi

if [ "$service" -eq 1 ]; then
    echo "-> Starting Lighthouse"
    systemctl start lighthouse
else
    echo "   No lighthouse service here; start Lighthouse the way you usually do."
fi
echo
echo "Done: $("$BINARY" --version 2>/dev/null)"
echo "Settings and data are unchanged; a copy is in $DATA/backups. The previous program is kept as $BINARY.old."
