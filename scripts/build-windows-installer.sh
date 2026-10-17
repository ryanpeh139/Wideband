#!/usr/bin/env bash
# Cross-builds the Windows release on Linux and wraps it in dist/Wideband-Setup.exe.
# Needs: rustup target x86_64-pc-windows-gnu, x86_64-w64-mingw32-gcc, makensis (NSIS 3).
# J2534Host.exe is compiled with mcs (-platform:x86) and checked with --check-pack when mono is
# installed; otherwise the repository's J2534Host.exe is used.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
stage="$repo/dist/installer-stage"
out="$repo/dist/Wideband-Setup.exe"
cd "$repo"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
commit="$(git rev-parse --short HEAD 2>/dev/null || echo local)"

echo "==> cargo build --release (x86_64-pc-windows-gnu)"
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  cargo build --release --locked --target x86_64-pc-windows-gnu

echo "==> stage $stage"
rm -rf "$stage"
mkdir -p "$stage"
cp target/x86_64-pc-windows-gnu/release/wideband.exe "$stage/"
if command -v mcs >/dev/null 2>&1 && command -v mono >/dev/null 2>&1; then
  mcs -nologo -warnaserror -langversion:5 -target:exe -platform:x86 -out:"$stage/J2534Host.exe" J2534Host.cs
  mono "$stage/J2534Host.exe" --check-pack
else
  cp J2534Host.exe "$stage/"
fi
cp security-config.json flash-layout.json telemetry-channels.json strategy-managers.json device_check.ps1 "$stage/"
cp scripts/Friday-CablePrep.ps1 "$stage/"
cp docs/tacoma-can-j2534-bring-up.md "$stage/Tacoma-Setup-Guide.md"
sed 's/$/\r/' installer/README-FIRST.txt > "$stage/README-FIRST.txt"
cp -r profiles definition-packs "$stage/"

echo "==> makensis"
makensis -V2 -DSTAGE="$stage" -DVERSION="$version ($commit)" -DOUTFILE="$out" installer/wideband.nsi
echo "Installer: $out"
