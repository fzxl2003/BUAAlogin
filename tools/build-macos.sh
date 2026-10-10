#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] || { echo 'Mac ARM 版须在 Apple Silicon Mac 上构建' >&2; exit 1; }
export MACOSX_DEPLOYMENT_TARGET=12.0
cargo build --locked --release -p buaa-core
app=build/macos-native/BUAALogin.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" releases
cp platform/macos/Info.plist "$app/Contents/Info.plist"
swiftc -target arm64-apple-macos12 -O -import-objc-header platform/macos/BUAACore.h \
  platform/macos/main.swift platform/macos/WiFi.swift platform/macos/AutoStart.swift target/release/libbuaa_core.a \
  -framework AppKit -framework ServiceManagement -framework Security -framework SystemConfiguration -framework CoreWLAN -framework CoreLocation -lcurl \
  -o "$app/Contents/MacOS/BUAALogin"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
rm -f releases/BUAALogin-Mac-arm64.zip
ditto -c -k --sequesterRsrc --keepParent "$app" releases/BUAALogin-Mac-arm64.zip
