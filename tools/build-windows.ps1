$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --locked --release -p buaalogin-windows --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
$stage = 'build/windows-native/BUAALogin'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage, 'releases' | Out-Null
Copy-Item 'target/x86_64-pc-windows-msvc/release/buaalogin-windows.exe' "$stage/BUAALogin.exe"
Copy-Item 'README.md' "$stage/README.md"
Compress-Archive -Path 'build/windows-native/BUAALogin' -DestinationPath 'releases/BUAALogin-Windows-x64.zip' -Force
