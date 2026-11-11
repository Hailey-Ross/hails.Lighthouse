#!/bin/sh
# Lighthouse installer for Linux (systemd; x86_64 or ARM64).
#
#   curl -fsSLO https://github.com/Hailey-Ross/hails.Lighthouse/releases/download/stable/install-linux.sh
#   sudo sh install-linux.sh
#
# It downloads Lighthouse (checked against the release's checksums), creates a "lighthouse" system
# user, asks you a few questions (your game servers, your Discord bot, a dashboard password), and
# runs Lighthouse as a service that starts at boot. Data lives in /var/lib/lighthouse.
#
# Already installed? Running it again updates Lighthouse and keeps all your settings and data.
# Remove: sudo sh install-linux.sh --uninstall (keeps your settings and data).
set -eu

REPO="Hailey-Ross/hails.Lighthouse"
CHANNEL="stable"
HOME_DIR=/var/lib/lighthouse
[ "${1:-}" = "--bleeding-edge" ] && CHANNEL="bleeding-edge"

if [ "$(id -u)" -ne 0 ]; then
    echo "Please run it with sudo:  sudo sh $0" >&2
    exit 1
fi

if [ "${1:-}" = "--uninstall" ]; then
    systemctl disable --now lighthouse 2>/dev/null || true
    rm -f /etc/systemd/system/lighthouse.service
    systemctl daemon-reload
    echo "Lighthouse is stopped and won't start at boot any more."
    echo "Your settings and data are still in $HOME_DIR. Remove that folder (and the lighthouse user) to delete them."
    exit 0
fi

case "$(uname -m)" in
    x86_64|amd64) ASSET="lighthouse-x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) ASSET="lighthouse-aarch64-unknown-linux-gnu" ;;
    *) echo "Sorry, there's no Lighthouse build for $(uname -m)." >&2; exit 1 ;;
esac
if command -v curl >/dev/null 2>&1; then fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then fetch() { wget -q "$1" -O "$2"; }
else echo "Please install curl or wget first." >&2; exit 1; fi
base="https://github.com/$REPO/releases/download/$CHANNEL"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ -x "$HOME_DIR/bin/lighthouse" ]; then
    echo "Lighthouse is already installed. Updating it (your settings and data stay)."
    fetch "$base/update-linux.sh" "$tmp/update-linux.sh"
    sh "$tmp/update-linux.sh" --channel "$CHANNEL"
    exit 0
fi

echo "-> Downloading Lighthouse"
fetch "$base/$ASSET" "$tmp/$ASSET"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"
fetch "$base/lighthouse.service" "$tmp/lighthouse.service"
want=$(awk -v a="$ASSET" '$2 == a || $2 == "*"a { print $1; exit }' "$tmp/SHA256SUMS")
if [ -z "$want" ] || [ "$want" != "$(sha256sum "$tmp/$ASSET" | cut -d' ' -f1)" ]; then
    echo "The download didn't match the release's checksum. Please try again." >&2
    exit 1
fi

echo "-> Installing to $HOME_DIR"
id lighthouse >/dev/null 2>&1 || useradd --system --home "$HOME_DIR" --create-home --shell /usr/sbin/nologin lighthouse
install -d -o lighthouse -g lighthouse -m 750 "$HOME_DIR" "$HOME_DIR/bin"
install -m 755 -o lighthouse -g lighthouse "$tmp/$ASSET" "$HOME_DIR/bin/lighthouse"
"$HOME_DIR/bin/lighthouse" --version

run_as() { (cd "$HOME_DIR" && runuser -u lighthouse -- "$@"); }
if [ ! -f "$HOME_DIR/lighthouse.toml" ]; then
    echo
    echo "-> A few questions to set things up"
    echo "   Type your answer and press Enter. Use the arrow keys where you're given a list."
    echo "   Not sure about something? Pick the quick setup: you can do the rest in the dashboard."
    run_as "$HOME_DIR/bin/lighthouse" --config "$HOME_DIR/lighthouse.toml" setup </dev/tty
    [ -f "$HOME_DIR/lighthouse.toml" ] || { echo "Setup was not finished. Run this installer again to start over." >&2; exit 1; }
fi
chown lighthouse:lighthouse "$HOME_DIR/lighthouse.toml"
chmod 600 "$HOME_DIR/lighthouse.toml"
run_as "$HOME_DIR/bin/lighthouse" --config "$HOME_DIR/lighthouse.toml" check-config

echo "-> Starting Lighthouse as a service"
install -m 644 "$tmp/lighthouse.service" /etc/systemd/system/lighthouse.service
systemctl daemon-reload
systemctl enable --now lighthouse

echo
echo "All done. Lighthouse is running and starts by itself at boot."
echo "  Its log:        journalctl -u lighthouse -f"
echo "  Its settings:   $HOME_DIR/lighthouse.toml"
if grep -q '^enabled = true' "$HOME_DIR/lighthouse.toml"; then
    echo "  The dashboard:  http://127.0.0.1:8650 on this machine. From your own computer, run"
    echo "                  ssh -L 8650:127.0.0.1:8650 <you>@<this server>   and open that address."
fi
