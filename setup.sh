#!/usr/bin/env bash
# Install everything needed to build and run Document2Voice (Kokoro) from source:
#   system packages (compiler, curl, git, speech tools, ffmpeg, PDF tools)
#   the Rust programming toolchain (installs rustup if needed)
#   Kokoro voice model files (~340 MB; skipped if you already have them)
#
# How to run (as your normal user — do NOT use sudo):
#   ./setup.sh
#
# Force a fresh download of the voice models even if they are already present:
#   FORCE_MODELS=1 ./setup.sh
#
# After this finishes, build a portable AppImage with:
#   ./build-appimage.sh

set -euo pipefail

ROOT="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
cd "$ROOT"

if [[ "$(id -u)" -eq 0 ]]; then
  echo "ERROR: do not run this script with sudo / as root." >&2
  echo "  Run as your normal user:  ./setup.sh" >&2
  echo "  (sudo is used only when installing system packages.)" >&2
  exit 1
fi

have_cmd() { command -v "$1" >/dev/null 2>&1; }

echo "==> Document2Voice (Kokoro) — setup"
echo "    packages + Rust + models"

# --- find which package manager this system uses ---
PM=""
if have_cmd pacman; then
  PM=pacman
elif have_cmd apt-get; then
  PM=apt
elif have_cmd dnf; then
  PM=dnf
elif have_cmd zypper; then
  PM=zypper
elif have_cmd apk; then
  PM=apk
else
  echo "ERROR: no supported package manager (pacman/apt/dnf/zypper/apk)." >&2
  exit 1
fi
echo "    package manager: $PM"

run_root() {
  if have_cmd sudo; then
    sudo "$@"
  else
    echo "ERROR: need sudo to install packages: $*" >&2
    exit 1
  fi
}

pkg_installed() {
  local pkg="$1"
  case "$PM" in
    pacman) pacman -Q "$pkg" &>/dev/null ;;
    apt) dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "install ok installed" ;;
    dnf|zypper) rpm -q "$pkg" &>/dev/null ;;
    apk) apk info -e "$pkg" &>/dev/null ;;
    *) return 1 ;;
  esac
}

case "$PM" in
  pacman)
    pkgs=(base-devel curl git python espeak-ng ffmpeg poppler)
    ;;
  apt)
    pkgs=(build-essential curl git python3 pkg-config libssl-dev espeak-ng ffmpeg poppler-utils)
    ;;
  dnf)
    pkgs=(gcc gcc-c++ make curl git python3 pkgconf-pkg-config openssl-devel espeak-ng ffmpeg poppler-utils)
    ;;
  zypper)
    pkgs=(gcc gcc-c++ make curl git python3 pkg-config libopenssl-devel espeak-ng ffmpeg poppler-tools)
    ;;
  apk)
    pkgs=(build-base curl git python3 pkgconf openssl-dev espeak-ng ffmpeg poppler-utils)
    ;;
esac

TO_INSTALL=()
echo "==> checking packages"
for pkg in "${pkgs[@]}"; do
  if pkg_installed "$pkg"; then
    echo "    ok: $pkg"
  else
    echo "    need: $pkg"
    TO_INSTALL+=("$pkg")
  fi
done

if [[ ${#TO_INSTALL[@]} -gt 0 ]]; then
  echo "==> installing: ${TO_INSTALL[*]}"
  case "$PM" in
    pacman) run_root pacman -S --needed "${TO_INSTALL[@]}" ;;
    apt)
      run_root apt-get update
      run_root apt-get install "${TO_INSTALL[@]}"
      ;;
    dnf) run_root dnf install "${TO_INSTALL[@]}" ;;
    zypper) run_root zypper install "${TO_INSTALL[@]}" ;;
    apk) run_root apk add --no-cache "${TO_INSTALL[@]}" ;;
  esac
else
  echo "==> all packages already installed"
fi

# --- Rust programming language ---
echo "==> checking Rust"
if have_cmd cargo && have_cmd rustc; then
  echo "    ok: $(cargo --version 2>/dev/null | head -1)"
  echo "    ok: $(rustc --version 2>/dev/null | head -1)"
  if have_cmd rustup && ! rustup show active-toolchain &>/dev/null; then
    echo "==> setting rustup default toolchain: stable"
    rustup default stable
  fi
elif have_cmd rustup; then
  echo "==> installing stable toolchain via rustup"
  rustup default stable
else
  if ! have_cmd curl; then
    echo "ERROR: need curl to install rustup." >&2
    exit 1
  fi
  echo "==> installing rustup (user install)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  [[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
  have_cmd cargo || {
    echo "ERROR: cargo not on PATH. Add \$HOME/.cargo/bin to PATH and re-run." >&2
    exit 1
  }
  echo "    ok: $(cargo --version)"
fi

# Make sure the cargo command is available in this shell if rustup just installed it.
if ! have_cmd cargo && [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

# --- voice model files ---
# Keep existing model files unless FORCE_MODELS=1 (then re-download and replace them).
DEST="${DOCUMENT2VOICE_MODEL_DIR:-$ROOT/models/kokoro}"
FORCE_MODELS="${FORCE_MODELS:-0}"
mkdir -p "$DEST"
echo "==> Kokoro models → $DEST"
[[ "$FORCE_MODELS" == "1" ]] && echo "    FORCE_MODELS=1 — re-downloading"

# Model URL + pinned SHA-256 (shared with build-appimage.sh): MODEL_BASE, MODEL_SHA256.
# shellcheck source=packaging/models.env
source "$ROOT/packaging/models.env"

fetch_model() {
  local url="$1" file="$2" want got
  want="${MODEL_SHA256[$file]}"
  if [[ "$FORCE_MODELS" != "1" && -f "$DEST/$file" && -s "$DEST/$file" ]]; then
    got="$(sha256sum "$DEST/$file" | awk '{print $1}')"
    if [[ "$got" == "$want" ]]; then
      echo "    skip (exists, sha256 ok): $file ($(du -h "$DEST/$file" | cut -f1))"
      return
    fi
    echo "    $file has wrong sha256 — downloading again"
  fi
  rm -f "$DEST/$file" "$DEST/$file.partial"
  echo "    get $file …"
  curl -fL --retry 5 --retry-delay 2 -o "$DEST/$file.partial" "$url"
  got="$(sha256sum "$DEST/$file.partial" | awk '{print $1}')"
  if [[ "$got" != "$want" ]]; then
    rm -f "$DEST/$file.partial"
    echo "ERROR: sha256 mismatch for $file (expected $want, got $got)" >&2
    exit 1
  fi
  mv "$DEST/$file.partial" "$DEST/$file"
  echo "    ok $(du -h "$DEST/$file" | cut -f1) (sha256 verified)"
}

fetch_model "$MODEL_BASE/kokoro-v1.0.onnx" "kokoro-v1.0.onnx"
fetch_model "$MODEL_BASE/voices-v1.0.bin" "voices-v1.0.bin"

# --- final check ---
echo
echo "==> environment check"
for c in cargo rustc curl git gcc make python3 sha256sum; do
  have_cmd "$c" && echo "    ok: $c" || echo "    missing: $c"
done
for c in espeak-ng ffmpeg pdftotext pdfinfo; do
  have_cmd "$c" && echo "    ok: $c" || echo "    missing: $c"
done
[[ -f "$DEST/kokoro-v1.0.onnx" && -f "$DEST/voices-v1.0.bin" ]] \
  && echo "    ok: models" || echo "    missing: models"

echo
echo "✓ Setup complete."
echo
echo "  cargo run -p document2voice-gui"
echo "  cargo build --release -p document2voice-gui -p document2voice-cli"
echo "  ./build-appimage.sh"
echo "  FORCE_MODELS=1 ./setup.sh   # re-download models"
