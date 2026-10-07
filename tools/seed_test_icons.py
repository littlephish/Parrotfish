import socket
import struct
import sys
import time
import zlib

USAGE = """usage: seed_test_icons.py --password PW [--host HOST] [--query-port N]
                             [--count N | --clear | --upload FILE... | --assign WHAT=ID...]

Puts icons on a local TeamSpeak test server through ServerQuery.

  (no option)      the standard test set: good icons, a large one, and files that must be refused
  --count N        upload N small icons and print their ids, one per line
  --upload FILE    upload a file as an icon and print its id (may be repeated)
  --assign WHAT=ID give an icon to something (may be repeated); WHAT is one of
                   server, channel:NAME, group:NAME, channelgroup:NAME, client:NICKNAME
                   and ID 0 takes the icon away again
  --clear          remove every icon assignment made by the standard test set
"""

COLOURS = [(240, 112, 95), (255, 192, 67), (30, 200, 120), (120, 150, 255), (200, 120, 220)]


def esc(text):
    return text.replace("\\", "\\\\").replace("/", "\\/").replace(" ", "\\s").replace("|", "\\p")


def unesc(text):
    return text.replace("\\s", " ").replace("\\p", "|").replace("\\/", "/").replace("\\\\", "\\")


def png(colour, salt, side=16):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(side):
        raw.append(0)
        for x in range(side):
            edge = x in (0, side - 1) or y in (0, side - 1)
            pixel = (255, 255, 255) if edge else colour
            if (x, y) == (1, 1):
                pixel = (salt & 255, (salt >> 8) & 255, 0)
            if side > 16 and (x // (side // 8) + y // (side // 8)) % 2 == 0 and not edge:
                pixel = tuple(value // 2 for value in colour)
            raw.extend(pixel)
    header = struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")


def gif(colour, side=16):
    codes = []
    for index in range(side * side):
        if index % 2 == 0:
            codes.append(4)
        codes.append(1)
    codes.append(5)
    bits = 0
    count = 0
    packed = bytearray()
    for code in codes:
        bits |= code << count
        count += 3
        while count >= 8:
            packed.append(bits & 255)
            bits >>= 8
            count -= 8
    if count:
        packed.append(bits & 255)
    blocks = bytearray()
    for start in range(0, len(packed), 255):
        part = packed[start : start + 255]
        blocks.append(len(part))
        blocks.extend(part)
    blocks.append(0)
    table = bytes((0, 0, 0)) + bytes(colour) + bytes(6)
    screen = struct.pack("<HHBBB", side, side, 0x81, 0, 0)
    image = b"," + struct.pack("<HHHHB", 0, 0, side, side, 0)
    return b"GIF89a" + screen + table + image + bytes([2]) + bytes(blocks) + b";"


class Query:
    def __init__(self, host, port, password):
        self.host = host
        self.sock = socket.create_connection((host, port), timeout=5)
        self.buffer = b""
        self.read(b"specific command.")
        self.cmd(f"login serveradmin {password}")
        self.cmd("use sid=1")
        self.transfer = 0

    def read(self, marker):
        while marker not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                break
            self.buffer += chunk
        end = self.buffer.find(b"\n", self.buffer.find(marker))
        end = len(self.buffer) if end < 0 else end + 1
        out, self.buffer = self.buffer[:end], self.buffer[end:]
        return out.decode("utf-8", "replace")

    def cmd(self, text, quiet=False):
        self.sock.sendall(text.encode("utf-8") + b"\n")
        lines = [line.strip() for line in self.read(b"error id=").replace("\r", "").split("\n") if line.strip()]
        if "id=0 " not in lines[-1] + " " and not quiet:
            shown = text if not text.startswith("login ") else "login"
            print(f"  ! {shown[:80]} -> {lines[-1]}")
        rows = []
        for line in lines[:-1]:
            for item in line.split("|"):
                rows.append({k: unesc(v) for k, _, v in (pair.partition("=") for pair in item.split(" "))})
        time.sleep(0.05)
        return rows

    def upload(self, name, data):
        self.transfer += 1
        rows = self.cmd(
            f"ftinitupload clientftfid={self.transfer} name={esc('/' + name)} cid=0 cpw= size={len(data)} overwrite=1 resume=0 proto=1"
        )
        if not rows or "ftkey" not in rows[0]:
            return False
        with socket.create_connection((self.host, int(rows[0]["port"])), timeout=5) as link:
            link.sendall(rows[0]["ftkey"].encode("ascii"))
            link.sendall(data)
        time.sleep(0.2)
        return True

    def icon(self, data):
        number = zlib.crc32(data) & 0xFFFFFFFF
        if not self.upload(f"icon_{number}", data):
            print(f"  ! icon_{number} was not accepted")
        return number

    def channels(self):
        return {row["channel_name"]: row["cid"] for row in self.cmd("channellist")}

    def group(self, name, channel=False):
        key, command = ("cgid", "channelgrouplist") if channel else ("sgid", "servergrouplist")
        for row in self.cmd(command):
            if row.get("name") == name and row.get("type") == "1":
                return row[key]
        sys.exit(f"there is no group called {name!r}")

    def assign(self, what, number):
        kind, _, name = what.partition(":")
        if kind == "server":
            self.cmd(f"serveredit virtualserver_icon_id={number}")
        elif kind == "channel":
            channels = self.channels()
            if name not in channels:
                sys.exit(f"there is no channel called {name!r}")
            if number:
                self.cmd(f"channeladdperm cid={channels[name]} permsid=i_icon_id permvalue={number}")
            else:
                self.cmd(f"channeldelperm cid={channels[name]} permsid=i_icon_id", quiet=True)
        elif kind == "group":
            group = self.group(name)
            if number:
                self.cmd(f"servergroupaddperm sgid={group} permsid=i_icon_id permvalue={number} permnegated=0 permskip=0")
            else:
                self.cmd(f"servergroupdelperm sgid={group} permsid=i_icon_id", quiet=True)
        elif kind == "channelgroup":
            group = self.group(name, channel=True)
            if number:
                self.cmd(f"channelgroupaddperm cgid={group} permsid=i_icon_id permvalue={number}")
            else:
                self.cmd(f"channelgroupdelperm cgid={group} permsid=i_icon_id", quiet=True)
        elif kind == "client":
            found = [row for row in self.cmd("clientlist") if row.get("client_nickname") == name]
            if not found:
                sys.exit(f"nobody called {name!r} is connected")
            owner = found[0]["client_database_id"]
            if number:
                self.cmd(f"clientaddperm cldbid={owner} permsid=i_icon_id permvalue={number} permskip=0")
            else:
                self.cmd(f"clientdelperm cldbid={owner} permsid=i_icon_id", quiet=True)
        else:
            sys.exit(f"cannot give an icon to {what!r}")


STANDARD_SET = ["channel:Deep Rock", "channel:Tide Pool", "channel:Lobby", "channel:Squad Alpha", "channel:Radio",
                "channel:Drift", "channel:Booth", "group:Guest", "server"]


def values(args, name):
    return [args[index + 1] for index, arg in enumerate(args) if arg == name and index + 1 < len(args)]


def main():
    args = sys.argv[1:]
    if "--password" not in args or "--help" in args:
        sys.exit(USAGE)
    host = (values(args, "--host") or ["127.0.0.1"])[0]
    port = int((values(args, "--query-port") or ["10011"])[0])
    query = Query(host, port, values(args, "--password")[0])
    if "--count" in args:
        for index in range(int(values(args, "--count")[0])):
            print(query.icon(png(COLOURS[index % len(COLOURS)], 1000 + index)))
        return
    if "--clear" in args:
        for what in STANDARD_SET:
            query.assign(what, 0)
        print("cleared")
        return
    uploads = values(args, "--upload")
    assignments = values(args, "--assign")
    if uploads or assignments:
        for path in uploads:
            with open(path, "rb") as source:
                print(query.icon(source.read()))
        for item in assignments:
            what, _, number = item.rpartition("=")
            query.assign(what, int(number))
        return
    first, second = query.icon(png(COLOURS[0], 1)), query.icon(png(COLOURS[1], 2))
    large = query.icon(png(COLOURS[2], 3, side=64))
    animated = query.icon(gif(COLOURS[3]))
    huge = query.icon(png(COLOURS[4], 4, side=300))
    query.upload("icon_600001", b"\x89PNG\r\n\x1a\n" + bytes(600 * 1024))
    query.upload("icon_600002", b"<html><body>not an icon</body></html>")
    query.assign("channel:Deep Rock", first)
    query.assign("channel:Tide Pool", second)
    query.assign("channel:Lobby", large)
    query.assign("channel:Squad Alpha", 600001)
    query.assign("channel:Radio", 600002)
    query.assign("channel:Drift", animated)
    query.assign("channel:Booth", huge)
    query.assign("group:Guest", second)
    query.assign("server", first)
    print(f"seeded: {first} on Deep Rock and the server, {second} on Tide Pool and Guest, {large} (64 x 64) on Lobby,")
    print(f"        {animated} (GIF, shown as its first frame) on Drift")
    print("        to be refused: 600001 (too many bytes) on Squad Alpha, 600002 (not an image) on Radio,")
    print(f"        {huge} (300 x 300) on Booth")


main()
