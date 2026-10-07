import os
import re
import subprocess
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
TELLING = (
    re.compile(r"^error(\[E\d+\])?:"),
    re.compile(r"panicked at"),
    re.compile(r"^test .* \.\.\. FAILED"),
    re.compile(r"^---- .* stdout ----"),
    re.compile(r"^\s*(left|right):"),
    re.compile(r"assertion .* failed"),
    re.compile(r"^test result: FAILED"),
    re.compile(r"^\s+--> "),
    re.compile(r"^Error"),
    re.compile(r"failed with exit code"),
)
MAX_ANNOTATIONS = 10


def on_github():
    return os.environ.get("GITHUB_ACTIONS") == "true"


def annotate(message):
    text = message.strip()[:400].replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::error::{text}" if on_github() else f"error: {message.strip()}", flush=True)


def fail(message):
    annotate(message)
    sys.exit(1)


def run_logged(command, cwd=None, env=None):
    process = subprocess.Popen(
        [str(part) for part in command],
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    kept = []
    for line in process.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        plain = ANSI.sub("", line.rstrip())
        if any(pattern.search(plain) for pattern in TELLING) and plain not in kept:
            kept.append(plain)
    code = process.wait()
    if code != 0:
        for line in kept[:MAX_ANNOTATIONS]:
            annotate(line)
        if not kept:
            annotate(f"{command[0]} failed with exit code {code} and printed nothing that looks like an error")
    return code


def main():
    if len(sys.argv) < 2:
        sys.exit("usage: ci_run.py COMMAND [ARGUMENT]...\nRuns the command and, when it fails on GitHub, repeats its error lines as annotations.")
    sys.exit(run_logged(sys.argv[1:]))


if __name__ == "__main__":
    main()
