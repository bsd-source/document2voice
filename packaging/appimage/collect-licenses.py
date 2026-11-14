#!/usr/bin/env python3
"""Collect license texts and a third-party notice for everything the AppImage ships.

Called by build-appimage.sh after the AppDir is assembled. Writes:

  <appdir>/usr/share/licenses/document2voice/
    THIRD-PARTY-NOTICES.md      overview: component, version, license, source, sha256
    document2voice/LICENSE           this project (GPL-3.0-or-later)
    rust/<crate>-<ver>/...      LICENSE*/COPYING*/NOTICE* of every linked Rust crate
    conda/<pkg>-<ver>/...       info/licenses of every conda package in the runtime prefix
    ffmpeg/, espeak-ng/         upstream license files
    kokoro/                     Apache-2.0 text for the Kokoro-82M model + voices
    host/<pkg>/                 license files of host packages that own copied libraries

Fails (exit 1) if a required license source is missing, so a release can't
silently ship without notices.
"""

import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys

LICENSE_RE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE|COPYRIGHT|AUTHORS)", re.I)


def die(msg):
    print(f"ERROR (licenses): {msg}", file=sys.stderr)
    sys.exit(1)


def copy_license_files(src_dir, dest_dir):
    """Copy top-level license-like files from src_dir. Returns the list copied."""
    found = []
    if not os.path.isdir(src_dir):
        return found
    for name in sorted(os.listdir(src_dir)):
        path = os.path.join(src_dir, name)
        if os.path.isfile(path) and LICENSE_RE.match(name):
            os.makedirs(dest_dir, exist_ok=True)
            shutil.copy2(path, os.path.join(dest_dir, name))
            found.append(name)
    return found


def rust_crates(repo):
    """(name, version, license, manifest_dir) for crates linked into the shipped binaries."""
    tree = subprocess.run(
        [
            "cargo", "tree", "--locked",
            "-p", "document2voice-gui", "-p", "document2voice-cli",
            "-e", "normal", "--target", "x86_64-unknown-linux-gnu",
            "--prefix", "none", "-f", "{p}",
        ],
        cwd=repo, check=True, capture_output=True, text=True,
    ).stdout
    used = set()
    for line in tree.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            used.add((parts[0], parts[1][1:]))
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1",
             "--filter-platform", "x86_64-unknown-linux-gnu"],
            cwd=repo, check=True, capture_output=True, text=True,
        ).stdout
    )
    out = []
    for pkg in meta["packages"]:
        key = (pkg["name"], pkg["version"])
        if key in used:
            out.append((pkg["name"], pkg["version"], pkg.get("license") or pkg.get("license_file") or "UNKNOWN",
                        os.path.dirname(pkg["manifest_path"])))
    return sorted(out)


def host_lib_paths():
    """{soname: path} for x86-64 libraries in the build host's linker cache."""
    try:
        out = subprocess.run(["ldconfig", "-p"], check=True, capture_output=True, text=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return {}
    paths = {}
    for line in out.splitlines():
        parts = line.strip().split(" => ")
        if len(parts) == 2 and "x86-64" in parts[0]:
            paths.setdefault(parts[0].split()[0], parts[1])
    return paths


def owning_package(path):
    """Name of the system package that owns `path` (pacman or dpkg), or None."""
    if shutil.which("pacman"):
        cmd, parse = ["pacman", "-Qqo", path], lambda o: o.split()[0]
    elif shutil.which("dpkg"):
        cmd, parse = ["dpkg", "-S", path], lambda o: o.split(":")[0]
    else:
        return None
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0 or not r.stdout.strip():
        return None
    return parse(r.stdout.strip().splitlines()[0])


def host_lib_packages(libs, dest_root):
    """Map each host library to its owning package and copy that package's license files
    (/usr/share/licenses/<pkg>, or the Debian /usr/share/doc/<pkg>/copyright)."""
    paths = host_lib_paths()
    owners = {}
    for name in libs:
        path = paths.get(name)
        pkg = owning_package(os.path.realpath(path)) if path else None
        if not pkg:
            continue
        owners[name] = pkg
        dest = os.path.join(dest_root, pkg)
        lic_dir = os.path.join("/usr/share/licenses", pkg)
        deb_copyright = os.path.join("/usr/share/doc", pkg, "copyright")
        if os.path.isdir(lic_dir):
            shutil.copytree(lic_dir, dest, dirs_exist_ok=True)
        elif os.path.isfile(deb_copyright):
            os.makedirs(dest, exist_ok=True)
            shutil.copy2(deb_copyright, os.path.join(dest, "copyright"))
    return owners


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--appdir", required=True)
    ap.add_argument("--prefix", required=True, help="conda runtime prefix (poppler + deps, espeak-ng)")
    ap.add_argument("--ffmpeg-tarball", required=True)
    ap.add_argument("--espeak-src", required=True)
    ap.add_argument("--component", action="append", default=[],
                    help="name|version|license|source url|sha256 (repeatable)")
    a = ap.parse_args()

    root = os.path.join(a.appdir, "usr/share/licenses/document2voice")
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    rows = []  # (component, version, license, where, source)

    # --- this project
    if not copy_license_files(a.repo, os.path.join(root, "document2voice")):
        die("project LICENSE missing")

    # --- bundled programs (pinned downloads)
    for spec in a.component:
        name, ver, lic, url, sha = spec.split("|")
        rows.append((name, ver, lic, f"licenses/document2voice/{name}/", f"{url} (sha256 {sha})"))

    # ffmpeg static build: GPLv3 text + build readme (lists the linked libraries)
    ffdir = os.path.join(root, "ffmpeg")
    os.makedirs(ffdir)
    members = subprocess.run(["tar", "-tJf", a.ffmpeg_tarball], check=True,
                             capture_output=True, text=True).stdout.split()
    wanted = [m for m in members if m.endswith(("/GPLv3.txt", "/readme.txt"))]
    if not any(m.endswith("GPLv3.txt") for m in wanted):
        die("GPLv3.txt not found in ffmpeg tarball")
    subprocess.run(["tar", "-xJf", a.ffmpeg_tarball, "-C", ffdir, "--strip-components=1", *wanted], check=True)

    # espeak-ng source tree
    if not copy_license_files(a.espeak_src, os.path.join(root, "espeak-ng")):
        die(f"no COPYING files in {a.espeak_src}")

    # --- conda packages (poppler and its shared-library deps, all copied into usr/lib)
    metas = sorted(glob.glob(os.path.join(a.prefix, "conda-meta", "*.json")))
    if not metas:
        die(f"no conda-meta in {a.prefix}")
    for m in metas:
        with open(m) as fh:
            j = json.load(fh)
        name, ver, lic = j["name"], j["version"], j.get("license", "UNKNOWN")
        dest = os.path.join(root, "conda", f"{name}-{ver}")
        src = os.path.join(j.get("extracted_package_dir", ""), "info", "licenses")
        if os.path.isdir(src):
            shutil.copytree(src, dest, dirs_exist_ok=True)
        else:
            os.makedirs(dest, exist_ok=True)
            with open(os.path.join(dest, "LICENSE-NOT-SHIPPED-BY-PACKAGE.txt"), "w") as f:
                f.write(f"conda-forge package {name} {ver} declares license: {lic}\n"
                        f"Package: {j.get('url', '')}\n")
        rows.append((name, ver, lic, f"licenses/document2voice/conda/{name}-{ver}/",
                     f"{j.get('url', '')} (sha256 {j.get('sha256', '?')})"))

    # --- Rust crates statically linked into document2voice / document2voice-gui
    apache_text = None
    for name, ver, lic, mdir in rust_crates(a.repo):
        dest = os.path.join(root, "rust", f"{name}-{ver}")
        files = copy_license_files(mdir, dest)
        if not files:
            os.makedirs(dest, exist_ok=True)
            with open(os.path.join(dest, "LICENSE-FROM-CARGO-METADATA.txt"), "w") as f:
                f.write(f"{name} {ver} declares license: {lic}\n"
                        f"(no license file in the published crate)\n")
        for fname in files:
            if apache_text is None and "APACHE" in fname.upper():
                apache_text = os.path.join(dest, fname)
        rows.append((f"{name} (Rust crate)", ver, lic, f"licenses/document2voice/rust/{name}-{ver}/",
                     "crates.io" if "registry" in mdir else "vendored in source tree"))

    # --- ONNX Runtime: linked statically into the binaries by the ort-sys crate
    ort = [r for r in rows if r[0] == "ort-sys (Rust crate)"]
    if ort:
        odir = os.path.join(root, "onnxruntime")
        os.makedirs(odir)
        with open(os.path.join(odir, "NOTICE"), "w") as f:
            f.write("Microsoft ONNX Runtime, MIT License, Copyright (c) Microsoft Corporation.\n"
                    "Statically linked via the ort-sys crate (prebuilt binaries from pyke.io).\n"
                    "Third-party notices: https://github.com/microsoft/onnxruntime/blob/main/ThirdPartyNotices.txt\n")
        rows.append(("ONNX Runtime (static, via ort-sys)", f"ort-sys {ort[0][1]}", "MIT",
                     "licenses/document2voice/onnxruntime/", "https://github.com/microsoft/onnxruntime"))

    # --- Kokoro-82M model + voices (Apache-2.0)
    kdir = os.path.join(root, "kokoro")
    os.makedirs(kdir)
    if not apache_text:
        die("no Apache-2.0 license text found among the Rust crates (needed for the Kokoro model)")
    shutil.copy2(apache_text, os.path.join(kdir, "LICENSE-APACHE-2.0"))
    with open(os.path.join(kdir, "NOTICE"), "w") as f:
        f.write("Kokoro-82M model weights and voice packs by hexgrad, licensed Apache-2.0.\n"
                "https://huggingface.co/hexgrad/Kokoro-82M\n"
                "ONNX export: https://github.com/thewh1teagle/kokoro-onnx (model-files-v1.0)\n")

    # --- shared libraries copied from the build host (not from the conda prefix)
    prefix_libs = set()
    for d in ("lib", "lib64"):
        prefix_libs.update(os.path.basename(p) for p in glob.glob(os.path.join(a.prefix, d, "*.so*")))
    host_libs = sorted(n for n in os.listdir(os.path.join(a.appdir, "usr/lib")) if n not in prefix_libs)
    host_pkgs = host_lib_packages(host_libs, os.path.join(root, "host"))

    notice = os.path.join(root, "THIRD-PARTY-NOTICES.md")
    with open(notice, "w") as f:
        f.write("# Document2Voice — third-party notices\n\n"
                "Document2Voice's own source code is licensed GPL-3.0-or-later (`document2voice/LICENSE`).\n"
                "This AppImage also **bundles separate programs and libraries under their own "
                "licenses**, including GPL-licensed programs (ffmpeg, espeak-ng, Poppler's "
                "`pdftotext`/`pdfinfo`). They run as separate processes; their license texts are "
                "in this folder.\n\n"
                "## Corresponding source (GPL / LGPL / MPL components)\n\n"
                "The source archives for espeak-ng, FFmpeg and Poppler are distributed next to "
                "this AppImage in the `Document2Voice-Kokoro-<version>-sources/` folder (see its "
                "SOURCES.md). Upstream links and the checksums this build used are below. If you "
                "received this AppImage without that folder, ask the distributor for it; under "
                "the GPL they must provide it.\n\n"
                "- ffmpeg static build: the build's `readme.txt` lists the FFmpeg version and every "
                "library linked in; sources at https://johnvansickle.com/ffmpeg/ and "
                "https://ffmpeg.org/releases/\n"
                "- espeak-ng: source tarball listed in the table\n"
                "- Poppler and libraries: conda-forge source recipes at "
                "https://github.com/conda-forge/<package>-feedstock\n\n"
                "## Components\n\n"
                "| Component | Version | License | Files | Source |\n|---|---|---|---|---|\n")
        for r in rows:
            f.write("| " + " | ".join(str(x).replace("|", "\\|") for x in r) + " |\n")
        f.write("| Kokoro-82M model + voices | v1.0 | Apache-2.0 | licenses/document2voice/kokoro/ "
                "| https://huggingface.co/hexgrad/Kokoro-82M |\n")
        f.write("\n## Shared libraries copied from the build machine\n\n"
                "These libraries are copied from the build host's system packages and are "
                "distributed under their respective licenses. Where the host package manager "
                "could name the owning package, its license files are in `host/<package>/`.\n\n")
        for n in host_libs:
            pkg = host_pkgs.get(n)
            f.write(f"- `{n}` (package `{pkg}`)\n" if pkg else f"- `{n}`\n")
    print(f"    licenses: {len(rows)} components → {root}")


if __name__ == "__main__":
    main()
