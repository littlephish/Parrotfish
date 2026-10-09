import argparse
import ctypes
import os
import pathlib
import shutil
import subprocess
import tempfile
import time
import winreg

from ci_run import annotate, fail, note, on_github, run_logged
from package_release import DIST, ROOT, TARGET, find_iscc, workspace_version

UNINSTALL = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{B21EECED-B833-438E-BA5E-C0C98D9278CD}_is1"
SCHEME = "ts3server-test"
LINKS = "Software\\Classes\\" + SCHEME
SETUP_QUIET = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"]
REMOVE_QUIET = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]
EARLIER = "PhishSpeak"
NOW = "Parrotfish"
EARLIER_VERSION = "0.4.1"
EARLIER_SCRIPT = ROOT / "installer" / "upgrade-test" / "phishspeak-0.4.1.iss"
SETUP_WAIT = 300
START_WAIT = 60
BOOKMARKS = "[bookmark]\nname=Reef Runners\naddress=reef.example.net\n"
STAND_IN = b"a shortcut somebody made themselves"

problems = []
checked = 0
started = []


def check(good, what):
    global checked
    checked += 1
    print(("  ok    " if good else "  WRONG ") + what, flush=True)
    if not good:
        problems.append(what)
    return good


def shell_folder(number):
    buffer = ctypes.create_unicode_buffer(520)
    ctypes.windll.shell32.SHGetFolderPathW(None, number, None, 0, buffer)
    if not buffer.value:
        fail(f"Windows did not name its folder number {number}")
    return pathlib.Path(buffer.value)


def uninstall_entry():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, UNINSTALL) as key:
            found = {}
            for name in ("DisplayName", "DisplayVersion", "InstallLocation"):
                try:
                    found[name] = winreg.QueryValueEx(key, name)[0]
                except OSError:
                    found[name] = None
            return found
    except OSError:
        return None


def links_command():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, LINKS + "\\shell\\open\\command") as key:
            return winreg.QueryValueEx(key, None)[0]
    except OSError:
        return None


def links_key_exists():
    try:
        winreg.CloseKey(winreg.OpenKey(winreg.HKEY_CURRENT_USER, LINKS))
        return True
    except OSError:
        return False


def take_links(command):
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, LINKS) as key:
        winreg.SetValueEx(key, None, 0, winreg.REG_SZ, f"URL:{SCHEME} link")
        winreg.SetValueEx(key, "URL Protocol", 0, winreg.REG_SZ, "")
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, LINKS + "\\shell\\open\\command") as key:
        winreg.SetValueEx(key, None, 0, winreg.REG_SZ, command)


def drop_links():
    for sub in ("\\shell\\open\\command", "\\shell\\open", "\\shell", ""):
        try:
            winreg.DeleteKey(winreg.HKEY_CURRENT_USER, LINKS + sub)
        except OSError:
            pass


def setting(path, name):
    if not path.is_file():
        return None
    for line in path.read_text(encoding="utf-8").splitlines():
        key, _, value = line.partition("=")
        if key == name:
            return value
    return None


def command_for(program):
    return f'"{program}" "%1"'


def links_on(folder, command):
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "settings.ini").write_text(f"links=1\nlinks_command={command}\n", encoding="utf-8", newline="\n")
    take_links(command)


def wait_until(condition, seconds):
    end = time.time() + seconds
    while time.time() < end:
        if condition():
            return True
        time.sleep(0.5)
    return bool(condition())


def show_log(path, only=None):
    if not path.is_file():
        return
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    if only:
        lines = [line for line in lines if any(word in line for word in only)]
        print(f"--- what {path.name} says about closing programs:")
        told = " | ".join(line.split("   ", 1)[-1].strip() for line in lines)
        note(f"Setup, with the earlier program running: {told or 'its log says nothing about closing programs'}")
    else:
        lines = lines[-45:]
        print(f"--- the last lines of {path.name}:")
    for line in lines:
        print("    " + line)


def quiet_run(command, log, env=None):
    print("+ " + " ".join(str(part) for part in command), flush=True)
    try:
        code = subprocess.run([str(part) for part in command] + [f"/LOG={log}"], env=env, timeout=SETUP_WAIT).returncode
    except subprocess.TimeoutExpired:
        show_log(log)
        fail(f"{pathlib.Path(str(command[0])).name} did not finish within {SETUP_WAIT} seconds")
    if code != 0:
        show_log(log)
    return code


def program_env(profile):
    return dict(os.environ, APPDATA=str(profile), PARROTFISH_LINK_SCHEME=SCHEME, SLINT_BACKEND="winit-software")


def start_program(program, profile):
    process = subprocess.Popen([str(program)], env=program_env(profile), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    started.append(process)
    return process


def stop_program(process, what):
    if process.poll() is not None:
        return
    subprocess.run(["taskkill", "/PID", str(process.pid)], capture_output=True)
    closed = wait_until(lambda: process.poll() is not None, 20)
    check(closed, f"{what} closed when asked to")
    if not closed:
        process.kill()
        process.wait(timeout=20)


def uninstall(folder, profile, log):
    remover = folder / "unins000.exe"
    if not check(remover.is_file(), f"the uninstaller is in {folder}"):
        return
    quiet_run([remover, *REMOVE_QUIET], log, program_env(profile))
    finished = wait_until(lambda: uninstall_entry() is None and not remover.exists(), 90)
    time.sleep(1.0)
    check(finished, "the uninstaller finished within 90 seconds")


class Places:
    def __init__(self):
        self.menu = shell_folder(0x0002)
        self.desktop = shell_folder(0x0010)
        self.programs = shell_folder(0x001C) / "Programs"
        self.earlier_folder = self.programs / EARLIER
        self.new_folder = self.programs / NOW
        self.shortcuts = [place / f"{name}.lnk" for place in (self.menu, self.desktop) for name in (EARLIER, NOW)]

    def in_the_way(self):
        return [str(path) for path in (self.earlier_folder, self.new_folder, *self.shortcuts) if path.exists()]


def install_earlier(places, earlier_setup, log, desktop):
    command = [earlier_setup, *SETUP_QUIET] + (["/TASKS=desktopicon"] if desktop else [])
    code = quiet_run(command, log)
    check(code == 0, f"the earlier installer ended with code 0 (it was {code})")
    check((places.earlier_folder / f"{EARLIER}.exe").is_file(), f"{EARLIER}.exe is in {places.earlier_folder}")
    check((places.menu / f"{EARLIER}.lnk").is_file(), f"its Start menu shortcut is in {places.menu}")
    check((places.desktop / f"{EARLIER}.lnk").is_file() == desktop, f"its desktop shortcut is {'there' if desktop else 'not there'}")
    entry = uninstall_entry() or {}
    check(entry.get("DisplayName") == EARLIER, f"Windows lists it as {EARLIER} (it says {entry.get('DisplayName')!r})")


def upgraded(places, version, desktop):
    folder = places.earlier_folder
    check((folder / f"{NOW}.exe").is_file(), f"{NOW}.exe is in the folder the program was already in")
    check(not (folder / f"{EARLIER}.exe").exists(), f"{EARLIER}.exe is gone from it")
    check(not places.new_folder.exists(), "no second program folder was made")
    check((places.menu / f"{NOW}.lnk").is_file(), f"the Start menu has a {NOW} shortcut")
    check(not (places.menu / f"{EARLIER}.lnk").exists(), f"the {EARLIER} shortcut is gone from the Start menu")
    check((places.desktop / f"{NOW}.lnk").is_file() == desktop, f"the desktop {'has' if desktop else 'has no'} {NOW} shortcut, as before")
    check(not (places.desktop / f"{EARLIER}.lnk").exists(), f"no {EARLIER} shortcut is on the desktop")
    entry = uninstall_entry() or {}
    check(entry.get("DisplayName") == NOW, f"Windows lists it as {NOW} (it says {entry.get('DisplayName')!r})")
    check(entry.get("DisplayVersion") == version, f"with version {version} (it says {entry.get('DisplayVersion')!r})")
    location = (entry.get("InstallLocation") or "").rstrip("\\").lower()
    check(location == str(folder).lower(), "and in the same folder as before")


def removed(places, folder):
    left = sorted(path.name for path in folder.iterdir()) if folder.exists() else []
    check(not left, f"nothing is left in the program folder (left: {left})")
    check(not any(path.exists() for path in places.shortcuts), "no shortcut is left")
    check(uninstall_entry() is None, "Windows no longer lists it")
    check(links_command() is None and not links_key_exists(), "the links entry is gone")


def upgrade_while_running(places, earlier_setup, setup, version, work):
    print("== an upgrade while the earlier program runs, then the first start of the new one")
    profile = work / "appdata-a"
    install_earlier(places, earlier_setup, work / "a-earlier-install.log", desktop=True)
    earlier_program = places.earlier_folder / f"{EARLIER}.exe"
    earlier_command = command_for(earlier_program)
    links_on(profile / EARLIER, earlier_command)
    (profile / EARLIER / "bookmarks.ini").write_text(BOOKMARKS, encoding="utf-8", newline="\n")

    elsewhere = work / "appdata-running"
    running = start_program(earlier_program, elsewhere)
    up = wait_until(lambda: (elsewhere / NOW / "instance").is_file() or running.poll() is not None, START_WAIT)
    if not check(up and running.poll() is None, "the program starts on this machine and listens for a second start"):
        print(f"    it ended with code {running.poll()}" if running.poll() is not None else "    it wrote no note within the wait")

    log = work / "a-upgrade.log"
    code = quiet_run([setup, *SETUP_QUIET], log)
    check(code == 0, f"the new installer ended with code 0 (it was {code})")
    show_log(log, only=("RestartManager", "Restart Manager", "Shutting down", "applications"))
    closed = wait_until(lambda: running.poll() is not None, 30)
    check(closed, f"Setup closed the {EARLIER} that was running")
    if not closed:
        running.kill()
        running.wait(timeout=20)
        time.sleep(1.0)
    upgraded(places, version, desktop=True)
    check(links_command() == earlier_command, "the links entry was left for the program to look after")

    program = places.earlier_folder / f"{NOW}.exe"
    if not program.is_file():
        return
    first = start_program(program, profile)
    moved = wait_until(lambda: (profile / NOW / "instance").is_file() or first.poll() is not None, START_WAIT)
    check(moved and first.poll() is None, f"the new program started and listens in the folder named {NOW}")
    check(not (profile / EARLIER).exists(), f"the settings folder named {EARLIER} is gone")
    carried = profile / NOW / "bookmarks.ini"
    check(carried.is_file() and carried.read_text(encoding="utf-8") == BOOKMARKS, "the bookmarks came along unchanged")
    command = command_for(program)
    check(wait_until(lambda: links_command() == command, 20), f"the links entry points at {NOW}.exe")
    written = wait_until(lambda: setting(profile / NOW / "settings.ini", "links_command") == command, 20)
    check(written, "the settings file says so too, while the program is still running")
    if first.poll() is None:
        second = start_program(program, profile)
        left = wait_until(lambda: second.poll() is not None, 30)
        check(left and second.returncode == 0, "a second start hands over to the first and leaves")
        check(first.poll() is None, "and the first one keeps running")
        if not left:
            second.kill()
    stop_program(first, f"{NOW}")

    uninstall(places.earlier_folder, profile, work / "a-uninstall.log")
    removed(places, places.earlier_folder)
    check(setting(profile / NOW / "settings.ini", "links") == "0", "and the settings say links are off")
    drop_links()


def upgrade_never_started(places, earlier_setup, setup, version, work):
    print("== an upgrade that is removed before the new program ever ran")
    profile = work / "appdata-b"
    install_earlier(places, earlier_setup, work / "b-earlier-install.log", desktop=False)
    earlier_command = command_for(places.earlier_folder / f"{EARLIER}.exe")
    links_on(profile / EARLIER, earlier_command)
    code = quiet_run([setup, *SETUP_QUIET], work / "b-upgrade.log")
    check(code == 0, f"the new installer ended with code 0 (it was {code})")
    upgraded(places, version, desktop=False)
    uninstall(places.earlier_folder, profile, work / "b-uninstall.log")
    removed(places, places.earlier_folder)
    check(setting(profile / EARLIER / "settings.ini", "links") == "0", "the settings in the earlier folder say links are off")
    check(not (profile / NOW).exists(), "removing the program moved no settings folder")
    drop_links()


def first_install(places, setup, work):
    print("== the new installer on a PC that never had the program, with shortcuts somebody made")
    profile = work / "appdata-c"
    own = [places.menu / f"{EARLIER}.lnk", places.desktop / f"{EARLIER}.lnk"]
    for path in own:
        path.write_bytes(STAND_IN)
    try:
        code = quiet_run([setup, *SETUP_QUIET], work / "c-install.log")
        check(code == 0, f"the new installer ended with code 0 (it was {code})")
        program = places.new_folder / f"{NOW}.exe"
        check(program.is_file(), f"{NOW}.exe is in {places.new_folder}")
        check(not places.earlier_folder.exists(), f"nothing was put under the name {EARLIER}")
        check((places.menu / f"{NOW}.lnk").is_file(), f"the Start menu has a {NOW} shortcut")
        check(not (places.desktop / f"{NOW}.lnk").exists(), "no desktop shortcut was made without being asked for")
        kept = all(path.is_file() and path.read_bytes() == STAND_IN for path in own)
        check(kept, f"shortcuts named {EARLIER} that the installer never made were left alone")
        entry = uninstall_entry() or {}
        check(entry.get("DisplayName") == NOW, f"Windows lists it as {NOW} (it says {entry.get('DisplayName')!r})")
        if program.is_file():
            links_on(profile / NOW, command_for(program))
            uninstall(places.new_folder, profile, work / "c-uninstall.log")
            check(not places.new_folder.exists(), "the program folder is gone")
            check(not (places.menu / f"{NOW}.lnk").exists(), f"the {NOW} shortcut is gone")
            check(uninstall_entry() is None, "Windows no longer lists it")
            check(links_command() is None and not links_key_exists(), "the links entry is gone")
            check(setting(profile / NOW / "settings.ini", "links") == "0", "and the settings say links are off")
            still = all(path.is_file() and path.read_bytes() == STAND_IN for path in own)
            check(still, "and the shortcuts somebody made are still there")
    finally:
        for path in own:
            if path.is_file() and path.read_bytes() == STAND_IN:
                path.unlink()
        drop_links()


def main():
    parser = argparse.ArgumentParser(
        description="Install the earlier PhishSpeak, put the new installer over it, start and remove it, and install fresh. For a build machine."
    )
    parser.add_argument("--setup", help="the new installer; dist/Parrotfish-<version>-setup.exe if not given")
    parser.add_argument("--program", help="the built program that the earlier installer is made from")
    parser.add_argument("--iscc", help="path to Inno Setup's ISCC.exe")
    parser.add_argument("--this-pc", action="store_true", help="run although this is not a GitHub build machine")
    args = parser.parse_args()

    if not on_github() and not args.this_pc:
        fail("this installs and removes the program for the current user; it only runs on a build machine, or with --this-pc")
    version = workspace_version()
    setup = pathlib.Path(args.setup) if args.setup else DIST / f"{NOW}-{version}-setup.exe"
    program = pathlib.Path(args.program) if args.program else TARGET / "release" / "ps-app.exe"
    iscc = find_iscc(args.iscc)
    for needed, name in ((setup, "the new installer"), (program, "the built program"), (EARLIER_SCRIPT, "the earlier installer script")):
        if not needed.is_file():
            fail(f"{name} is missing: {needed}")
    if iscc is None:
        fail("Inno Setup 6 was not found; pass --iscc")

    places = Places()
    print(f"Start menu: {places.menu}\ndesktop: {places.desktop}\nprograms: {places.programs}")
    if uninstall_entry() is not None or places.in_the_way() or links_key_exists():
        fail(f"the program or this test's leftovers are already here, so nothing was touched: {places.in_the_way() or 'an entry in the registry'}")

    work = pathlib.Path(tempfile.mkdtemp(prefix="parrotfish-installer-test-"))
    try:
        print("== the installer from before the rename, made from the program built now")
        notices = work / "THIRD-PARTY-NOTICES.txt"
        notices.write_text("stand-in for the test\n", encoding="utf-8")
        made = work / "earlier"
        code = run_logged(
            [
                iscc,
                "/Qp",
                f"/DAppVersion={EARLIER_VERSION}",
                f"/DSourceExe={program}",
                f"/DNoticesFile={notices}",
                f"/DReadmeFile={ROOT / 'README.md'}",
                f"/DOutputDir={made}",
                EARLIER_SCRIPT,
            ],
            cwd=ROOT,
        )
        earlier_setup = made / f"{EARLIER}-{EARLIER_VERSION}-setup.exe"
        if code != 0 or not earlier_setup.is_file():
            fail(f"Inno Setup did not make the earlier installer (exit code {code})")

        counts = []
        for case in (
            lambda: upgrade_while_running(places, earlier_setup, setup, version, work),
            lambda: upgrade_never_started(places, earlier_setup, setup, version, work),
            lambda: first_install(places, setup, work),
        ):
            before = checked
            case()
            counts.append(checked - before)
    finally:
        for process in started:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=20)
        if uninstall_entry() is not None:
            for folder in (places.earlier_folder, places.new_folder):
                if (folder / "unins000.exe").is_file():
                    subprocess.run([str(folder / "unins000.exe"), *REMOVE_QUIET], env=program_env(work / "appdata-left"), timeout=SETUP_WAIT)
            wait_until(lambda: uninstall_entry() is None, 60)
        drop_links()
        if problems:
            for log in sorted(work.glob("*.log")):
                show_log(log)
        shutil.rmtree(work, ignore_errors=True)

    if problems:
        for problem in problems[:9]:
            annotate(f"installer test: {problem}")
        fail(f"{len(problems)} of {checked} installer checks went wrong")
    note(
        f"installer test passed: {checked} checks ({counts[0]} for an upgrade while the earlier program runs and the first start, "
        f"{counts[1]} for an upgrade removed before a first start, {counts[2]} for a first install)"
    )


if __name__ == "__main__":
    main()
