import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import zipfile

from ci_run import fail, run_logged

ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGET = ROOT / "target" / "dist"
DIST = ROOT / "dist"
STATIC_RUNTIME = "-C target-feature=+crt-static"
TRIPLE = "x86_64-pc-windows-msvc"
NOTICES = "THIRD-PARTY-NOTICES.txt"
SLINT_LICENCE = "LicenseRef-Slint-Royalty-free-2.0"
LICENCE_NAMES = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)", re.I)


def workspace_version():
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    block = re.search(r"^\[workspace\.package\]$(.*?)(?=^\[|\Z)", text, re.S | re.M)
    found = re.search(r'^version\s*=\s*"([^"]+)"', block.group(1), re.M) if block else None
    if not found:
        fail("Cargo.toml has no version under [workspace.package]")
    return found.group(1)


def find_iscc(given):
    candidates = [given, shutil.which("iscc"), shutil.which("ISCC.exe")]
    for base in (os.environ.get("ProgramFiles(x86)"), os.environ.get("ProgramFiles")):
        if base:
            candidates.append(str(pathlib.Path(base) / "Inno Setup 6" / "ISCC.exe"))
    for candidate in candidates:
        if candidate and pathlib.Path(candidate).is_file():
            return candidate
    return None


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(command, env=None):
    print("+", " ".join(str(part) for part in command), flush=True)
    code = run_logged(command, cwd=ROOT, env=env)
    if code != 0:
        fail(f"{pathlib.Path(str(command[0])).name} failed with exit code {code}")


def capture(command):
    done = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if done.returncode != 0:
        fail(f"{command[0]} {command[1]} failed: {done.stderr.strip()[-300:]}")
    return done.stdout


PORTED = [
    (
        "Speex 1.2.1, the decoder, rewritten in Rust for Parrotfish (ps-oldcodecs)  https://www.speex.org",
        ROOT / "ps-oldcodecs" / "LICENSE-speex.txt",
    ),
]


def shipped_packages():
    listed = capture(
        ["cargo", "tree", "--locked", "-p", "ps-app", "-e", "normal", "--target", TRIPLE, "--prefix", "none", "--format", "{p}"]
    )
    wanted = set()
    for line in listed.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            wanted.add((parts[0], parts[1][1:]))
    meta = json.loads(capture(["cargo", "metadata", "--locked", "--format-version", "1"]))
    found = [p for p in meta["packages"] if (p["name"], p["version"]) in wanted and p.get("source")]
    if len(found) < 50:
        fail(f"only {len(found)} third-party packages were found; the notices would be incomplete")
    return sorted(found, key=lambda p: (p["name"].lower(), p["version"]))


def licence_texts(package):
    folder = pathlib.Path(package["manifest_path"]).parent
    declared = package.get("license") or ""
    files = []
    if SLINT_LICENCE in declared:
        files = [folder / "LICENSES" / f"{SLINT_LICENCE}.md"]
    else:
        files = sorted(path for path in folder.iterdir() if path.is_file() and LICENCE_NAMES.match(path.name))
        if package.get("license_file"):
            files.append(folder / package["license_file"])
    texts = []
    for path in files:
        if path.is_file():
            body = path.read_text(encoding="utf-8", errors="replace").replace("\r\n", "\n").strip()
            if body and body not in texts:
                texts.append(body)
    return texts


def write_notices(path, version):
    packages = shipped_packages()
    by_text = {}
    bare = []
    for label, licence in PORTED:
        if not licence.is_file():
            fail(f"the licence text {licence.name} is missing")
        body = licence.read_text(encoding="utf-8").replace("\r\n", "\n").strip()
        by_text.setdefault(body, []).append(label)
    for package in packages:
        label = f"{package['name']} {package['version']} ({package.get('license') or 'see its files'})"
        if package.get("repository"):
            label += f"  {package['repository']}"
        texts = licence_texts(package)
        if not texts:
            bare.append(label)
        for text in texts:
            by_text.setdefault(text, []).append(label)
    rule = "=" * 78
    out = [
        f"Parrotfish {version} is built with the software listed below.",
        "Each block names the packages and then gives the licence text they are shipped under.",
        "Slint is used under the Slint Royalty-free Desktop, Mobile, and Web Applications License.",
        f"{len(packages)} packages, {len(by_text)} distinct licence texts.",
        "",
    ]
    for text, labels in sorted(by_text.items(), key=lambda item: (-len(item[1]), item[1][0])):
        out += [rule, *labels, "-" * 78, text, ""]
    if bare:
        out += [rule, "These packages carry no licence file; their declared licence is given in brackets.", *bare, ""]
    path.write_text("\n".join(out), encoding="utf-8", newline="\n")
    return len(packages), len(by_text), len(bare)


def main():
    parser = argparse.ArgumentParser(description="Build the Parrotfish release files into dist/.")
    parser.add_argument("--tag", help="the release tag, for example v0.1.0; it must match the version in Cargo.toml")
    parser.add_argument("--check", action="store_true", help="only check the tag against Cargo.toml")
    parser.add_argument("--skip-build", action="store_true", help="package the program that is already built")
    parser.add_argument("--skip-installer", action="store_true", help="make the zip only")
    parser.add_argument("--notices-only", action="store_true", help="only write the third-party notices into dist/")
    parser.add_argument("--iscc", help="path to Inno Setup's ISCC.exe")
    args = parser.parse_args()

    version = workspace_version()
    if args.tag is not None and args.tag != f"v{version}":
        fail(f"the tag is {args.tag} but Cargo.toml says version {version}; the tag must be v{version}")

    if args.check:
        print(f"version {version}")
        return

    if args.notices_only:
        DIST.mkdir(parents=True, exist_ok=True)
        counts = write_notices(DIST / NOTICES, version)
        print(f"{NOTICES}: {counts[0]} packages, {counts[1]} licence texts, {counts[2]} without a licence file")
        return

    iscc = None if args.skip_installer else find_iscc(args.iscc)
    if not args.skip_installer and iscc is None:
        fail("Inno Setup 6 was not found. Install it, pass --iscc, or use --skip-installer.")

    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(TARGET)
    env.setdefault("RUSTFLAGS", STATIC_RUNTIME)
    if not args.skip_build:
        run(["cargo", "build", "--release", "--locked", "-p", "ps-app"], env)
    built = TARGET / "release" / "ps-app.exe"
    if not built.is_file():
        fail(f"{built} does not exist; run without --skip-build first")

    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir(parents=True)
    produced = []

    notices = DIST / NOTICES
    counts = write_notices(notices, version)
    print(f"{NOTICES}: {counts[0]} packages, {counts[1]} licence texts, {counts[2]} without a licence file")

    archive = DIST / f"Parrotfish-{version}-windows-x64.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        bundle.write(built, "Parrotfish.exe")
        bundle.write(ROOT / "README.md", "README.md")
        bundle.write(notices, NOTICES)
    produced.append(archive)

    if iscc is not None:
        run(
            [
                iscc,
                f"/DAppVersion={version}",
                f"/DSourceExe={built}",
                f"/DNoticesFile={notices}",
                f"/DReadmeFile={ROOT / 'README.md'}",
                f"/DOutputDir={DIST}",
                "installer\\parrotfish.iss",
            ]
        )
        setup = DIST / f"Parrotfish-{version}-setup.exe"
        if not setup.is_file():
            fail(f"Inno Setup finished but {setup.name} is missing")
        produced.append(setup)

    notices.unlink()
    sums = DIST / "SHA256SUMS.txt"
    sums.write_text("".join(f"{sha256(path)}  {path.name}\n" for path in produced), encoding="utf-8", newline="\n")
    produced.append(sums)

    print(f"Parrotfish {version}")
    for path in produced:
        print(f"  {path.relative_to(ROOT)}  {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
