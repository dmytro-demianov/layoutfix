# LayoutFix

Fixes words typed in the wrong keyboard layout on macOS (`ghbdtn` → `привет`, `руддщ` → `hello`, `ghbdsn` → `привіт`). EN ↔ RU / UA, fully offline.

- `./install.sh` builds the app, signs it with a local certificate (so permissions survive rebuilds), installs it to `/Applications/LayoutFix.app` and sets it to start at login.
- `./uninstall.sh` removes it.
- Needs Accessibility and Input Monitoring (System Settings → Privacy & Security).

How it works: event tap → keycodes of the current word → on space, read the keys in every enabled layout → macOS spell checker (local) → if the typed text is not a word but another layout gives one: erase, switch layout, retype.

- RU vs UA: whichever dictionary has the word; if both, the last Cyrillic layout you used.
- ⌫ within 0.7 s of a fix reverts it; that word is then never fixed again. A later ⌫ is ordinary editing.
- Learns from you: erase a word it left alone and type the same keys in the other layout (within 30 s) — those keys are fixed from now on (slang, names, brands).
- Wrong layout plus a typo (5+ letters): fixed when the typed text gets no spell checker suggestions and another layout is one edit from a word (`ghbdtnn` → `приветт`, `cgfcb,jj` → `спасибоо`); the typo itself stays.
- Shift held too long (5+ letters): `GHbdtn` → `Привет`, `ПРивет` → `Привет` (Cyrillic only; Latin CamelCase like `OAuth` is left alone).
- Memory (learned keys, reverted words) lives in `~/Library/Application Support/LayoutFix/memory`, 0600, salted hashes only; delete the file to forget.
- 2-letter words are fixed only into a list of common ones (`ns` → `ты`, `ше` → `it`). A 1-letter word is fixed together with the next word (`z levf.` → `я думаю`), unless it already is a word (`a`, `I`).
- Left alone: tech tokens that happen to read as a Cyrillic word (`db`, `vs`, `ls`, `ctx`, `len`, `csv`… — `KEEP` in `src/decide.rs`), ALL-CAPS tokens up to 4 letters (`API`, `KB`).
- Fixed on space and on Enter; your own space / Enter / ⇧Enter follows the fix untouched. A key that is a letter in the target layout is read as that letter (`levf.` → `думаю`, `'nj` → `это`); otherwise punctuation stays as typed (`ghbdtn,` → `привет,`, `"цщгдв"` → `"would"`). Hyphens and apostrophes inside a word work (`rfr-nj` → `как-то`, `вщтэе` → `don't`).
- Not fixed: words with digits, a word in which a key was held until it auto-repeated, password fields (macOS Secure Input hides keys from every app, also in Ghostty when a prompt hides input).
- Typed text is never written to disk (only salted hashes in the memory file), logged or sent anywhere. No third-party dependencies.

Status log (permissions only): `~/Library/Logs/layoutfix.log`.

Checks:
- `cargo test` — decision logic.
- `target/release/layoutfix --dump ghbdtn` — how those keys read in each layout and what would be fixed.
- `tests/human.py [seeds]` — human-like rhythm (bursts, pauses mid-word, key rollover) on mixed-language phrases.
- `tests/e2e.py [delay]` — types 46 real cases into a separate Ghostty window and compares (build `tests/bin` first: `swiftc -O tests/typer.swift -o tests/bin/typer && swiftc -O tests/layout.swift -o tests/bin/layout`).
