#!/usr/bin/env bash
# Build a self-contained Document2Voice (Kokoro) AppImage you can run on most Linux PCs.
#
# Before you start: run ./setup.sh once (compiler, Rust, voice models).
#
# How to run (as your normal user — do NOT use sudo):
#   ./build-appimage.sh
#
# On success the AppImage is written under dist/. Set COPY_TO_DIR=<folder> to
# also copy it there as document2voice.AppImage (name override: COPY_NAME=...).
#
# What gets packed: the graphical app + command-line tool + voice models +
# ffmpeg + speech tools + PDF tools.
# Helper tools are kept in a project-local cache (not taken from your system PATH).
# Shared libraries are stored as real files only (no shortcuts/links — works better
# on external drives).
#
# Every downloaded tool is PINNED to an exact version and verified against a
# SHA-256 below (conda packages via the lockfiles in packaging/appimage/locks/).
# Downloads are cached under target/appimage-cache/ and models/kokoro/; a cached
# file is reused only when its checksum matches, otherwise it is fetched again.
# Installed tools under runtime-prefix/ are reused when they report the pinned version.
#
# To bump a tool: change its *_VER / *_URL and *_SHA256 together.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
cd "$REPO_ROOT"
PKG_DIR="$REPO_ROOT/packaging/appimage"

if [[ "$(id -u)" -eq 0 ]]; then
  echo "ERROR: do not run this script with sudo / as root." >&2
  echo "  Run as your normal user:  ./build-appimage.sh" >&2
  exit 1
fi

# The pinned ffmpeg, conda lockfiles and AppImage runtime are all x86_64 builds.
ARCH=x86_64
if [[ "$(uname -m)" != "$ARCH" ]]; then
  echo "ERROR: this build only supports $ARCH hosts (this machine is $(uname -m))." >&2
  exit 1
fi
VERSION="${VERSION:-$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')}"
APP_NAME="Document2Voice-Kokoro"
OUT_DIR="$(realpath -m "${OUT_DIR:-$REPO_ROOT/dist}")"
BUILD_DIR="$(realpath -m "${BUILD_DIR:-$REPO_ROOT/target/appimage}")"
APPDIR="${BUILD_DIR}/AppDir"
CACHE_DIR="$(realpath -m "${CACHE_DIR:-$REPO_ROOT/target/appimage-cache}")"
RUNTIME_PREFIX="${RUNTIME_PREFIX:-$CACHE_DIR/runtime-prefix}"
PREFIX="$RUNTIME_PREFIX"

# --- Pinned third-party downloads (version + SHA-256) -----------------------
ESPEAK_VER="1.52.0"
ESPEAK_URL="https://github.com/espeak-ng/espeak-ng/archive/refs/tags/${ESPEAK_VER}.tar.gz"
ESPEAK_SHA256="bb4338102ff3b49a81423da8a1a158b420124b055b60fa76cfb4b18677130a23"

# johnvansickle.com moves superseded builds to old-releases/, so try both.
FFMPEG_VER="7.0.2"
FFMPEG_URLS=(
  "https://johnvansickle.com/ffmpeg/releases/ffmpeg-${FFMPEG_VER}-amd64-static.tar.xz"
  "https://johnvansickle.com/ffmpeg/old-releases/ffmpeg-${FFMPEG_VER}-amd64-static.tar.xz"
)
FFMPEG_SHA256="abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67"

MICROMAMBA_VER="2.8.1"
MICROMAMBA_URL="https://github.com/mamba-org/micromamba-releases/releases/download/${MICROMAMBA_VER}-0/micromamba-linux-64.tar.bz2"
MICROMAMBA_SHA256="a934c3709c997feae403a27fd1e321c106d26ffa4f294800ffb11cbc9a3e8515"

APPIMAGETOOL_VER="1.9.1"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VER}/appimagetool-${ARCH}.AppImage"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
# appimagetool otherwise downloads the latest "continuous" runtime at pack time.
APPIMAGE_RUNTIME_VER="20251108"
APPIMAGE_RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/${APPIMAGE_RUNTIME_VER}/runtime-${ARCH}"
APPIMAGE_RUNTIME_SHA256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

# Corresponding source for the GPL programs shipped in the AppImage (put next to
# it in dist/, so the download can be shared with its source).
FFMPEG_SRC_URL="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VER}.tar.xz"
FFMPEG_SRC_SHA256="8646515b638a3ad303e23af6a3587734447cb8fc0a0c064ecdb8e95c4fd8b389"
POPPLER_SRC_URL="https://poppler.freedesktop.org/poppler-26.07.0.tar.xz"
POPPLER_SRC_SHA256="304832f48f8a47fdca90c6b6d1f684e68f37c10c9a0726f345f4ca9df4ca01e2"

# Poppler (pdftotext/pdfinfo) + libraries, and espeak-ng build tools: exact
# conda-forge package URLs with SHA-256, verified by micromamba.
POPPLER_VER="26.07.0"
RUNTIME_LOCK="$PKG_DIR/locks/runtime-linux-64.lock"
ESPEAK_BUILD_LOCK="$PKG_DIR/locks/espeak-build-linux-64.lock"

# Kokoro model URL + SHA-256 (shared with setup.sh): MODEL_BASE, MODEL_SHA256.
# shellcheck source=packaging/models.env
source "$REPO_ROOT/packaging/models.env"

export CACHE_DIR RUNTIME_PREFIX MAMBA_ROOT_PREFIX="${MAMBA_ROOT_PREFIX:-$CACHE_DIR/mamba-root}"
mkdir -p "$OUT_DIR" "$BUILD_DIR" "$CACHE_DIR" "$PREFIX/bin" "$MAMBA_ROOT_PREFIX"

die() { echo "ERROR: $*" >&2; exit 1; }

echo "==> Document2Voice (Kokoro) AppImage builder"
echo "    version=$VERSION"
echo "    runtime deps: $RUNTIME_PREFIX"

if ! command -v gcc >/dev/null 2>&1 && ! command -v cc >/dev/null 2>&1; then
  die "no C compiler on PATH. Run: ./setup.sh"
fi
if ! command -v cargo >/dev/null 2>&1; then
  die "cargo not on PATH. Run: ./setup.sh"
fi
[[ -f "$PKG_DIR/AppRun" && -f "$PKG_DIR/document2voice.desktop" ]] \
  || die "missing packaging assets under packaging/appimage/ (AppRun, .desktop)"
[[ -f "$RUNTIME_LOCK" && -f "$ESPEAK_BUILD_LOCK" ]] \
  || die "missing conda lockfiles under packaging/appimage/locks/"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum not on PATH (coreutils)"
command -v python3 >/dev/null 2>&1 || die "python3 not on PATH (needed to collect license notices). Run: ./setup.sh"
chmod +x "$PKG_DIR/AppRun"

# ---------------------------------------------------------------------------
# Helper tools (stored in the project cache — never taken from the system PATH)
# ---------------------------------------------------------------------------

ensure_soname_files() {
  local root="${1:-$PREFIX}" libdir f base sn dest created=0
  command -v readelf >/dev/null 2>&1 || return 0
  for libdir in "$root/lib" "$root/lib64"; do
    [[ -d "$libdir" ]] || continue
    for f in "$libdir"/*; do
      [[ -e "$f" ]] || continue
      [[ -f "$f" && ! -L "$f" ]] || continue
      base="$(basename "$f")"
      [[ "$base" == *.so* ]] || continue
      sn="$(readelf -d "$f" 2>/dev/null | sed -n '/SONAME/s/.*\[\([^]]*\)\].*/\1/p' | head -1)" || sn=""
      [[ -n "$sn" && "$sn" != "$base" ]] || continue
      dest="$libdir/$sn"
      [[ -L "$dest" ]] && rm -f "$dest"
      [[ -e "$dest" ]] && continue
      cp -a "$f" "$dest"
      chmod a+rx "$dest" 2>/dev/null || true
      created=$((created + 1))
    done
  done
  # Do not end with a failing test under set -e — that would stop the whole script.
  if [[ "$created" -gt 0 ]]; then
    echo "    materialised $created library SONAME file(s) under $root"
  fi
  return 0
}

densify_symlinks() {
  local root="${1:-$PREFIX}" link target count=0 broken=0
  [[ -d "$root" ]] || return 0
  local -a links=()
  while IFS= read -r -d '' link; do
    links+=("$link")
  done < <(find "$root" -type l -print0 2>/dev/null || true)
  for link in "${links[@]}"; do
    [[ -L "$link" ]] || continue
    target="$(readlink -f "$link" 2>/dev/null || true)"
    if [[ -z "$target" || ! -e "$target" ]]; then
      rm -f "$link"; broken=$((broken + 1)); continue
    fi
    rm -f "$link"
    if [[ -d "$target" ]]; then cp -a "$target" "$link"
    else cp -aL "$target" "$link" 2>/dev/null || cp -a "$target" "$link"
    fi
    count=$((count + 1))
  done
  if [[ "$count" -gt 0 || "$broken" -gt 0 ]]; then
    if [[ "$broken" -gt 0 ]]; then
      echo "    densified $count symlink(s) (removed $broken broken)"
    else
      echo "    densified $count symlink(s)"
    fi
  fi
  return 0
}

heal_prefix_libs() {
  ensure_soname_files "${1:-$PREFIX}"
  densify_symlinks "${1:-$PREFIX}"
}

tool_runs() {
  local bin="$1"; shift
  [[ -x "$bin" ]] || return 1
  local libdir
  libdir="$(cd "$(dirname "$bin")/.." && pwd)/lib"
  if [[ -d "$libdir" ]]; then
    LD_LIBRARY_PATH="$libdir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$bin" "$@" >/dev/null 2>&1
  else
    "$bin" "$@" >/dev/null 2>&1
  fi
}

sha256_of() { sha256sum "$1" | awk '{print $1}'; }

# Download a pinned file and verify its SHA-256.
#   download <out> <sha256> <url> [fallback-url ...]
# A cached file is reused only if its checksum matches; otherwise it is replaced.
# Status messages go to standard error (safe when this is called inside $()).
download() {
  local out="$1" sha="$2"; shift 2
  local name partial url got
  name="$(basename "$out")"
  if [[ -f "$out" && -s "$out" ]]; then
    got="$(sha256_of "$out")"
    if [[ "$got" == "$sha" ]]; then
      echo "    skip (cached, sha256 ok): $name ($(du -h "$out" | cut -f1))" >&2
      return 0
    fi
    echo "    cached $name has wrong sha256 ($got) — downloading again" >&2
    rm -f "$out"
  fi
  mkdir -p "$(dirname "$out")"
  partial="${out}.partial"
  for url in "$@"; do
    echo "    download $name ← $url" >&2
    rm -f "$partial"
    if command -v curl >/dev/null; then
      curl -fL --retry 3 --retry-delay 2 -o "$partial" "$url" || continue
    elif command -v wget >/dev/null; then
      wget -O "$partial" "$url" || continue
    else
      die "need curl or wget"
    fi
    [[ -s "$partial" ]] || continue
    got="$(sha256_of "$partial")"
    if [[ "$got" != "$sha" ]]; then
      rm -f "$partial"
      die "sha256 mismatch for $name from $url
      expected $sha
      got      $got
    Refusing to use it. If upstream legitimately changed, update the pinned SHA-256 in build-appimage.sh (models: packaging/models.env)."
    fi
    mv -f "$partial" "$out"
    echo "    ok $name (sha256 verified, $(du -h "$out" | cut -f1))" >&2
    return 0
  done
  rm -f "$partial"
  die "download failed: $name (tried: $*)"
}

# Succeeds when "<bin> <args>" runs and its output contains <needle>.
tool_reports() {
  local needle="$1" bin="$2"; shift 2
  [[ -x "$bin" ]] || return 1
  local libdir out
  libdir="$(cd "$(dirname "$bin")/.." && pwd)/lib"
  # Capture everything (no `| head`: with pipefail an early-closed pipe looks like failure).
  out="$(LD_LIBRARY_PATH="$libdir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$bin" "$@" 2>&1)" || return 1
  [[ "$out" == *"$needle"* ]]
}

ensure_micromamba() {
  # Only the path is printed on standard output (callers capture it with MM="$(...)").
  # Progress messages go to standard error so they do not mix into that path.
  local mm="$CACHE_DIR/bin/micromamba"
  if tool_reports "$MICROMAMBA_VER" "$mm" --version; then
    echo "    skip (already installed): micromamba $MICROMAMBA_VER" >&2
    printf '%s\n' "$mm"
    return 0
  fi
  echo "    installing micromamba $MICROMAMBA_VER from pinned archive" >&2
  mkdir -p "$CACHE_DIR/bin"
  local tarball="$CACHE_DIR/micromamba-${MICROMAMBA_VER}.tar.bz2"
  local extract="$CACHE_DIR/mamba-extract"
  download "$tarball" "$MICROMAMBA_SHA256" "$MICROMAMBA_URL"
  rm -rf "$extract"
  mkdir -p "$extract"
  tar -xjf "$tarball" -C "$extract"
  local found
  found="$(find "$extract" -type f -name micromamba | head -1)"
  [[ -n "$found" ]] || die "micromamba missing from tarball"
  install -m755 "$found" "$mm"
  # Temporary unpack folder is only needed once.
  rm -rf "$extract"
  printf '%s\n' "$mm"
}

install_static_ffmpeg() {
  local dest="$PREFIX/bin/ffmpeg"
  local tarball="$CACHE_DIR/ffmpeg-${FFMPEG_VER}-amd64-static.tar.xz" tmp ff
  # The tarball is needed later for its license files, so always make sure it is present.
  download "$tarball" "$FFMPEG_SHA256" "${FFMPEG_URLS[@]}"
  if tool_reports "ffmpeg version ${FFMPEG_VER}" "$dest" -version; then
    echo "    ffmpeg: OK ($FFMPEG_VER already installed)"; return 0
  fi
  # Explicit cleanup: a `trap … RETURN` here would stay armed for later functions
  # and trip `set -u` once $tmp is out of scope.
  tmp="$(mktemp -d)"
  tar -xJf "$tarball" -C "$tmp"
  ff="$(find "$tmp" -type f -name ffmpeg | head -1)"
  [[ -n "$ff" && -x "$ff" ]] || { rm -rf "$tmp"; die "static ffmpeg missing in tarball"; }
  mkdir -p "$(dirname "$dest")"
  install -m755 "$ff" "$dest"
  rm -rf "$tmp"
  echo "    ffmpeg: $(du -h "$dest" | awk '{print $1}') static (from cached archive)"
}

install_poppler_conda() {
  heal_prefix_libs "$PREFIX"
  if tool_reports "$POPPLER_VER" "$PREFIX/bin/pdftotext" -v && tool_runs "$PREFIX/bin/pdfinfo" -v; then
    echo "    poppler: OK ($POPPLER_VER already installed)"; return 0
  fi
  local MM; MM="$(ensure_micromamba)"
  echo "    installing poppler $POPPLER_VER from conda-forge lockfile…"
  if [[ ! -f "$PREFIX/conda-meta/history" ]]; then
    # micromamba will not create an environment in a non-empty folder that is not already one.
    # Never delete a folder outside this project's cache.
    case "$PREFIX" in
      "$CACHE_DIR"/*|"$REPO_ROOT"/target/*) ;;
      *) die "refusing to wipe RUNTIME_PREFIX outside project cache: $PREFIX" ;;
    esac
    rm -rf "$PREFIX"
    mkdir -p "$(dirname "$PREFIX")"
    "$MM" create -y -p "$PREFIX" -f "$RUNTIME_LOCK"
  else
    "$MM" install -y -p "$PREFIX" -f "$RUNTIME_LOCK"
  fi
  heal_prefix_libs "$PREFIX"
  [[ -x "$PREFIX/bin/pdftotext" && -x "$PREFIX/bin/pdfinfo" ]] || die "poppler tools missing"
  tool_runs "$PREFIX/bin/pdftotext" -v || die "pdftotext cannot run (libpoppler SONAME?)"
  tool_runs "$PREFIX/bin/pdfinfo" -v || die "pdfinfo cannot run (libpoppler SONAME?)"
  echo "    pdftotext: $("$PREFIX/bin/pdftotext" -v 2>&1 | head -1)"
}

normalize_espeak_data() {
  if [[ -d "$PREFIX/share/espeak-ng-data" ]]; then return 0; fi
  local data
  data="$(find "$PREFIX" -type d -name 'espeak-ng-data' 2>/dev/null | head -1 || true)"
  [[ -n "$data" ]] || return 1
  mkdir -p "$PREFIX/share/espeak-ng-data"
  cp -a "$data"/. "$PREFIX/share/espeak-ng-data/"
}

install_espeak_from_source() {
  local src_tarball="$CACHE_DIR/espeak-ng-${ESPEAK_VER}.tar.gz"
  local src_dir="$CACHE_DIR/espeak-ng-${ESPEAK_VER}-src"
  # Source is also needed later for its license files.
  download "$src_tarball" "$ESPEAK_SHA256" "$ESPEAK_URL"
  if [[ -x "$PREFIX/bin/espeak-ng" ]] && normalize_espeak_data; then
    if tool_reports "$ESPEAK_VER" "$PREFIX/bin/espeak-ng" --path="$PREFIX/share/espeak-ng-data" --version \
       && [[ -f "$src_dir/COPYING" ]]; then
      echo "    espeak-ng: OK ($ESPEAK_VER already installed)"; return 0
    fi
  fi
  local CC="${CC:-$(command -v gcc || command -v cc || true)}"
  [[ -n "$CC" ]] || die "need a C compiler. Run: ./setup.sh"
  export CC
  echo "    espeak build CC=$CC"

  # Reuse already-unpacked source code if present; otherwise unpack from the archive once.
  if [[ ! -f "$src_dir/configure.ac" && ! -f "$src_dir/Makefile.am" ]]; then
    echo "    unpack espeak-ng ${ESPEAK_VER} sources"
    rm -rf "$src_dir"
    mkdir -p "$src_dir"
    tar -xzf "$src_tarball" -C "$src_dir" --strip-components=1
  else
    echo "    skip (sources already unpacked): espeak-ng-${ESPEAK_VER}-src"
  fi

  local MM build_env
  MM="$(ensure_micromamba)"
  build_env="$CACHE_DIR/espeak-build-env"
  if [[ ! -x "$build_env/bin/autoreconf" ]] \
     || ! cmp -s "$ESPEAK_BUILD_LOCK" "$build_env/.document2voice-lock" 2>/dev/null; then
    echo "    installing espeak build tools (autoconf/automake) from lockfile…"
    rm -rf "$build_env"
    "$MM" create -y -p "$build_env" -f "$ESPEAK_BUILD_LOCK"
    cp -f "$ESPEAK_BUILD_LOCK" "$build_env/.document2voice-lock"
  else
    echo "    skip (already installed): espeak build tools"
  fi

  echo "    building espeak-ng ${ESPEAK_VER} → $PREFIX"
  # Keep build tools on PATH only inside this subshell so they do not break the later
  # cargo build (which needs the normal system SSL libraries).
  (
    export PATH="$build_env/bin:$PATH"
    cd "$src_dir"
    if [[ ! -f ./configure ]]; then
      if [[ -f ./autogen.sh ]]; then bash ./autogen.sh; else autoreconf -fi; fi
    fi
    # --with-sonic=no: the system may have sonic headers but the conda linker cannot find the library
    ./configure --prefix="$PREFIX" --disable-rpath \
      --without-pcaudiolib --without-speechplayer --without-klatt --without-mbrola \
      --with-sonic=no \
      || ./configure --prefix="$PREFIX" --disable-rpath --with-sonic=no
    make -j"$(nproc 2>/dev/null || echo 4)"
    make install
  )
  normalize_espeak_data || die "espeak-ng-data missing after build"
  [[ -x "$PREFIX/bin/espeak-ng" ]] || die "espeak-ng binary missing after build"
  echo "    espeak-ng: $("$PREFIX/bin/espeak-ng" --path="$PREFIX/share/espeak-ng-data" --version 2>&1 | head -1)"
}

echo "==> preparing runtime tools (project cache)"
heal_prefix_libs "$PREFIX"
# Each installer is a no-op when the pinned version is already present.
install_poppler_conda
install_static_ffmpeg
install_espeak_from_source
heal_prefix_libs "$PREFIX"
for b in ffmpeg espeak-ng pdftotext pdfinfo; do
  [[ -x "$PREFIX/bin/$b" ]] || die "$PREFIX/bin/$b missing"
done
tool_runs "$PREFIX/bin/pdftotext" -v || die "pdftotext cannot run"
tool_runs "$PREFIX/bin/pdfinfo" -v || die "pdfinfo cannot run"
[[ -d "$PREFIX/share/espeak-ng-data" ]] || die "espeak-ng-data missing"
{
  echo "ffmpeg=static-johnvansickle-${FFMPEG_VER} sha256=${FFMPEG_SHA256}"
  echo "poppler=conda-forge-${POPPLER_VER} lock=$(sha256_of "$RUNTIME_LOCK")"
  echo "espeak-ng=source-${ESPEAK_VER} sha256=${ESPEAK_SHA256}"
} >"$PREFIX/.document2voice-deps-stamp"
echo "$PREFIX" >"$CACHE_DIR/runtime-prefix.path"

# ---------------------------------------------------------------------------
# Voice models (reuse models/kokoro if already present — no re-download)
# ---------------------------------------------------------------------------
MODEL_DIR="${DOCUMENT2VOICE_MODEL_DIR:-$REPO_ROOT/models/kokoro}"
mkdir -p "$MODEL_DIR"
echo "==> models → $MODEL_DIR"
for f in kokoro-v1.0.onnx voices-v1.0.bin; do
  download "$MODEL_DIR/$f" "${MODEL_SHA256[$f]}" "$MODEL_BASE/$f"
done
[[ -f "$MODEL_DIR/kokoro-v1.0.onnx" && -s "$MODEL_DIR/kokoro-v1.0.onnx" \
   && -f "$MODEL_DIR/voices-v1.0.bin" && -s "$MODEL_DIR/voices-v1.0.bin" ]] \
  || die "models missing — run ./setup.sh"

# ---------------------------------------------------------------------------
# Build the release apps with cargo
# ---------------------------------------------------------------------------
echo "==> cargo build --release (gui + cli)"
cargo build --release -p document2voice-gui -p document2voice-cli
GUI_BIN="$REPO_ROOT/target/release/document2voice-gui"
CLI_BIN="$REPO_ROOT/target/release/document2voice"
[[ -x "$GUI_BIN" && -x "$CLI_BIN" ]] || die "release binaries missing"

# ---------------------------------------------------------------------------
# Assemble the AppDir (folder layout that becomes the AppImage)
# ---------------------------------------------------------------------------
echo "==> preparing AppDir"
rm -rf "$APPDIR"
mkdir -p \
  "$APPDIR/usr/bin" \
  "$APPDIR/usr/lib" \
  "$APPDIR/usr/share/applications" \
  "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
  "$APPDIR/usr/share/document2voice/models/kokoro" \
  "$APPDIR/usr/share/espeak-ng-data" \
  "$APPDIR/usr/share/metainfo"

install -m755 "$GUI_BIN" "$APPDIR/usr/bin/document2voice-gui"
install -m755 "$CLI_BIN" "$APPDIR/usr/bin/document2voice"
cp -a "$MODEL_DIR/." "$APPDIR/usr/share/document2voice/models/kokoro/"
install -m755 "$PREFIX/bin/ffmpeg" "$APPDIR/usr/bin/ffmpeg"
install -m755 "$PREFIX/bin/espeak-ng" "$APPDIR/usr/bin/espeak-ng"
install -m755 "$PREFIX/bin/pdftotext" "$APPDIR/usr/bin/pdftotext"
install -m755 "$PREFIX/bin/pdfinfo" "$APPDIR/usr/bin/pdfinfo"
cp -a "$PREFIX/share/espeak-ng-data"/. "$APPDIR/usr/share/espeak-ng-data/"

for libdir in lib lib64; do
  if [[ -d "$PREFIX/$libdir" ]]; then
    find "$PREFIX/$libdir" -maxdepth 1 \( -name '*.so' -o -name '*.so.*' \) \
      -exec cp -aL {} "$APPDIR/usr/lib/" \;
  fi
done
densify_symlinks "$APPDIR/usr/lib"
ensure_soname_files "$APPDIR/usr"

ICON_SRC="$REPO_ROOT/crates/gui/assets/app-icon.png"
if [[ -f "$ICON_SRC" ]]; then
  install -m644 "$ICON_SRC" "$APPDIR/usr/share/icons/hicolor/256x256/apps/document2voice.png"
  install -m644 "$ICON_SRC" "$APPDIR/document2voice.png"
  install -m644 "$ICON_SRC" "$APPDIR/.DirIcon"
fi
install -m644 "$PKG_DIR/document2voice.desktop" \
  "$APPDIR/usr/share/applications/document2voice.desktop"
install -m644 "$PKG_DIR/document2voice.desktop" "$APPDIR/document2voice.desktop"
install -m755 "$PKG_DIR/AppRun" "$APPDIR/AppRun"

# ---------------------------------------------------------------------------
# Collect shared libraries (real files only, no shortcuts/links)
# ---------------------------------------------------------------------------
echo "==> collecting shared libraries"
LIBDIR="$APPDIR/usr/lib"
mkdir -p "$LIBDIR"

should_skip() {
  local base="$1"
  case "$base" in
    linux-vdso.so*|ld-linux*|ld-linux-x86-64.so*|ld.so*) return 0 ;;
    libc.so*|libm.so*|libdl.so*|librt.so*|libpthread.so*|libresolv.so*|libutil.so*|libcrypt.so*) return 0 ;;
    libGL.so*|libGLX.so*|libGLdispatch.so*|libOpenGL.so*|libEGL.so*|libGLES*) return 0 ;;
    libvulkan.so*|libdrm.so*|libgbm.so*|libglapi.so*) return 0 ;;
    libX11.so*|libXext.so*|libXrandr.so*|libXcursor.so*|libXrender.so*|libXfixes.so*|libXi.so*|libXxf86vm.so*|libXinerama.so*|libXss.so*|libXau.so*|libXdmcp.so*|libxcb*) return 0 ;;
    libwayland*|libxkbcommon*) return 0 ;;
    libpulse*.so*|libpipewire*.so*|libasound.so*|libjack.so*|libsndio.so*) return 0 ;;
    libnvidia*|libcuda.so*|libcudadebugger.so*|libnvcuvid.so*|libvdpau_nvidia.so*) return 0 ;;
    libnss_*.so*|libsystemd.so*|libudev.so*) return 0 ;;
  esac
  return 1
}

install_lib_file() {
  local src="$1" dest="$2"
  [[ -n "$src" && -e "$src" ]] || return 0
  [[ -L "$dest" ]] && rm -f "$dest"
  [[ -e "$dest" ]] && return 0
  cp -aL "$src" "$dest" 2>/dev/null || cp -a "$src" "$dest"
  chmod a+rx "$dest" 2>/dev/null || true
}

elf_soname() {
  readelf -d "$1" 2>/dev/null | sed -n '/SONAME/s/.*\[\([^]]*\)\].*/\1/p' | head -1 || true
}

declare -A SEEN=()
QUEUE=()
enqueue() {
  local f="$1" real
  [[ -n "$f" && -e "$f" ]] || return 0
  real="$(readlink -f "$f" 2>/dev/null || echo "$f")"
  [[ -n "${SEEN[$real]+x}" ]] && return 0
  SEEN[$real]=1
  QUEUE+=("$real")
}

for bin in \
  "$APPDIR/usr/bin/document2voice-gui" \
  "$APPDIR/usr/bin/document2voice" \
  "$APPDIR/usr/bin/espeak-ng" \
  "$APPDIR/usr/bin/pdftotext" \
  "$APPDIR/usr/bin/pdfinfo" \
  "$APPDIR/usr/bin/ffmpeg"
do
  enqueue "$bin"
done

i=0
while [[ $i -lt ${#QUEUE[@]} ]]; do
  cur="${QUEUE[$i]}"; i=$((i + 1))
  base="$(basename "$cur")"
  if [[ "$base" == *.so* ]] && ! should_skip "$base"; then
    realf="$(readlink -f "$cur" 2>/dev/null || echo "$cur")"
    realbase="$(basename "$realf")"
    install_lib_file "$realf" "${LIBDIR}/${realbase}"
    [[ "$base" != "$realbase" ]] && install_lib_file "$realf" "${LIBDIR}/${base}"
    if command -v readelf >/dev/null 2>&1; then
      sn="$(elf_soname "$realf")"
      if [[ -n "$sn" && "$sn" != "$realbase" ]]; then
        install_lib_file "$realf" "${LIBDIR}/${sn}"
      fi
    fi
  fi
  while IFS= read -r line; do
    [[ "$line" == *'=>'* ]] || continue
    path="$(echo "$line" | awk '{print $3}')"
    [[ "$path" == /* ]] || continue
    basep="$(basename "$path")"
    should_skip "$basep" && continue
    enqueue "$path"
  done < <(ldd "$cur" 2>/dev/null || true)
done

densify_symlinks "$LIBDIR"
ensure_soname_files "$APPDIR/usr"
echo "    ${#SEEN[@]} objects scanned → $(find "$LIBDIR" -type f | wc -l) files (no symlinks)"

# ---------------------------------------------------------------------------
# License texts + third-party notice for everything bundled
# ---------------------------------------------------------------------------
echo "==> collecting licenses"
python3 "$PKG_DIR/collect-licenses.py" \
  --repo "$REPO_ROOT" \
  --appdir "$APPDIR" \
  --prefix "$PREFIX" \
  --ffmpeg-tarball "$CACHE_DIR/ffmpeg-${FFMPEG_VER}-amd64-static.tar.xz" \
  --espeak-src "$CACHE_DIR/espeak-ng-${ESPEAK_VER}-src" \
  --component "ffmpeg|${FFMPEG_VER} (static)|GPL-3.0-or-later|${FFMPEG_URLS[0]}|${FFMPEG_SHA256}" \
  --component "espeak-ng|${ESPEAK_VER}|GPL-3.0-or-later|${ESPEAK_URL}|${ESPEAK_SHA256}" \
  || die "license collection failed"

# project_license describes the whole bundle: the GPL app plus the programs and libraries it ships.
cat > "$APPDIR/usr/share/metainfo/io.github.bsd_source.Document2Voice.metainfo.xml" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>io.github.bsd_source.Document2Voice</id>
  <name>Document2Voice (Kokoro)</name>
  <summary>Offline document-to-audiobook with Kokoro ONNX</summary>
  <metadata_license>MIT</metadata_license>
  <project_license>GPL-3.0-or-later AND GPL-2.0-or-later AND MIT AND Apache-2.0</project_license>
  <description>
    <p>Turn PDF, EPUB, TXT, DOCX, ODT and more into spoken audio offline using Kokoro-82M.</p>
    <p>Bundles ffmpeg, espeak-ng and Poppler under their own GPL licenses; see
    usr/share/licenses/document2voice/THIRD-PARTY-NOTICES.md inside the AppImage.</p>
  </description>
  <launchable type="desktop-id">document2voice.desktop</launchable>
  <categories>
    <category>AudioVideo</category>
  </categories>
  <developer id="io.github.bsd_source">
    <name>document2voice contributors</name>
  </developer>
  <content_rating type="oars-1.1" />
</component>
EOF

# ---------------------------------------------------------------------------
# Pack into the final AppImage file
# ---------------------------------------------------------------------------
echo "==> appimagetool $APPIMAGETOOL_VER + runtime $APPIMAGE_RUNTIME_VER (pinned)"
APPIMAGETOOL="$CACHE_DIR/appimagetool-${APPIMAGETOOL_VER}-${ARCH}.AppImage"
APPIMAGE_RUNTIME="$CACHE_DIR/appimage-runtime-${APPIMAGE_RUNTIME_VER}-${ARCH}"
download "$APPIMAGETOOL" "$APPIMAGETOOL_SHA256" "$APPIMAGETOOL_URL"
download "$APPIMAGE_RUNTIME" "$APPIMAGE_RUNTIME_SHA256" "$APPIMAGE_RUNTIME_URL"
chmod +x "$APPIMAGETOOL"

OUT_FILE="${OUT_DIR}/${APP_NAME}-${VERSION}-${ARCH}.AppImage"
echo "==> packing $OUT_FILE"
rm -f "$OUT_FILE"
export ARCH APPIMAGE_EXTRACT_AND_RUN=1
"$APPIMAGETOOL" --no-appstream --runtime-file "$APPIMAGE_RUNTIME" "$APPDIR" "$OUT_FILE"
chmod +x "$OUT_FILE"

echo "==> smoke"
SMOKE_DIR="${BUILD_DIR}/smoke-extract"
rm -rf "$SMOKE_DIR"; mkdir -p "$SMOKE_DIR"
(
  cd "$SMOKE_DIR"
  APPIMAGE_EXTRACT_AND_RUN=1 "$OUT_FILE" --appimage-extract >/dev/null 2>&1
) || die "smoke test: could not extract $OUT_FILE"
[[ -d "$SMOKE_DIR/squashfs-root" ]] || die "smoke test: extract produced no squashfs-root"
R="$SMOKE_DIR/squashfs-root"
echo "    ffmpeg: $(env PATH="$R/usr/bin" LD_LIBRARY_PATH="$R/usr/lib" \
  "$R/usr/bin/ffmpeg" -version 2>&1 | head -1)"
echo "    espeak: $(env PATH="$R/usr/bin" LD_LIBRARY_PATH="$R/usr/lib" \
  "$R/usr/bin/espeak-ng" --path="$R/usr/share/espeak-ng-data" --version 2>&1 | head -1)"
echo "    pdftotext: $(env PATH="$R/usr/bin" LD_LIBRARY_PATH="$R/usr/lib" \
  "$R/usr/bin/pdftotext" -v 2>&1 | head -1)"
[[ -f "$R/usr/share/document2voice/models/kokoro/kokoro-v1.0.onnx" ]] || die "models missing inside AppImage"
echo "    models: OK"
[[ -f "$R/usr/share/licenses/document2voice/THIRD-PARTY-NOTICES.md" ]] || die "license notices missing inside AppImage"
echo "    licenses: OK"
# Remove the temporary extract folder after the smoke test (it is large; the AppImage is the real product).
rm -rf "$SMOKE_DIR"

SIZE="$(du -h "$OUT_FILE" | awk '{print $1}')"
echo
echo "✓ Built: $OUT_FILE ($SIZE)"

# ---------------------------------------------------------------------------
# GPL corresponding source, next to the AppImage
# ---------------------------------------------------------------------------
echo "==> GPL sources"
SRC_DIR="${OUT_DIR}/${APP_NAME}-${VERSION}-sources"
mkdir -p "$SRC_DIR"
download "$CACHE_DIR/ffmpeg-${FFMPEG_VER}-src.tar.xz" "$FFMPEG_SRC_SHA256" "$FFMPEG_SRC_URL"
download "$CACHE_DIR/poppler-${POPPLER_VER}-src.tar.xz" "$POPPLER_SRC_SHA256" "$POPPLER_SRC_URL"
cp -f "$CACHE_DIR/espeak-ng-${ESPEAK_VER}.tar.gz" "$SRC_DIR/espeak-ng-${ESPEAK_VER}.tar.gz"
cp -f "$CACHE_DIR/ffmpeg-${FFMPEG_VER}-src.tar.xz" "$SRC_DIR/ffmpeg-${FFMPEG_VER}.tar.xz"
cp -f "$CACHE_DIR/poppler-${POPPLER_VER}-src.tar.xz" "$SRC_DIR/poppler-${POPPLER_VER}.tar.xz"
cp -f "$RUNTIME_LOCK" "$SRC_DIR/poppler-libraries-conda.lock"
cat > "$SRC_DIR/SOURCES.md" <<SOURCES_EOF
# Source code for the GPL programs in ${APP_NAME}-${VERSION}-${ARCH}.AppImage

Document2Voice itself is GPL-3.0-or-later. The AppImage also contains these GPL programs; their
source is in this folder (SHA-256 checked when built):

| Program | Version | License | Source here | Upstream |
|---|---|---|---|---|
| espeak-ng | ${ESPEAK_VER} | GPL-3.0-or-later | espeak-ng-${ESPEAK_VER}.tar.gz | ${ESPEAK_URL} |
| FFmpeg (static build) | ${FFMPEG_VER} | GPL-3.0-or-later | ffmpeg-${FFMPEG_VER}.tar.xz | ${FFMPEG_SRC_URL} |
| Poppler (pdftotext, pdfinfo) | ${POPPLER_VER} | GPL-2.0-or-later | poppler-${POPPLER_VER}.tar.xz | ${POPPLER_SRC_URL} |

- espeak-ng is built from that tarball by build-appimage.sh (configure flags in the script).
- FFmpeg is John Van Sickle's static ${FFMPEG_VER} build (${FFMPEG_URLS[0]},
  sha256 ${FFMPEG_SHA256}). Its readme.txt (in the AppImage under
  usr/share/licenses/document2voice/ffmpeg/) lists the libraries linked in; their build
  recipes are published at https://johnvansickle.com/ffmpeg/.
- Poppler and the libraries it uses are conda-forge packages; the exact package
  list with SHA-256 is poppler-libraries-conda.lock, and each package's recipe and
  sources are at https://github.com/conda-forge/<name>-feedstock.

Full license texts for every bundled component: usr/share/licenses/document2voice/ in
the AppImage (THIRD-PARTY-NOTICES.md).
SOURCES_EOF
echo "    sources: $SRC_DIR ($(du -sh "$SRC_DIR" | cut -f1))"

# Optional extra copy, e.g. COPY_TO_DIR="$HOME/Desktop".
if [[ -n "${COPY_TO_DIR:-}" ]]; then
  [[ -d "$COPY_TO_DIR" ]] || die "COPY_TO_DIR is not a directory: $COPY_TO_DIR"
  EXTRA_COPY="$COPY_TO_DIR/${COPY_NAME:-document2voice.AppImage}"
  cp -f "$OUT_FILE" "$EXTRA_COPY"
  chmod +x "$EXTRA_COPY"
  echo "✓ Copied to $EXTRA_COPY"
fi

echo
echo "  $OUT_FILE                 # GUI"
echo "  $OUT_FILE cli voices      # CLI"
if [[ -n "${EXTRA_COPY:-}" ]]; then
  echo "  $EXTRA_COPY            # extra copy"
fi
echo "  APPIMAGE_EXTRACT_AND_RUN=1 $OUT_FILE"
echo
echo "  Cache (kept for re-use, not re-downloaded if present):"
echo "    $CACHE_DIR"
