#!/bin/sh
# CI check for deploy/install/install-linux.sh on a real systemd machine (GitHub's Ubuntu runner).
# Settings are placed first, so the installer skips its questions. Uses the bleeding-edge channel,
# which carries the newest helper scripts.
set -eu
DATA=/var/lib/lighthouse
sudo install -d "$DATA"
sudo tee "$DATA/lighthouse.toml" >/dev/null <<'TOML'
mode = "mega"

[[servers]]
id = "test"
game = "7dtd"
host = "203.0.113.10"
game_port = 26900
TOML

sudo sh deploy/install/install-linux.sh --bleeding-edge
sleep 5
systemctl is-active --quiet lighthouse || { sudo journalctl -u lighthouse --no-pager | tail -20; echo "FAIL: service not running"; exit 1; }
sudo journalctl -u lighthouse --no-pager | grep -q "Lighthouse is running" || { echo "FAIL: Lighthouse did not start"; exit 1; }
echo "ok: installed and running"

before=$(sudo sha256sum "$DATA/lighthouse.toml")
sudo sh deploy/install/install-linux.sh --bleeding-edge
sleep 3
systemctl is-active --quiet lighthouse || { echo "FAIL: not running after the update run"; exit 1; }
[ "$before" = "$(sudo sha256sum "$DATA/lighthouse.toml")" ] || { echo "FAIL: settings changed by the update run"; exit 1; }
echo "ok: second run updated and kept the settings"

sudo sh deploy/install/install-linux.sh --uninstall
! systemctl is-active --quiet lighthouse || { echo "FAIL: still running after uninstall"; exit 1; }
sudo test -f "$DATA/lighthouse.toml" || { echo "FAIL: uninstall removed the settings"; exit 1; }
echo "ok: uninstall stopped it and kept the settings"
