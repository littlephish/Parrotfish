import argparse
import hashlib
import os
import pathlib
import re
import shutil
import subprocess
import sys
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGET = ROOT / "target" / "dist"
DIST = ROOT / "dist"
STATIC_RUNTIME = "-C target-feature=+crt-static"


def workspace_version():
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    block = re.search(r"^\[workspace\.package\]$(.*?)(?=^\[|\Z)", text, re.S | re.M)
    found = re.search(r'^version\s*=\s*"([^"]+)"', block.group(1), re.M) if block else None
    if not found:
        sys.exit("Cargo.toml has no version under [workspace.package]")
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
    done = subprocess.run(command, cwd=ROOT, env=env)
    if done.returncode != 0:
        sys.exit(f"{pathlib.Path(str(command[0])).name} failed with exit code {done.returncode}")


def main():
    parser = argparse.ArgumentParser(description="Build the PhishSpeak release files into dist/.")
    parser.add_argument("--tag", help="the release tag, for example v0.1.0; it must match the version in Cargo.toml")
    parser.add_argument("--check", action="store_true", help="only check the tag against Cargo.toml")
    parser.add_argument("--skip-build", action="store_true", help="package the program that is already built")
    parser.add_argument("--skip-installer", action="store_true", help="make the zip only")
    parser.add_argument("--iscc", help="path to Inno Setup's ISCC.exe")
    args = parser.parse_args()

    version = workspace_version()
    if args.tag is not None and args.tag != f"v{version}":
        sys.exit(f"the tag is {args.tag} but Cargo.toml says version {version}; the tag must be v{version}")

    if args.check:
        print(f"version {version}")
        return

    iscc = None if args.skip_installer else find_iscc(args.iscc)
    if not args.skip_installer and iscc is None:
        sys.exit("Inno Setup 6 was not found. Install it, pass --iscc, or use --skip-installer.")

    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(TARGET)
    env.setdefault("RUSTFLAGS", STATIC_RUNTIME)
    if not args.skip_build:
        run(["cargo", "build", "--release", "--locked", "-p", "ps-app"], env)
    built = TARGET / "release" / "ps-app.exe"
    if not built.is_file():
        sys.exit(f"{built} does not exist; run without --skip-build first")

    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir(parents=True)
    produced = []

    archive = DIST / f"PhishSpeak-{version}-windows-x64.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        bundle.write(built, "PhishSpeak.exe")
        bundle.write(ROOT / "README.md", "README.md")
    produced.append(archive)

    if iscc is not None:
        run([iscc, f"/DAppVersion={version}", f"/DSourceExe={built}", f"/DOutputDir={DIST}", "installer\\phishspeak.iss"])
        setup = DIST / f"PhishSpeak-{version}-setup.exe"
        if not setup.is_file():
            sys.exit(f"Inno Setup finished but {setup.name} is missing")
        produced.append(setup)

    sums = DIST / "SHA256SUMS.txt"
    sums.write_text("".join(f"{sha256(path)}  {path.name}\n" for path in produced), encoding="utf-8", newline="\n")
    produced.append(sums)

    print(f"PhishSpeak {version}")
    for path in produced:
        print(f"  {path.relative_to(ROOT)}  {path.stat().st_size} bytes")


main()
