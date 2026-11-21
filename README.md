# LayoutFix

Fixes words typed in the wrong keyboard layout on macOS (`ghbdtn` → `привет`, `руддщ` → `hello`, `ghbdsn` → `привіт`). EN ↔ RU / UA, fully offline.

- `./install.sh` builds the app, signs it with a local certificate (so permissions survive rebuilds), installs it to `/Applications/LayoutFix.app` and sets it to start at login.
- `./uninstall.sh` removes it.
- Needs Accessibility and Input Monitoring (System Settings → Privacy & Security).

How it works: event tap → keycodes of the current word → on space, read the keys in every enabled layout → macOS spell checker (local) → if the typed text is not a word but another layout gives one: erase, switch layout, retype.

- RU vs UA: whichever dictionary has the word; if both, the last Cyrillic layout you used.
- ⌫ within 0.7 s of a fix reverts it; that word is then left alone for 10 minutes. A later ⌫ is ordinary editing.
- A 1–2 letter word is fixed only together with the next word (`ьн зкщидуьы` → `my problems`).
- Fixed on space and on Enter (Enter is held until the fix is typed). Trailing punctuation stays as typed: `ghbdtn,` → `привет,`, `руддщб` → `hello,`.
- Not fixed: words with digits, password fields (macOS Secure Input hides keys from every app, also in Ghostty when a prompt hides input).
- Typed text is never written to disk, logged or sent anywhere. No third-party dependencies.

Status log (permissions only): `~/Library/Logs/layoutfix.log`.

Checks:
- `cargo test` — decision logic.
- `target/release/layoutfix --dump ghbdtn` — how those keys read in each layout and what would be fixed.
- `tests/e2e.py [delay]` — types 26 real cases into a separate Ghostty window and compares (build `tests/bin` first: `swiftc -O tests/typer.swift -o tests/bin/typer && swiftc -O tests/layout.swift -o tests/bin/layout`).
