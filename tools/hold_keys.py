import ctypes
import sys
import time

ALLOWED = {f"F{number}": 0x7C + number - 13 for number in range(13, 25)}


def main():
    if len(sys.argv) != 3 or any(name not in ALLOWED for name in sys.argv[1].split("+")):
        print("usage: hold_keys.py F13..F24[+F13..F24] <seconds>")
        sys.exit(2)
    codes = [ALLOWED[name] for name in sys.argv[1].split("+")]
    user32 = ctypes.WinDLL("user32")
    for code in codes:
        user32.keybd_event(code, 0, 0, 0)
    time.sleep(float(sys.argv[2]))
    for code in reversed(codes):
        user32.keybd_event(code, 0, 2, 0)
    print(f"held {sys.argv[1]} for {sys.argv[2]} s")


main()
