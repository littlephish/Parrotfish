import argparse
import socket
import sys

ESCAPES = [("\\", "\\\\"), ("/", "\\/"), (" ", "\\s"), ("|", "\\p"), ("\t", "\\t"), ("\n", "\\n"), ("\r", "\\r")]


def escape(text):
    for plain, coded in ESCAPES:
        text = text.replace(plain, coded)
    return text


def unescape(text):
    out = []
    index = 0
    codes = {"\\": "\\", "/": "/", "s": " ", "p": "|", "t": "\t", "n": "\n", "r": "\r"}
    while index < len(text):
        if text[index] == "\\" and index + 1 < len(text):
            out.append(codes.get(text[index + 1], text[index + 1]))
            index += 2
        else:
            out.append(text[index])
            index += 1
    return "".join(out)


class Query:
    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=5)
        self.pending = b""
        self.line()
        self.line()

    def line(self):
        while b"\n\r" not in self.pending:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("the server closed the query connection")
            self.pending += chunk
        raw, self.pending = self.pending.split(b"\n\r", 1)
        return raw.decode("utf-8", "replace")

    def run(self, command):
        self.sock.sendall(command.encode("utf-8") + b"\n")
        rows = []
        while True:
            line = self.line()
            if line.startswith("error "):
                fields = parse(line[6:])[0]
                return rows, int(fields.get("id", "1")), fields.get("msg", "")
            rows.extend(parse(line))

    def must(self, command):
        rows, code, message = self.run(command)
        if code != 0:
            shown = command.split(" ")[0]
            sys.exit(f"{shown} failed: {message} (error {code})")
        return rows


def parse(line):
    rows = []
    for item in line.split("|"):
        fields = {}
        for part in item.split(" "):
            name, _, value = part.partition("=")
            if name:
                fields[name] = unescape(value)
        rows.append(fields)
    return rows


def ensure_channel(query, channels, name, parent_name):
    parent = next((row for row in channels if row.get("channel_name") == parent_name), None)
    if parent is None:
        sys.exit(f"the server has no channel called {parent_name}")
    existing = next(
        (row for row in channels if row.get("channel_name") == name and row.get("pid") == parent["cid"]),
        None,
    )
    if existing is not None:
        return existing["cid"]
    made = query.must(f"channelcreate channel_name={escape(name)} channel_flag_permanent=1 cpid={parent['cid']}")
    return made[0]["cid"]


def main():
    parser = argparse.ArgumentParser(description="Prepare a TeamSpeak 3 test server for the whisper checks.")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=10011)
    parser.add_argument("--server", type=int, default=1)
    parser.add_argument("--password", required=True, help="the serveradmin query password")
    args = parser.parse_args()

    query = Query(args.host, args.port)
    query.must(f"login serveradmin {escape(args.password)}")
    query.must(f"use sid={args.server}")
    channels = query.must("channellist")
    booth = ensure_channel(query, channels, "Booth", "Radio")
    drift = ensure_channel(query, channels, "Drift", "Deep Rock")
    groups = query.must("servergrouplist")
    guest = next((row for row in groups if row.get("name") == "Guest" and row.get("type") == "1"), None)
    if guest is None:
        sys.exit("the server has no regular server group called Guest")
    query.must(
        f"servergroupaddperm sgid={guest['sgid']} permsid=b_client_use_channel_commander "
        "permvalue=1 permnegated=0 permskip=0"
    )
    query.run("quit")
    print(f"--booth {booth} --drift {drift}")


main()
