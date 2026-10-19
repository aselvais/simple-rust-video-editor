#!/usr/bin/env bash
#
# Build QuickEdit for the *current* platform (Linux or macOS) and copy the
# release binary into ./builds/ with a platform+arch-stamped name, e.g.
#
#   builds/quickedit-darwin-arm64     (Apple Silicon)
#   builds/quickedit-darwin-x86_64    (Intel macOS)
#   builds/quickedit-linux-x86_64
#
# FFmpeg is a *runtime* dependency (the editor shells out to it), not a build
# dependency, so a missing FFmpeg does not stop the build — it only warns.
#
set -euo pipefail

# Always run from the script's directory (the project root).
cd "$(dirname "$0")"

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "warning: 'ffmpeg' is not on PATH — the editor will build, but it won't be able to" >&2
  echo "         probe/preview/export until FFmpeg is installed. See README.md." >&2
fi

echo "→ cargo build --release"
cargo build --release

mkdir -p builds

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
out="builds/quickedit-${os}-${arch}"

cp "target/release/quickedit" "$out"
chmod +x "$out"
echo "Built: ${out}"
echo "Run:   ./${out}"
