#!/bin/bash
# Build, sign with a stable local certificate (permissions survive rebuilds), install, start at login.
set -euo pipefail
cd "$(dirname "$0")"
APP=/Applications/LayoutFix.app
CERT="LayoutFix Local"
AGENT=~/Library/LaunchAgents/local.layoutfix.plist

cargo test --quiet
cargo build --release --quiet

if ! security find-identity -p codesigning | grep -q "\"$CERT\""; then
  tmp=$(mktemp -d)
  printf '[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=%s\n[ext]\nbasicConstraints=critical,CA:false\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,codeSigning\n' "$CERT" > "$tmp/c.cnf"
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$tmp/k.pem" -out "$tmp/c.pem" -days 3650 -config "$tmp/c.cnf" 2>/dev/null
  openssl pkcs12 -export -inkey "$tmp/k.pem" -in "$tmp/c.pem" -out "$tmp/c.p12" -passout pass:x -name "$CERT" -legacy 2>/dev/null
  security import "$tmp/c.p12" -k ~/Library/Keychains/login.keychain-db -P x -T /usr/bin/codesign >/dev/null
  rm -rf "$tmp"
fi

launchctl bootout "gui/$(id -u)/local.layoutfix" 2>/dev/null || true
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp bundle/Info.plist "$APP/Contents/"
cp target/release/layoutfix "$APP/Contents/MacOS/"
codesign --force --sign "$CERT" "$APP"

cat > "$AGENT" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>local.layoutfix</string>
  <key>Program</key><string>$APP/Contents/MacOS/layoutfix</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/layoutfix.log</string>
</dict></plist>
PLIST
launchctl bootstrap "gui/$(id -u)" "$AGENT"
echo "Installed. First time only: allow LayoutFix in System Settings → Privacy & Security → Accessibility and Input Monitoring; status is in ~/Library/Logs/layoutfix.log."
