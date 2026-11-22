#!/usr/bin/env python3
"""Human-like stress test of the installed LayoutFix: bursts, pauses mid-word, key rollover,
space right after the last letter. Types into a separate Ghostty window running `cat` (checked by pid
before every line) and compares. Usage: tests/human.py [seeds] [cat|readline|node]  (readline/node: raw-mode line editors like a shell or Claude Code)"""
import os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from e2e import L, codes, osa, front_pid  # noqa: E402

SEEDS = int(sys.argv[1]) if len(sys.argv) > 1 else 6
MODE = sys.argv[2] if len(sys.argv) > 2 else "cat"
PHRASES = [
    ("en", "ghbdtn world rfr are you ltkf okay yjhvfkmyj thanks cgfcb,j lzre.",
     "привет world как are you дела okay нормально thanks спасибо дякую"),
    ("ru", "how are you doing ghbdtn my problems ckjdj",
     "how are you doing привет my problems слово"),
    ("en", "ghbdtn rfr ltkf ye;yj gjvjxm", "привет как дела нужно помочь"),
    ("en", "z levf. xnj dct hf,jnftn", "я думаю что все работает"),
]


def main():
    out = f"{HERE}/bin/human-out.txt"
    if os.path.exists(out):
        os.remove(out)
    before = set(subprocess.run(["pgrep", "-x", "ghostty"], capture_output=True, text=True).stdout.split())
    cmd = {
        "cat": f"cat > {out}",
        "readline": f"/bin/bash {HERE}/sink-readline.sh {out}",
        "node": f"node {HERE}/rawline.js {out}",
    }[MODE]
    subprocess.run(["open", "-na", "Ghostty", "--args", f"--command=/bin/bash -c '{cmd}'",
                    "--quit-after-last-window-closed=true"])
    time.sleep(4)
    new = set(subprocess.run(["pgrep", "-x", "ghostty"], capture_output=True, text=True).stdout.split()) - before
    if len(new) != 1:
        sys.exit("could not open the test window")
    pid = new.pop()
    osa(f'tell application "System Events" to set frontmost of (first process whose unix id is {pid}) to true')
    time.sleep(1)
    cases = [(p, s) for s in range(1, SEEDS + 1) for p in PHRASES]
    try:
        for (layout, keys, _), seed in cases:
            if front_pid() != pid:
                sys.exit("test window lost focus, stopped")
            subprocess.run([f"{HERE}/bin/layout", L[layout]])
            time.sleep(0.4)
            if front_pid() != pid:
                sys.exit("test window lost focus, stopped")
            subprocess.run([f"{HERE}/bin/human", str(seed), codes(keys + " \n")])
            time.sleep(0.6)
        time.sleep(1)
    finally:
        subprocess.run(["kill", pid])
        subprocess.run([f"{HERE}/bin/layout", L["en"]])
    got = open(out).read().split("\n") if os.path.exists(out) else []
    fails = 0
    for i, ((layout, keys, want), seed) in enumerate(cases):
        line = got[i].rstrip(" ") if i < len(got) else "<missing>"
        ok = line == want
        fails += not ok
        print(f"{'✅' if ok else '❌'} seed={seed} [{layout}] → {line!r}" + ("" if ok else f"\n      want {want!r}"))
    print(f"\n{len(cases) - fails}/{len(cases)} passed")
    sys.exit(1 if fails else 0)


main()
