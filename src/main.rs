//! layoutfix — fixes words typed in the wrong keyboard layout (ghbdtn → привет), fully offline.
//! Event tap → keycode buffer → macOS spell checker → backspaces + retype + layout switch.
//! Typed text is never stored, logged or sent anywhere; it lives in memory until the next space.

mod decide;

use decide::{decide, Key, Layout};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::ffi::c_void;
use std::ptr::null;

type Id = *const c_void; // CF / ObjC object
type Ev = *mut c_void; // CGEventRef

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: Id;
    fn CFRelease(o: Id);
    fn CFArrayGetCount(a: Id) -> isize;
    fn CFArrayGetValueAtIndex(a: Id, i: isize) -> Id;
    fn CFDataGetBytePtr(d: Id) -> *const u8;
    fn CFStringCreateWithBytes(al: Id, b: *const u8, n: isize, enc: u32, bom: bool) -> Id;
    fn CFStringGetCString(s: Id, buf: *mut u8, n: isize, enc: u32) -> bool;
    fn CFEqual(a: Id, b: Id) -> bool;
    fn CFBooleanGetValue(b: Id) -> bool;
    fn CFMachPortCreateRunLoopSource(al: Id, port: Id, order: isize) -> Id;
    fn CFRunLoopGetCurrent() -> Id;
    fn CFRunLoopAddSource(rl: Id, src: Id, mode: Id);
    fn CFRunLoopRun();
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, opts: u32, mask: u64, cb: TapCb, user: *mut c_void) -> Id;
    fn CGEventTapEnable(tap: Id, on: bool);
    fn CGEventTapPostEvent(proxy: Id, e: Ev);
    fn CGEventGetIntegerValueField(e: Ev, f: u32) -> i64;
    fn CGEventGetFlags(e: Ev) -> u64;
    fn CGEventSetFlags(e: Ev, f: u64);
    fn CGEventCreateKeyboardEvent(src: Id, key: u16, down: bool) -> Ev;
    fn CGEventCreateCopy(e: Ev) -> Ev;
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventKeyboardSetUnicodeString(e: Ev, n: usize, s: *const u16);
    fn CGRequestPostEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGPreflightListenEventAccess() -> bool;
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    static kTISPropertyInputSourceID: Id;
    static kTISPropertyInputSourceType: Id;
    static kTISPropertyInputSourceIsSelectCapable: Id;
    static kTISPropertyInputSourceLanguages: Id;
    static kTISPropertyUnicodeKeyLayoutData: Id;
    static kTISTypeKeyboardLayout: Id;
    fn TISCreateInputSourceList(props: Id, all: bool) -> Id;
    fn TISCopyCurrentKeyboardInputSource() -> Id;
    fn TISGetInputSourceProperty(src: Id, key: Id) -> Id;
    fn TISSelectInputSource(src: Id) -> i32;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const u8, key: u16, action: u16, mods: u32, kbd: u32, opts: u32,
        dead: *mut u32, max: usize, len: *mut usize, buf: *mut u16,
    ) -> i32;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}
#[link(name = "objc")]
extern "C" {
    fn objc_getClass(n: *const u8) -> Id;
    fn sel_registerName(n: *const u8) -> Id;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(p: *mut c_void);
}

type TapCb = extern "C" fn(Id, u32, Ev, *mut c_void) -> Ev;

const UTF8: u32 = 0x0800_0100;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_USER: u32 = 0xFFFF_FFFF;
const F_KEYCODE: u32 = 9;
const F_AUTOREPEAT: u32 = 8;
const F_TARGET_PID: u32 = 40;
const SHIFT: u64 = 0x20000;
const CAPS: u64 = 0x10000;
const CMD_CTRL_ALT: u64 = 0x100000 | 0x40000 | 0x80000;
const K_SPACE: u16 = 49;
const K_RETURN: u16 = 36;
const K_ENTER: u16 = 76;
const K_BACKSPACE: u16 = 51;

fn cfstr(s: &str) -> Id {
    unsafe { CFStringCreateWithBytes(null(), s.as_ptr(), s.len() as isize, UTF8, false) }
}

fn rstr(s: Id) -> String {
    let mut buf = [0u8; 256];
    if s.is_null() || !unsafe { CFStringGetCString(s, buf.as_mut_ptr(), 256, UTF8) } {
        return String::new();
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[repr(C)]
struct Range {
    loc: usize,
    len: usize,
}

/// macOS spell checker (local AppleSpell service), cached per word.
struct Speller {
    checker: Id,
    sel: Id,
    guess_sel: Id,
    cache: HashMap<(String, String), bool>,
    guess_cache: HashMap<(String, String), Vec<String>>,
}

impl Speller {
    fn new() -> Self {
        unsafe {
            let cls = objc_getClass(b"NSSpellChecker\0".as_ptr());
            let shared: extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let checker = shared(cls, sel_registerName(b"sharedSpellChecker\0".as_ptr()));
            let sel = sel_registerName(
                b"checkSpellingOfString:startingAt:language:wrap:inSpellDocumentWithTag:wordCount:\0".as_ptr(),
            );
            let guess_sel = sel_registerName(b"guessesForWordRange:inString:language:inSpellDocumentWithTag:\0".as_ptr());
            Speller { checker, sel, guess_sel, cache: HashMap::new(), guess_cache: HashMap::new() }
        }
    }

    fn valid(&mut self, word: &str, lang: &str) -> bool {
        let key = (word.to_lowercase(), lang.to_string());
        if let Some(&v) = self.cache.get(&key) {
            return v;
        }
        let ok = unsafe {
            let check: extern "C" fn(Id, Id, Id, isize, Id, bool, isize, *mut isize) -> Range =
                std::mem::transmute(objc_msgSend as *const ());
            let pool = objc_autoreleasePoolPush();
            let (w, l) = (cfstr(word), cfstr(lang));
            let r = check(self.checker, self.sel, w, 0, l, false, 0, std::ptr::null_mut());
            CFRelease(w);
            CFRelease(l);
            objc_autoreleasePoolPop(pool);
            r.loc == isize::MAX as usize // NSNotFound: no misspelling found
        };
        if self.cache.len() > 50_000 {
            self.cache.clear();
        }
        self.cache.insert(key, ok);
        ok
    }

    /// Spell checker suggestions, lowercased.
    fn guesses(&mut self, word: &str, lang: &str) -> &[String] {
        let key = (word.to_lowercase(), lang.to_string());
        if !self.guess_cache.contains_key(&key) {
            let list = unsafe {
                let guesses: extern "C" fn(Id, Id, Range, Id, Id, isize) -> Id = std::mem::transmute(objc_msgSend as *const ());
                let pool = objc_autoreleasePoolPush();
                let (w, l) = (cfstr(word), cfstr(lang));
                let arr = guesses(self.checker, self.guess_sel, Range { loc: 0, len: word.encode_utf16().count() }, w, l, 0);
                let n = if arr.is_null() { 0 } else { CFArrayGetCount(arr) };
                let list: Vec<String> = (0..n).map(|i| rstr(CFArrayGetValueAtIndex(arr, i)).to_lowercase()).collect();
                CFRelease(w);
                CFRelease(l);
                objc_autoreleasePoolPop(pool);
                list
            };
            if self.guess_cache.len() > 50_000 {
                self.guess_cache.clear();
            }
            self.guess_cache.insert(key.clone(), list);
        }
        &self.guess_cache[&key]
    }

    /// far: not a word and nothing to suggest. Otherwise: a word, or one edit from a suggestion.
    fn typo_check(&mut self, word: &str, lang: &str, far: bool) -> bool {
        if self.valid(word, lang) {
            return !far;
        }
        let w = word.to_lowercase();
        let g = self.guesses(word, lang);
        if far { g.is_empty() } else { g.iter().any(|g| decide::within_one(&w, g)) }
    }
}

/// What the user taught us, kept across restarts: key sequences they retyped in another layout (fixed
/// from now on) and words they reverted (never fixed). Only salted hashes are written, never the text.
/// shortcut: salt sits in the same file, so a short word can be brute-forced by whoever reads it (0600, own account only).
struct Memory {
    path: PathBuf,
    salt: u64,
    learned: HashMap<u64, String>, // keys hash → layout id
    ignored: HashSet<u64>,         // typed word hash
}

impl Memory {
    fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = PathBuf::from(home).join("Library/Application Support/LayoutFix/memory");
        let mut m = Memory { path, salt: 0, learned: HashMap::new(), ignored: HashSet::new() };
        for line in std::fs::read_to_string(&m.path).unwrap_or_default().lines() {
            let p: Vec<&str> = line.splitn(3, ' ').collect();
            let h = p.get(1).and_then(|h| u64::from_str_radix(h, 16).ok()).unwrap_or(0);
            match (p[0], p.get(2)) {
                ("salt", _) => m.salt = h,
                ("learn", Some(id)) => drop(m.learned.insert(h, id.to_string())),
                ("ignore", _) => drop(m.ignored.insert(h)),
                _ => {}
            }
        }
        if m.salt == 0 {
            let mut b = [0u8; 8];
            use std::io::Read;
            let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
            m.salt = u64::from_le_bytes(b) | 1;
        }
        m
    }

    fn save(&self) {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let mut s = format!("salt {:x}\n", self.salt);
        self.learned.iter().for_each(|(h, id)| s += &format!("learn {h:x} {id}\n"));
        self.ignored.iter().for_each(|h| s += &format!("ignore {h:x}\n"));
        let dir = self.path.parent().unwrap();
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
        let r = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&self.path)
            .and_then(|mut f| f.write_all(s.as_bytes()));
        if let Err(e) = r {
            eprintln!("layoutfix: cannot save memory: {e}");
        }
    }

    fn hash(&self, kind: &str, s: &str) -> u64 {
        // FNV-1a: stable across runs, unlike std's RandomState
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in self.salt.to_le_bytes().iter().chain(kind.as_bytes()).chain(s.as_bytes()) {
            h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
        }
        h
    }

    fn word(&self, w: &str) -> u64 {
        self.hash("w", &w.to_lowercase())
    }

    fn keys(&self, keys: &[Key]) -> u64 {
        self.hash("k", &keys.iter().map(|k| k.0.to_string()).collect::<Vec<_>>().join(","))
    }
}

struct Undo {
    at: Instant,
    keys: Vec<Key>,
    original: String, // text to put back (may include a one-letter word before)
    word: String,     // the word itself: never fixed again after a revert
    word_keys: u64,   // its keys hash: forgotten if it was learned
    from: usize,
}

/// A non-word left as typed: if the user erases it and types the same keys in the other script, learn that.
struct Retype {
    keys: Vec<Key>,
    layout: usize,
    erased: usize,
    at: Instant,
}

/// A fix waiting for the word's keys to be released: a key retyped while it is still physically held
/// is dropped by apps ("ns" + quick space gave "т" instead of "ты").
struct Pending {
    erase: usize,
    keys: Vec<Key>,
    text: String,
    to: usize,
    boundary: Ev,         // copy of the swallowed space / Return key-down, posted after the fix
    boundary_up: Ev,      // its key-up if it came while waiting (null otherwise)
    undo: Option<Undo>,
    since: Instant,
}

struct State {
    layouts: Vec<Layout>,
    sources: Vec<Id>, // TISInputSourceRef per layout; the list holding them is never freed
    speller: Speller,
    keys: Vec<Key>,
    dirty: bool, // caret moved or an unknown key was pressed: skip the current word
    prev: Option<(Vec<Key>, usize)>, // one-letter word right before the current one, with its layout
    last_cyrillic: Option<usize>,
    undo: Option<Undo>,
    memory: Memory,
    retype: Option<Retype>,
    tap: Id,
    pid: i64, // app that received the previous key
    last_key: Instant,
    debug: bool, // ~/Library/Logs/layoutfix.debug exists: log event classes, counts and timings (never characters)
    pending: Option<Pending>,
    word_layout: Option<usize>, // layout the current word was started in
}

fn load_layouts() -> (Vec<Layout>, Vec<Id>) {
    let (mut layouts, mut sources) = (Vec::new(), Vec::new());
    unsafe {
        let list = TISCreateInputSourceList(null(), false);
        let kbd = LMGetKbdType() as u32;
        for i in 0..CFArrayGetCount(list) {
            let src = CFArrayGetValueAtIndex(list, i);
            let ty = TISGetInputSourceProperty(src, kTISPropertyInputSourceType);
            let selectable = TISGetInputSourceProperty(src, kTISPropertyInputSourceIsSelectCapable);
            let data = TISGetInputSourceProperty(src, kTISPropertyUnicodeKeyLayoutData);
            if ty.is_null() || !CFEqual(ty, kTISTypeKeyboardLayout) || data.is_null()
                || selectable.is_null() || !CFBooleanGetValue(selectable)
            {
                continue;
            }
            let langs = TISGetInputSourceProperty(src, kTISPropertyInputSourceLanguages);
            let lang = if langs.is_null() || CFArrayGetCount(langs) == 0 {
                String::new()
            } else {
                rstr(CFArrayGetValueAtIndex(langs, 0))
            };
            let lang = lang.split(['-', '_']).next().unwrap_or("").to_string();
            if !["en", "ru", "uk"].contains(&lang.as_str()) {
                continue;
            }
            let ptr = CFDataGetBytePtr(data);
            let mut table = vec![None; 256];
            for key in 0u16..128 {
                for shift in [false, true] {
                    let (mut dead, mut len, mut buf) = (0u32, 0usize, [0u16; 4]);
                    let mods = if shift { 2 } else { 0 }; // shiftKey >> 8
                    let st = UCKeyTranslate(ptr, key, 0, mods, kbd, 1, &mut dead, 4, &mut len, buf.as_mut_ptr());
                    if st == 0 && len == 1 {
                        table[key as usize * 2 + shift as usize] =
                            char::from_u32(buf[0] as u32).filter(|c| !c.is_control());
                    }
                }
            }
            layouts.push(Layout {
                id: rstr(TISGetInputSourceProperty(src, kTISPropertyInputSourceID)),
                cyrillic: lang != "en",
                lang,
                table,
            });
            sources.push(src);
        }
    }
    (layouts, sources)
}

/// Learned → dictionary → doubled capital → typo, in that order.
fn find_fix(st: &mut State, keys: &[Key], cur: usize) -> Option<(usize, String)> {
    let learned = st.memory.learned.get(&st.memory.keys(keys))
        .and_then(|id| st.layouts.iter().position(|l| &l.id == id))
        .filter(|&to| st.layouts[to].cyrillic != st.layouts[cur].cyrillic);
    if let Some(to) = learned {
        return decide::translate(keys, &st.layouts[to]).map(|t| (to, t));
    }
    let (layouts, last, sp) = (&st.layouts, st.last_cyrillic, &mut st.speller);
    if let Some(f) = decide(keys, layouts, cur, last, &mut |w, l| sp.valid(w, l)) {
        return Some(f);
    }
    if let Some(k2) = decide::fix_case(keys) {
        if let Some(f) = decide(&k2, layouts, cur, last, &mut |w, l| sp.valid(w, l)) {
            return Some(f);
        }
        // same layout: Cyrillic only, Latin has too many CamelCase names (OAuth, IPhone)
        let c = &layouts[cur];
        if let Some(t) = decide::translate(&k2, c).filter(|t| c.cyrillic && decide::word_ok(t, true) && sp.valid(t, &c.lang)) {
            return Some((cur, t));
        }
    }
    decide::typo(keys, layouts, cur, last, &mut |w, l, far| sp.typo_check(w, l, far))
}

fn current_layout(st: &State) -> Option<usize> {
    unsafe {
        let src = TISCopyCurrentKeyboardInputSource();
        let id = rstr(TISGetInputSourceProperty(src, kTISPropertyInputSourceID));
        CFRelease(src);
        st.layouts.iter().position(|l| l.id == id)
    }
}

fn post_key(proxy: Id, key: u16, ch: char, shift: bool) {
    unsafe {
        for down in [true, false] {
            let e = CGEventCreateKeyboardEvent(null(), key, down);
            CGEventSetFlags(e, if shift { SHIFT } else { 0 });
            if ch != '\u{8}' {
                let mut b = [0u16; 2];
                let s = ch.encode_utf16(&mut b);
                CGEventKeyboardSetUnicodeString(e, s.len(), s.as_ptr());
            }
            CGEventTapPostEvent(proxy, e);
            CFRelease(e);
        }
    }
}

/// Erase `erase` chars, switch layout, retype `text` on the same physical keys.
fn replace(st: &State, proxy: Id, erase: usize, keys: &[Key], text: &str, to: usize) {
    unsafe { TISSelectInputSource(st.sources[to]) };
    for _ in 0..erase {
        post_key(proxy, K_BACKSPACE, '\u{8}', false);
    }
    // Key + explicit string, so apps that read either get the same char. Letters land on the keys
    // that were typed; kept punctuation ("руддщ." → "hello.") moves to the key that types it in the new layout.
    let table = &st.layouts[to].table;
    for (&(k, s), c) in keys.iter().zip(text.chars()) {
        let (key, shift) = match table.get(k as usize * 2 + s as usize) {
            Some(&Some(t)) if t == c => (k, s),
            _ => table.iter().position(|t| *t == Some(c)).map_or((k, s), |i| ((i / 2) as u16, i % 2 == 1)),
        };
        post_key(proxy, key, c, shift);
    }
}

/// Physical key state from the HID system: authoritative even if a key-up was delivered elsewhere.
fn held(key: u16) -> bool {
    unsafe { CGEventSourceKeyState(1, key) }
}

/// Type a waiting fix now, then the space / Return it held back.
fn flush(st: &mut State, proxy: Id) {
    let Some(p) = st.pending.take() else { return };
    if st.debug {
        eprintln!("  flush after {}ms", p.since.elapsed().as_millis());
    }
    replace(st, proxy, p.erase, &p.keys, &p.text, p.to);
    unsafe {
        CGEventTapPostEvent(proxy, p.boundary);
        CFRelease(p.boundary);
        if !p.boundary_up.is_null() {
            CGEventTapPostEvent(proxy, p.boundary_up);
            CFRelease(p.boundary_up);
        }
    }
    st.undo = p.undo.map(|u| Undo { at: Instant::now(), ..u });
}

/// Drop the word in progress. `dirty`: the caret may now sit inside a word, so skip until the next space.
fn reset(st: &mut State, dirty: bool) {
    st.keys.clear();
    st.prev = None;
    st.dirty = dirty;
}

extern "C" fn on_event(proxy: Id, ty: u32, e: Ev, user: *mut c_void) -> Ev {
    let st = unsafe { &mut *(user as *mut State) };
    if ty == TAP_DISABLED_TIMEOUT || ty == TAP_DISABLED_USER {
        unsafe { CGEventTapEnable(st.tap, true) };
        return e;
    }
    if ty == KEY_UP {
        let key = unsafe { CGEventGetIntegerValueField(e, F_KEYCODE) } as u16;
        let Some(p) = &mut st.pending else { return e };
        if p.boundary_up.is_null() && unsafe { CGEventGetIntegerValueField(p.boundary, F_KEYCODE) } as u16 == key {
            p.boundary_up = unsafe { CGEventCreateCopy(e) }; // keep the space's key-up after its key-down
            return std::ptr::null_mut();
        }
        if p.keys.iter().any(|k| k.0 != key && held(k.0)) && p.since.elapsed() < Duration::from_millis(400) {
            return e;
        }
        // last word key released: let this key-up through first, then the fix
        unsafe {
            let up = CGEventCreateCopy(e);
            CGEventTapPostEvent(proxy, up);
            CFRelease(up);
        }
        flush(st, proxy);
        return std::ptr::null_mut();
    }
    // Anything else while a fix waits (next key, click, a key held too long): type the fix first.
    flush(st, proxy);
    let undo = st.undo.take();
    if ty != KEY_DOWN {
        reset(st, false);
        st.retype = None; // mouse click: usually a new place to type; we only ever erase what we saw typed
        if st.debug {
            eprintln!("mouse");
        }
        return e;
    }
    let key = unsafe { CGEventGetIntegerValueField(e, F_KEYCODE) } as u16;
    let flags = unsafe { CGEventGetFlags(e) };
    // Keys buffered for another app (focus moved without a click or ⌘Tab) or long ago are not this word.
    let pid = unsafe { CGEventGetIntegerValueField(e, F_TARGET_PID) };
    if pid != st.pid || st.last_key.elapsed() > Duration::from_secs(5) {
        reset(st, false);
    }
    if st.debug {
        let class = match key { K_SPACE => "space", K_RETURN | K_ENTER => "return", K_BACKSPACE => "bs", _ => "key" };
        eprintln!(
            "{class} dt={}ms buf={} mods={} repeat={} newapp={}",
            st.last_key.elapsed().as_millis(), st.keys.len(), flags & CMD_CTRL_ALT != 0,
            unsafe { CGEventGetIntegerValueField(e, F_AUTOREPEAT) }, pid != st.pid
        );
    }
    st.pid = pid;
    st.last_key = Instant::now();
    if key == K_BACKSPACE && st.keys.is_empty() {
        if let Some(r) = &mut st.retype {
            r.erased += if flags & CMD_CTRL_ALT != 0 { 1000 } else { 1 }; // ⌥⌫ / ⌘⌫ erase a whole word or more
        }
    }
    if flags & CMD_CTRL_ALT != 0 {
        // shortcuts, ⌘Tab: next word starts fresh; mid-word (⌃Space layout switch, ⌘V) the rest is not a whole word
        let mid = !st.keys.is_empty();
        reset(st, mid);
        return e;
    }
    if unsafe { CGEventGetIntegerValueField(e, F_AUTOREPEAT) } != 0 {
        // A held key repeats in terminals but not in apps with the accent popup: the count is unknown.
        reset(st, true);
        return e;
    }

    if key == K_BACKSPACE {
        // Only a reflex ⌫ right after a fix reverts it; a later ⌫ is ordinary editing.
        if let Some(u) = undo.filter(|u| u.at.elapsed() < Duration::from_millis(700)) {
            if st.debug {
                eprintln!("  undo erase={}", u.keys.len() + 1);
            }
            let w = st.memory.word(&u.word);
            st.memory.ignored.insert(w);
            st.memory.learned.remove(&u.word_keys);
            st.memory.save();
            replace(st, proxy, u.keys.len() + 1, &u.keys, &u.original, u.from);
            post_key(proxy, K_SPACE, ' ', false);
            reset(st, false);
            return std::ptr::null_mut();
        }
        if st.keys.pop().is_none() {
            st.dirty = true;
        }
        return e;
    }

    if key == K_SPACE || key == K_RETURN || key == K_ENTER {
        let keys = std::mem::take(&mut st.keys);
        let dirty = std::mem::replace(&mut st.dirty, false);
        let Some(cur) = current_layout(st) else { return e };
        if st.layouts[cur].cyrillic {
            st.last_cyrillic = Some(cur);
        }
        let prev = st.prev.take();
        if let Some(r) = st.retype.take() {
            let same = r.keys.len() == keys.len() && r.keys.iter().zip(&keys).all(|(a, b)| a.0 == b.0);
            if same && r.erased >= r.keys.len() && r.at.elapsed() < Duration::from_secs(30)
                && st.layouts[r.layout].cyrillic != st.layouts[cur].cyrillic
            {
                let h = st.memory.keys(&keys);
                st.memory.learned.insert(h, st.layouts[cur].id.clone());
                let old = decide::translate(&keys, &st.layouts[r.layout]).unwrap_or_default();
                let w = st.memory.word(&old);
                st.memory.ignored.remove(&w);
                st.memory.save();
                if st.debug {
                    eprintln!("  learned len={}", keys.len());
                }
            }
        }
        if dirty || keys.is_empty() {
            return e;
        }
        let typed = decide::translate(&keys, &st.layouts[cur]).unwrap_or_default();
        if st.memory.ignored.contains(&st.memory.word(&typed)) {
            return e;
        }
        let word_keys = st.memory.keys(&keys);
        if let Some((to, text)) = find_fix(st, &keys, cur) {
            // A one-letter word just before ("z levf." = "я думаю") is fixed along, unless it already is a word ("a", "I").
            let one = prev.filter(|(_, l)| *l == cur).and_then(|(pk, _)| {
                let orig = decide::translate(&pk, &st.layouts[cur])?;
                let t = decide::translate(&pk, &st.layouts[to])?;
                let ok = !decide::short_word(&orig, &st.layouts[cur].lang) && decide::short_word(&t, &st.layouts[to].lang);
                ok.then_some((pk, orig, t))
            });
            let (keys, original, text) = match one {
                Some((pk, orig, t)) => ([pk, vec![(K_SPACE, false)], keys].concat(), format!("{orig} {typed}"), format!("{t} {text}")),
                None => (keys, typed.clone(), text),
            };
            if st.debug {
                eprintln!("  fix erase={} type={} {}→{}", keys.len(), text.chars().count(), st.layouts[cur].lang, st.layouts[to].lang);
            }
            if st.layouts[to].cyrillic {
                st.last_cyrillic = Some(to);
            }
            let undo = (key == K_SPACE).then(|| Undo { at: Instant::now(), keys: keys.clone(), original, word: typed, word_keys, from: cur });
            if keys.iter().any(|k| held(k.0)) {
                st.pending = Some(Pending {
                    erase: keys.len(), keys, text, to,
                    boundary: unsafe { CGEventCreateCopy(e) },
                    boundary_up: std::ptr::null_mut(),
                    undo, since: Instant::now(),
                });
                return std::ptr::null_mut();
            }
            replace(st, proxy, keys.len(), &keys, &text, to);
            st.undo = undo;
            // Posted events enter the stream before the event returned here, so the user's own
            // space / Return / ⇧Return follows the fix untouched.
            return e;
        }
        if st.debug {
            eprintln!("  nofix len={} cur={}", keys.len(), st.layouts[cur].lang);
        }
        let c = &st.layouts[cur];
        if keys.len() >= 2 && !(decide::word_ok(&typed, c.cyrillic) && st.speller.valid(&typed, &c.lang)) {
            st.retype = Some(Retype { keys: keys.clone(), layout: cur, erased: 0, at: Instant::now() });
        }
        if keys.len() == 1 && key == K_SPACE {
            st.prev = Some((keys, cur));
        }
        return e;
    }

    // A key that types a character continues the word (punctuation is sorted out by decide);
    // keys that type nothing (arrows, Home, F-keys) leave the caret somewhere unknown.
    let prints = key < 128 && st.layouts.iter().any(|l| l.table[key as usize * 2].is_some());
    // Caps Lock upper-cases letters only; a key that is punctuation in some layout stays unshifted
    let caps = flags & CAPS != 0
        && prints
        && st.layouts.iter().all(|l| l.table[key as usize * 2].is_some_and(char::is_alphabetic));
    let shift = flags & SHIFT != 0 || caps;
    if prints && st.keys.len() < 64 {
        // layout switched by hand mid-word (Globe, Caps Lock): the user is fixing it, leave the word alone
        let now = current_layout(st);
        if st.keys.is_empty() {
            st.word_layout = now;
        } else if now != st.word_layout {
            st.dirty = true;
        }
        st.keys.push((key, shift));
    } else {
        // Tab/Esc start a fresh word
        reset(st, !matches!(key, 48 | 53));
    }
    e
}

fn main() {
    let (layouts, sources) = load_layouts();
    if !layouts.iter().any(|l| l.cyrillic) || !layouts.iter().any(|l| !l.cyrillic) {
        eprintln!("layoutfix: enable an English and a Russian or Ukrainian keyboard layout");
        std::process::exit(1);
    }
    let last_cyrillic = layouts.iter().position(|l| l.cyrillic);
    if std::env::args().nth(1).as_deref() == Some("--dump") {
        // self-check: `--dump ghbdsn` = those keys read in every layout, and the decision when typed in each
        let word = std::env::args().nth(2).unwrap_or("ghbdsn".into());
        let en = layouts.iter().find(|l| !l.cyrillic).unwrap();
        let keys: Vec<Key> = word.chars()
            .filter_map(|c| en.table.iter().position(|t| *t == Some(c)).map(|i| ((i / 2) as u16, i % 2 == 1)))
            .collect();
        let mut sp = Speller::new();
        for (i, l) in layouts.iter().enumerate() {
            let t = decide::translate(&keys, l).unwrap_or_default();
            let d = decide(&keys, &layouts, i, last_cyrillic, &mut |w, lang| sp.valid(w, lang));
            let ty = decide::typo(&keys, &layouts, i, last_cyrillic, &mut |w, lang, far| sp.typo_check(w, lang, far));
            println!("{} lang={} → {} valid={} | typed here, fix to: {:?} typo: {:?}", l.id, l.lang, t, sp.valid(&t, &l.lang), d, ty);
        }
        return;
    }
    let st = Box::into_raw(Box::new(State {
        layouts,
        sources,
        speller: Speller::new(),
        keys: Vec::new(),
        dirty: false,
        prev: None,
        last_cyrillic,
        undo: None,
        memory: Memory::load(),
        retype: None,
        tap: null(),
        pid: 0,
        last_key: Instant::now(),
        debug: std::env::var("HOME").is_ok_and(|h| std::path::Path::new(&h).join("Library/Logs/layoutfix.debug").exists()),
        pending: None,
        word_layout: None,
    }));
    unsafe {
        CGRequestListenEventAccess();
        CGRequestPostEventAccess();
        eprintln!("layoutfix: input monitoring={} post events={}", CGPreflightListenEventAccess(), CGPreflightPostEventAccess());
        let mask = 1u64 << KEY_DOWN | 1 << KEY_UP | 1 << 1 | 1 << 3 | 1 << 25; // keys, mouse down (left/right/other)
        let tap = CGEventTapCreate(1, 0, 0, mask, on_event, st as *mut c_void);
        if tap.is_null() {
            // macOS caches the permission per process: exit so launchd restarts us with a fresh check
            eprintln!("layoutfix: no Accessibility permission yet (System Settings → Privacy & Security)");
            std::thread::sleep(Duration::from_secs(5));
            std::process::exit(1);
        }
        (*st).tap = tap;
        CFRunLoopAddSource(CFRunLoopGetCurrent(), CFMachPortCreateRunLoopSource(null(), tap, 0), kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRunLoopRun();
    }
}
