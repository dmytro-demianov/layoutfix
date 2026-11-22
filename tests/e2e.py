#!/usr/bin/env python3
"""End-to-end check of the installed LayoutFix: types real key events into a separate Ghostty
window running `cat`, one case per line, and compares what the app received.
Every case is typed only if that window is frontmost, so keys never land in another app.
(No `stty -echo`: Ghostty then turns on Secure Keyboard Entry and no event tap sees keys.)
Usage: tests/e2e.py [delay_seconds]"""
import os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
TYPER, LAYOUT = f"{HERE}/bin/typer", f"{HERE}/bin/layout"
DELAY = sys.argv[1] if len(sys.argv) > 1 else "0.06"
L = {"en": "com.apple.keylayout.ABC", "ru": "com.apple.keylayout.Russian", "uk": "com.apple.keylayout.Ukrainian-PC"}

# US-keyboard character → (keycode, shift)
BASE = {"a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12,
        "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23,
        "=": 24, "9": 25, "7": 26, "-": 27, "8": 28, "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34,
        "p": 35, "l": 37, "j": 38, "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46,
        ".": 47, "`": 50, " ": 49, "\n": 36}
SHIFTED = {"@": "2", "(": "9", ")": "0", "&": "7", "?": "/", "!": "1", ":": ";", '"': "'", "<": ",", ">": "."}


def codes(keys):
    out = []
    for ch in keys:
        if ch in SHIFTED:
            out.append(f"{BASE[SHIFTED[ch]]}s")
        elif ch.isupper():
            out.append(f"{BASE[ch.lower()]}s")
        else:
            out.append(str(BASE[ch]))
    return ",".join(out)


# (layout active while typing, keys pressed as if on a US keyboard, expected text)
CASES = [
    # typed in English layout, meant Cyrillic
    ("en", "ghbdtn rfr ltkf", "привет как дела"),
    ("en", "yjhvfkmyj hf,jnftn", "нормально работает"),
    ("en", "ye;yj gjvjxm", "нужно помочь"),
    ("en", "ghbdtn, rfr ltkf?", "привет, как дела?"),
    ("en", "Ghbdtn", "Привет"),
    ("en", "ghbdsn", "привіт"),
    ("en", "ckfdf erhf]ys", "слава україні"),
    ("en", "ghbdtn world", "привет world"),
    ("en", "ghbdtn\n", "привет"),  # fixed before Enter
    ("en", "ghbdsn, zr cghfdb", "привіт, як справи"),
    ("en", "levf.", "думаю"),  # . key is ю
    ("en", "what ns ns ns ns", "what ты ты ты ты"),
    ("en", "lf yt pyf.", "да не знаю"),
    ("ru", "is it ok", "is it ok"),
    ("en", "git db vs ls", "git db vs ls"),
    ("en", "kj,", "лоб"),  # , key is б
    ("en", "ckjdj.", "слово."),
    ("en", '"ghbdtn"', '"привет"'),
    ("en", "(ghbdtn)", "(привет)"),
    ("en", '"ghbdtn"', '"привет"'),
    ("en", "(ghbdtn)", "(привет)"),
    # typed in Cyrillic layout, meant English
    ("ru", "hello world", "hello world"),
    ("ru", "my problems", "my problems"),
    ("ru", "@would@", '"would"'),  # quotes are ⇧2 in Apple Russian
    ("ru", "would", "would"),
    ("ru", "hello.", "hello."),
    ("ru", "hello,", "hello,"),
    ("ru", "hello&", "hello."),  # "." is ⇧7 in Apple Russian
    ("uk", "hello/", "hello."),  # "." is the / key in Ukrainian-PC
    ("ru", "How are you", "How are you"),
    ("uk", "hello there", "hello there"),
    ("ru", "how are you doing?", "how are you doing?"),
    # must stay as typed
    ("en", "hello world", "hello world"),
    ("ru", "ghbdtn rfr ltkf", "привет как дела"),
    ("en", "kubectl npm ls cd", "kubectl npm ls cd"),
    ("en", "API SQL", "API SQL"),
    ("en", "test1 123 abc", "test1 123 abc"),
    ("en", "z", "z"),
    ("en", "git status", "git status"),
]


def osa(script):
    return subprocess.run(["osascript", "-e", script], capture_output=True, text=True).stdout.strip()


def front_pid():
    return osa('tell application "System Events" to get unix id of first process whose frontmost is true')


def main():
    out = f"{HERE}/bin/e2e-out.txt"
    if os.path.exists(out):
        os.remove(out)
    before = set(subprocess.run(["pgrep", "-x", "ghostty"], capture_output=True, text=True).stdout.split())
    subprocess.run(["open", "-na", "Ghostty", "--args", f"--command=/bin/bash -c 'cat > {out}'",
                    "--quit-after-last-window-closed=true"])
    time.sleep(4)
    new = set(subprocess.run(["pgrep", "-x", "ghostty"], capture_output=True, text=True).stdout.split()) - before
    if len(new) != 1:
        sys.exit("could not open the test window")
    pid = new.pop()
    osa(f'tell application "System Events" to set frontmost of (first process whose unix id is {pid}) to true')
    time.sleep(1)
    try:
        for layout, keys, _ in CASES:
            if front_pid() != pid:
                sys.exit("test window lost focus, stopped")
            subprocess.run([LAYOUT, L[layout]])
            time.sleep(0.4)
            if front_pid() != pid:
                sys.exit("test window lost focus, stopped")
            # trailing space triggers the fix; a case ending in Enter tests the Enter path instead
            tail = "" if keys.endswith("\n") else " \n"
            subprocess.run([TYPER, DELAY, codes(keys + tail)])
            time.sleep(0.5)
        time.sleep(1)
    finally:
        subprocess.run(["kill", pid])
        subprocess.run([LAYOUT, L["en"]])
    got = open(out).read().split("\n") if os.path.exists(out) else []
    fails = 0
    for i, (layout, keys, want) in enumerate(CASES):
        line = got[i].rstrip(" ") if i < len(got) else "<missing>"
        ok = line == want
        fails += not ok
        print(f"{'✅' if ok else '❌'} [{layout}] {keys.strip()!r:28} → {line!r}" + ("" if ok else f"   want {want!r}"))
    print(f"\n{len(CASES) - fails}/{len(CASES)} passed")
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
