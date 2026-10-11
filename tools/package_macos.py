import argparse
import gzip
import hashlib
import os
import pathlib
import plistlib
import shutil
import struct
import subprocess
import zipfile
import zlib

import package_release as release
from ci_run import fail

ROOT = pathlib.Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
SOURCES = ROOT / "installer" / "macos"
ICON = ROOT / "ps-app" / "ui" / "app-icon.png"
NAME = "Parrotfish"
IDENTIFIER = "io.github.littlephish.Parrotfish"
MINIMUM = "11.0"
TRIPLES = ("x86_64-apple-darwin", "aarch64-apple-darwin")
PROCESSORS = {0x01000007: "x86_64", 0x0100000C: "arm64"}
SIGNATURE = 0x1D
ICON_SIZES = (("icp4", 16), ("icp5", 32), ("ic11", 32), ("ic12", 64), ("ic07", 128), ("ic13", 256), ("ic08", 256))
PNG = b"\x89PNG\r\n\x1a\n"


def thin_parts(data):
    if data[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
        wide = data[3] == 0xBF
        count = struct.unpack(">I", data[4:8])[0]
        parts = []
        for index in range(min(count, 8)):
            at = 8 + index * (32 if wide else 20)
            if wide:
                offset, size = struct.unpack(">QQ", data[at + 8 : at + 24])
            else:
                offset, size = struct.unpack(">II", data[at + 8 : at + 16])
            parts.append(data[offset : offset + size])
        return parts
    return [data] if data[:4] == b"\xcf\xfa\xed\xfe" else []


def signatures(path):
    found = {}
    for part in thin_parts(path.read_bytes()):
        processor, commands = struct.unpack("<I", part[4:8])[0], struct.unpack("<I", part[16:20])[0]
        at, signed = 32, False
        for _ in range(commands):
            command, size = struct.unpack("<II", part[at : at + 8])
            signed = signed or command == SIGNATURE
            at += size
        found[PROCESSORS.get(processor, hex(processor))] = signed
    return found


def one_program(parts, out):
    for part in parts:
        if len(signatures(part)) != 1:
            fail(f"{part} is not a program for one kind of Mac")
    if len(parts) == 1:
        shutil.copyfile(parts[0], out)
    else:
        lipo = shutil.which("lipo") or shutil.which("llvm-lipo")
        if not lipo:
            fail("lipo was not found; it joins the programs for the two kinds of Mac into one")
        release.run([lipo, "-create", "-output", out, *parts])
    os.chmod(out, 0o755)
    return sorted(signatures(out))


def read_png(data):
    if data[:8] != PNG:
        fail("the icon is not a PNG file")
    at, packed, head = 8, b"", None
    while at + 8 <= len(data):
        size, kind = struct.unpack(">I4s", data[at : at + 8])
        body = data[at + 8 : at + 8 + size]
        at += 12 + size
        if kind == b"IHDR":
            head = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            packed += body
    if not head or head[2] != 8 or head[3] not in (2, 6) or head[6] != 0:
        fail("the icon has to be a plain PNG with 8 bits for each colour")
    width, height, step = head[0], head[1], 4 if head[3] == 6 else 3
    raw, stride = zlib.decompress(packed), head[0] * step
    rows, before = [], bytearray(stride)
    for y in range(height):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1 : (y + 1) * (stride + 1)])
        for i in range(stride):
            left = line[i - step] if i >= step else 0
            above = before[i]
            corner = before[i - step] if i >= step else 0
            if kind == 1:
                add = left
            elif kind == 2:
                add = above
            elif kind == 3:
                add = (left + above) // 2
            elif kind == 4:
                guess = left + above - corner
                near = (abs(guess - left), abs(guess - above), abs(guess - corner))
                add = left if near[0] <= near[1] and near[0] <= near[2] else above if near[1] <= near[2] else corner
            else:
                add = 0
            line[i] = (line[i] + add) & 255
        before = line
        if step == 3:
            solid = bytearray()
            for x in range(0, stride, 3):
                solid += line[x : x + 3] + b"\xff"
            rows.append(solid)
        else:
            rows.append(line)
    return width, height, rows


def write_png(rows):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    side = len(rows)
    raw = b"".join(b"\x00" + bytes(row) for row in rows)
    return PNG + chunk(b"IHDR", struct.pack(">IIBBBBB", side, side, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def halve(rows):
    out = []
    for y in range(0, len(rows) - 1, 2):
        upper, lower, line = rows[y], rows[y + 1], bytearray()
        for x in range(0, len(upper) - 7, 8):
            block = (upper[x : x + 4], upper[x + 4 : x + 8], lower[x : x + 4], lower[x + 4 : x + 8])
            alpha = sum(pixel[3] for pixel in block)
            if alpha == 0:
                line += b"\x00\x00\x00\x00"
                continue
            for colour in range(3):
                line.append(min(255, round(sum(pixel[colour] * pixel[3] for pixel in block) / alpha)))
            line.append(round(alpha / 4))
        out.append(line)
    return out


def icon_file(source):
    width, height, rows = read_png(source.read_bytes())
    if width != height or width != 256:
        fail(f"the icon has to be 256 pixels square; it is {width} by {height}")
    pictures, side = {width: rows}, width
    while side > 16:
        rows, side = halve(rows), side // 2
        pictures[side] = rows
    body = b""
    for kind, wanted in ICON_SIZES:
        picture = write_png(pictures[wanted])
        body += kind.encode("ascii") + struct.pack(">I", len(picture) + 8) + picture
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def bundle(version, program, folder, notices):
    app = folder / f"{NAME}.app"
    if app.exists():
        shutil.rmtree(app)
    contents = app / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir()
    shutil.copyfile(program, contents / "MacOS" / NAME)
    os.chmod(contents / "MacOS" / NAME, 0o755)
    facts = (SOURCES / "Info.plist").read_text(encoding="utf-8")
    for mark, value in (("@VERSION@", version), ("@IDENTIFIER@", IDENTIFIER), ("@MINIMUM@", MINIMUM)):
        facts = facts.replace(mark, value)
    read = plistlib.loads(facts.encode("utf-8"))
    if "@" in facts or read.get("CFBundleExecutable") != NAME or read.get("CFBundleShortVersionString") != version:
        fail("installer/macos/Info.plist did not come out as it should")
    (contents / "Info.plist").write_text(facts, encoding="utf-8", newline="\n")
    (contents / "PkgInfo").write_bytes(b"APPL????")
    (contents / "Resources" / f"{NAME}.icns").write_bytes(icon_file(ICON))
    for licence in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copyfile(ROOT / licence, contents / "Resources" / licence)
    if notices:
        counts = release.write_notices(contents / "Resources" / release.NOTICES, version, TRIPLES, release.PORTED[:1])
        print(f"{release.NOTICES}: {counts[0]} packages, {counts[1]} licence texts, {counts[2]} without a licence file")
    return app


def sign(app, key, password):
    tool = shutil.which("rcodesign")
    if not tool:
        fail("rcodesign was not found; a Mac with an Apple processor refuses a program that is not signed at all")
    command = [tool, "sign"]
    if key:
        command += ["--p12-file", key, "--p12-password-file", password, "--code-signature-flags", "runtime"]
        command += ["--entitlements-xml-file", SOURCES / "entitlements.plist"]
    release.run(command + [app])
    marks = signatures(app / "Contents" / "MacOS" / NAME)
    if not marks or not all(marks.values()) or not (app / "Contents" / "_CodeSignature" / "CodeResources").is_file():
        fail(f"the app did not come out signed: {marks}")
    return "with your certificate" if key else "ad hoc, with no certificate"


def tidy(app):
    for path in [app, *app.rglob("*")]:
        os.chmod(path, 0o755 if path.is_dir() or path.parent.name == "MacOS" else 0o644)


def listed(bom):
    data = bom.read_bytes()
    if data[:8] != b"BOMStore":
        return -1
    index, names = struct.unpack(">I", data[16:20])[0], struct.unpack(">I", data[24:28])[0]

    def block(number):
        at, size = struct.unpack(">II", data[index + 4 + 8 * number : index + 12 + 8 * number])
        return data[at : at + size]

    at = names + 4
    for _ in range(struct.unpack(">I", data[names : names + 4])[0]):
        number, length = struct.unpack(">IB", data[at : at + 5])
        if data[at + 5 : at + 5 + length] == b"Paths":
            tree = block(number)
            return struct.unpack(">I", tree[16:20])[0] if tree[:4] == b"tree" else -1
        at += 5 + length
    return -1


def zipped(app, out):
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as packed:
        for path in [app, *sorted(app.rglob("*"))]:
            name = path.relative_to(app.parent).as_posix()
            if path.is_dir():
                entry = zipfile.ZipInfo(name + "/")
                entry.create_system = 3
                entry.external_attr = (0o40755 << 16) | 0x10
                packed.writestr(entry, b"")
            else:
                entry = zipfile.ZipInfo(name)
                entry.create_system = 3
                entry.compress_type = zipfile.ZIP_DEFLATED
                entry.external_attr = (0o100755 if path.parent.name == "MacOS" else 0o100644) << 16
                packed.writestr(entry, path.read_bytes())


def fresh(folder):
    if folder.exists():
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    return folder


def disk_image(app, out, work):
    stage = fresh(work / "dmg")
    shutil.copytree(app, stage / app.name, symlinks=True)
    os.symlink("/Applications", stage / "Applications")
    if shutil.which("xorriso"):
        command = ["xorriso", "-as", "mkisofs", "-V", NAME, "-D", "-r", "-hfsplus", "-no-pad", "-o", out, stage]
    elif shutil.which("genisoimage"):
        command = ["genisoimage", "-quiet", "-V", NAME, "-D", "-r", "-apple", "-no-pad", "-o", out, stage]
    else:
        fail("neither xorriso nor genisoimage was found; one of them writes the disk image")
    release.run(command)


def installer_package(app, out, work, version):
    tools = {name: shutil.which(name) for name in ("mkbom", "xar", "cpio")}
    missing = [name for name, found in tools.items() if not found]
    if missing:
        fail(f"{', '.join(missing)} not found; the installer package is made with mkbom, xar and cpio")
    work = fresh(work / "pkg")
    root = work / "root"
    shutil.copytree(app, root / app.name, symlinks=True)
    release.run([tools["mkbom"], "-u", "0", "-g", "80", root, work / "Bom"])
    entries = sorted(root.rglob("*"))
    if listed(work / "Bom") != len(entries) + 1:
        fail(f"the list of files in the package names {listed(work / 'Bom')} things, not the {len(entries) + 1} that are in it")
    listing = "".join(["./\n"] + [f"./{path.relative_to(root).as_posix()}\n" for path in entries])
    archive = subprocess.run([tools["cpio"], "-o", "--format", "odc", "--owner", "0:80"], cwd=root, input=listing.encode(), capture_output=True)
    if archive.returncode != 0 or not archive.stdout:
        fail(f"cpio failed: {archive.stderr.decode(errors='replace').strip()[-300:]}")
    (work / "Payload").write_bytes(gzip.compress(archive.stdout, 9))
    kilobytes = sum(path.stat().st_size for path in entries if path.is_file()) // 1024 + 1
    (work / "PackageInfo").write_text(
        f"""<?xml version="1.0" encoding="utf-8"?>
<pkg-info format-version="2" identifier="{IDENTIFIER}.pkg" version="{version}" install-location="/Applications" auth="root" overwrite-action="upgrade" relocatable="false" postinstall-action="none">
    <payload numberOfFiles="{len(entries) + 1}" installKBytes="{kilobytes}"/>
    <bundle path="./{app.name}" id="{IDENTIFIER}" CFBundleShortVersionString="{version}" CFBundleVersion="{version}"/>
    <bundle-version>
        <bundle id="{IDENTIFIER}"/>
    </bundle-version>
    <upgrade-bundle>
        <bundle id="{IDENTIFIER}"/>
    </upgrade-bundle>
    <update-bundle/>
    <atomic-update-bundle/>
    <strict-identifier>
        <bundle id="{IDENTIFIER}"/>
    </strict-identifier>
    <relocate/>
</pkg-info>
""",
        encoding="utf-8",
        newline="\n",
    )
    if out.exists():
        out.unlink()
    release.run([tools["xar"], "--compression", "none", "-cf", out, "Bom", "PackageInfo", "Payload"], cwd=work)


def main():
    parser = argparse.ArgumentParser(description="Make the Mac files of Parrotfish in dist/ from programs that are already built.")
    parser.add_argument("--program", action="append", default=[], help="a built program; by default the two under target/<triple>/release")
    parser.add_argument("--skip-notices", action="store_true", help="leave the third-party notices out (cargo is needed to write them)")
    parser.add_argument("--skip-image", action="store_true", help="do not make the disk image (.dmg)")
    parser.add_argument("--skip-installer", action="store_true", help="do not make the installer package (.pkg)")
    parser.add_argument("--p12", help="a Developer ID certificate with its key, to sign with instead of ad hoc")
    parser.add_argument("--p12-password-file", help="a file holding the password of --p12")
    args = parser.parse_args()
    if bool(args.p12) != bool(args.p12_password_file):
        fail("--p12 and --p12-password-file go together")
    version = release.workspace_version()
    programs = [pathlib.Path(given) for given in args.program]
    if not programs:
        programs = [path for path in (ROOT / "target" / triple / "release" / "ps-app" for triple in TRIPLES) if path.is_file()]
    if not programs:
        fail("no built program was found; build with cargo build --release -p ps-app --target <triple> first")
    work = fresh(DIST / "macos")
    kinds = one_program(programs, work / NAME)
    label = "universal" if len(kinds) > 1 else kinds[0]
    app = bundle(version, work / NAME, work, not args.skip_notices)
    signed = sign(app, args.p12, args.p12_password_file)
    tidy(app)
    made = [DIST / f"{NAME}-{version}-macos-{label}.zip"]
    zipped(app, made[0])
    if not args.skip_image:
        made.append(DIST / f"{NAME}-{version}-macos-{label}.dmg")
        disk_image(app, made[-1], work)
    if not args.skip_installer:
        made.append(DIST / f"{NAME}-{version}-macos-{label}.pkg")
        installer_package(app, made[-1], work, version)
    sums = DIST / "SHA256SUMS-macos.txt"
    sums.write_text("".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in made), encoding="utf-8", newline="\n")
    print(f"{NAME} {version} for {' and '.join(kinds)}, signed {signed}")
    for path in [*made, sums]:
        print(f"  {path.stat().st_size:>10}  {path.relative_to(ROOT).as_posix()}")


if __name__ == "__main__":
    main()
