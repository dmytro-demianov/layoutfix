//! layoutfix — fixes words typed in the wrong keyboard layout (ghbdtn → привет), fully offline.
//! Event tap → keycode buffer → macOS spell checker → backspaces + retype + layout switch.
//! Typed text is never stored, logged or sent anywhere; it lives in memory until the next space.

mod decide;

use decide::{decide, Key, Layout};
use std::collections::HashMap;
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
const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_USER: u32 = 0xFFFF_FFFF;
const F_KEYCODE: u32 = 9;
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

/// macOS spell checker (local AppleSpell service), cached per word.
struct Speller {
    checker: Id,
    sel: Id,
    cache: HashMap<(String, String), bool>,
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
            Speller { checker, sel, cache: HashMap::new() }
        }
    }

    fn valid(&mut self, word: &str, lang: &str) -> bool {
        let key = (word.to_lowercase(), lang.to_string());
        if let Some(&v) = self.cache.get(&key) {
            return v;
        }
        #[repr(C)]
        struct Range {
            loc: usize,
            len: usize,
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
}

struct Undo {
    at: Instant,
    end: (u16, char), // boundary that was re-posted: space or return
    keys: Vec<Key>,
    original: String,
    from: usize,
}

struct State {
    layouts: Vec<Layout>,
    sources: Vec<Id>, // TISInputSourceRef per layout; the list holding them is never freed
    speller: Speller,
    keys: Vec<Key>,
    dirty: bool, // caret moved or an unknown key was pressed: skip the current word
    prev: Option<(Vec<Key>, usize)>, // short word right before the current one, with its layout
    last_cyrillic: Option<usize>,
    undo: Option<Undo>,
    ignored: HashMap<String, Instant>, // words the user just reverted: left alone for a while
    tap: Id,
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

/// Erase `erase` chars, switch layout, retype `text` on the same physical keys, then a space.
fn replace(st: &State, proxy: Id, erase: usize, keys: &[Key], text: &str, to: usize, end: (u16, char)) {
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
    post_key(proxy, end.0, end.1, false);
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
    let undo = st.undo.take();
    if ty != KEY_DOWN {
        reset(st, false); // mouse click: usually a new place to type; we only ever erase what we saw typed
        return e;
    }
    let key = unsafe { CGEventGetIntegerValueField(e, F_KEYCODE) } as u16;
    let flags = unsafe { CGEventGetFlags(e) };
    if flags & CMD_CTRL_ALT != 0 {
        reset(st, false); // shortcuts, ⌘Tab: next word starts fresh
        return e;
    }

    if key == K_BACKSPACE {
        // Only a reflex ⌫ right after a fix reverts it; a later ⌫ is ordinary editing.
        if let Some(u) = undo.filter(|u| u.at.elapsed() < Duration::from_millis(700)) {
            // Backspace right after a fix reverts it; that word is then left alone for the session.
            let inserted = u.keys.len();
            st.ignored.insert(u.original.to_lowercase(), Instant::now());
            replace(st, proxy, inserted + 1, &u.keys, &u.original, u.from, u.end);
            reset(st, false);
            return std::ptr::null_mut();
        }
        if st.keys.pop().is_none() {
            st.dirty = true;
        }
        return e;
    }

    if key == K_SPACE || key == K_RETURN || key == K_ENTER {
        let end = if key == K_SPACE { (K_SPACE, ' ') } else { (key, '\r') };
        let keys = std::mem::take(&mut st.keys);
        let dirty = std::mem::replace(&mut st.dirty, false);
        let Some(cur) = current_layout(st) else { return e };
        if st.layouts[cur].cyrillic {
            st.last_cyrillic = Some(cur);
        }
        let prev = st.prev.take();
        if dirty || keys.is_empty() {
            return e;
        }
        let typed = decide::translate(&keys, &st.layouts[cur]).unwrap_or_default();
        st.ignored.retain(|_, t| t.elapsed() < Duration::from_secs(600));
        if st.ignored.contains_key(&typed.to_lowercase()) {
            return e;
        }
        let (layouts, last, speller) = (&st.layouts, st.last_cyrillic, &mut st.speller);
        if let Some((to, text)) = decide(&keys, layouts, cur, last, &mut |w, l| speller.valid(w, l)) {
            // A short word just before ("ьн problems") is too ambiguous alone; fix it too if it fits the same layout.
            let short = prev.filter(|(_, l)| *l == cur).and_then(|(pk, _)| {
                let t = decide::translate(&pk, &st.layouts[to])?;
                let ok = decide::word_ok(&t, st.layouts[to].cyrillic) && speller.valid(&t, &st.layouts[to].lang);
                ok.then(|| (pk, t))
            });
            let (keys, typed, text) = match short {
                Some((pk, t)) => {
                    let orig = decide::translate(&pk, &st.layouts[cur]).unwrap_or_default();
                    ([pk, vec![(K_SPACE, false)], keys].concat(), format!("{orig} {typed}"), format!("{t} {text}"))
                }
                None => (keys, typed, text),
            };
            replace(st, proxy, keys.len(), &keys, &text, to, end);
            st.undo = Some(Undo { at: Instant::now(), end, keys, original: typed, from: cur });
            if st.layouts[to].cyrillic {
                st.last_cyrillic = Some(to);
            }
            return std::ptr::null_mut(); // the space is re-posted after the fix
        }
        if keys.len() < decide::MIN_LEN && key == K_SPACE {
            st.prev = Some((keys, cur));
        }
        return e;
    }

    // A key that types a character continues the word (punctuation is sorted out by decide);
    // keys that type nothing (arrows, Home, F-keys) leave the caret somewhere unknown.
    let shift = flags & (SHIFT | CAPS) != 0;
    let prints = key < 128 && st.layouts.iter().any(|l| l.table[key as usize * 2].is_some());
    if prints && st.keys.len() < 64 {
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
            println!("{} lang={} → {} valid={} | typed here, fix to: {:?}", l.id, l.lang, t, sp.valid(&t, &l.lang), d);
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
        ignored: HashMap::new(),
        tap: null(),
    }));
    unsafe {
        CGRequestListenEventAccess();
        CGRequestPostEventAccess();
        eprintln!("layoutfix: input monitoring={} post events={}", CGPreflightListenEventAccess(), CGPreflightPostEventAccess());
        let mask = 1u64 << KEY_DOWN | 1 << 1 | 1 << 3 | 1 << 25; // key down, mouse down (left/right/other)
        let tap = loop {
            let t = CGEventTapCreate(1, 0, 0, mask, on_event, st as *mut c_void);
            if !t.is_null() {
                break t;
            }
            eprintln!("layoutfix: waiting for Accessibility permission");
            std::thread::sleep(std::time::Duration::from_secs(5));
        };
        (*st).tap = tap;
        CFRunLoopAddSource(CFRunLoopGetCurrent(), CFMachPortCreateRunLoopSource(null(), tap, 0), kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRunLoopRun();
    }
}
