//! Pure decision logic: which layout a typed word was meant for. No system calls.

pub struct Layout {
    pub id: String,
    pub lang: String,
    pub cyrillic: bool,
    /// char for keycode (0..128), index = keycode * 2 + shift
    pub table: Vec<Option<char>>,
}

/// Key as typed: (keycode, shift-or-capslock).
pub type Key = (u16, bool);

pub fn translate(keys: &[Key], l: &Layout) -> Option<String> {
    keys.iter()
        .map(|&(k, s)| *l.table.get(k as usize * 2 + s as usize)?)
        .collect::<Option<String>>()
}

/// Letters of the layout's script; a hyphen or apostrophe is allowed between letters ("как-то", "don't").
pub fn word_ok(text: &str, cyrillic: bool) -> bool {
    let letter = |c: char| {
        if cyrillic { ('\u{400}'..='\u{4FF}').contains(&c) && c.is_alphabetic() } else { c.is_ascii_alphabetic() }
    };
    let t: Vec<char> = text.chars().collect();
    !t.is_empty()
        && t.iter().enumerate().all(|(i, &c)| {
            letter(c) || (matches!(c, '-' | '\'' | '’') && i > 0 && i + 1 < t.len() && letter(t[i - 1]) && letter(t[i + 1]))
        })
}

// Words under 3 keys are too ambiguous for the spell checker ("z"/"я", "db"/"ви"): 2-letter words are fixed only
// into this list, 1-letter ones only into this list and only together with the next word (see main.rs).
pub const MIN_LEN: usize = 3;
const SHORT: &[(&str, &[&str])] = &[
    ("ru", &["ты", "мы", "вы", "он", "да", "не", "на", "по", "за", "из", "от", "до", "же", "ли", "бы", "во", "со", "ну", "но", "то", "их", "об", "уж", "ей", "ща",
            "я", "в", "с", "к", "и", "а", "о", "у"]),
    ("uk", &["ти", "ми", "ви", "та", "це", "як", "чи", "ще", "не", "на", "по", "за", "із", "до", "же", "би", "то", "ні", "ну", "бо", "аж",
            "я", "в", "з", "і", "й", "а", "у", "о"]),
    ("en", &["is", "it", "in", "on", "at", "to", "of", "or", "if", "me", "my", "we", "he", "be", "do", "go", "no", "so", "up", "us", "an", "as", "by", "am", "hi", "ok", "oh",
            "a", "i"]),
];
// shortcut: tech tokens typed in the Latin layout that read as a word in a Cyrillic one ("vs" → "мы", "ctx" → "сеч").
// The 3-letter ones are what a run of ~480 common dev tokens through the real spell checker turned up; extend when one bites.
const KEEP: &[&str] = &[
    "db", "js", "ts", "ui", "ux", "ci", "cd", "pr", "ps", "vs", "os", "id", "ip", "qa", "vm", "ls", "rm", "mv", "py",
    "ctx", "csv", "len", "vec", "rfc", "jwt", "tls", "buf", "otp", "cwe", "lfi", "rfi", "ide", "csr",
];

pub fn short_word(t: &str, lang: &str) -> bool {
    let t = t.to_lowercase();
    SHORT.iter().any(|(l, w)| *l == lang && w.contains(&t.as_str()))
}

/// Returns (target layout index, corrected text) or None to leave the word alone.
pub fn decide(
    keys: &[Key],
    layouts: &[Layout],
    cur: usize,
    last_cyrillic: Option<usize>,
    valid: &mut dyn FnMut(&str, &str) -> bool,
) -> Option<(usize, String)> {
    let c = &layouts[cur];
    // Punctuation around the word where it was typed ("цщгдв" in quotes, "ghbdtn,", "руддщ.") stays as typed.
    let typed_all: Vec<char> = translate(keys, c)?.chars().collect();
    let head = typed_all.iter().take_while(|ch| !ch.is_alphabetic()).count();
    let tail = typed_all[head..].iter().rev().take_while(|ch| !ch.is_alphabetic()).count();
    let core = &keys[head..keys.len() - tail];
    let prefix: String = typed_all[..head].iter().collect();
    let suffix: String = typed_all[typed_all.len() - tail..].iter().collect();
    // ALL-CAPS short tokens are usually acronyms (API, SQL, KB)
    if (2..=4).contains(&keys.len()) && keys.iter().all(|k| k.1) {
        return None;
    }
    let typed = translate(core, c)?;
    if !c.cyrillic && KEEP.contains(&typed.to_lowercase().as_str()) {
        return None;
    }
    if core.len() == 2 && head + tail == 0 {
        if short_word(&typed, &c.lang) || valid(&typed, &c.lang) {
            return None;
        }
        let mut found: Vec<(usize, String)> = layouts.iter().enumerate()
            .filter(|(_, l)| l.cyrillic != c.cyrillic)
            .filter_map(|(i, l)| translate(core, l).filter(|t| short_word(t, &l.lang)).map(|t| (i, t)))
            .collect();
        if let Some(hit) = last_cyrillic.and_then(|p| found.iter().position(|(i, _)| *i == p)) {
            return Some(found.swap_remove(hit));
        }
        return found.into_iter().next();
    }
    if keys.len() < MIN_LEN {
        return None;
    }
    // Already a word where it was typed. A 1–2 letter core next to punctuation ("'nj" = "это") counts only
    // if it is a common short word: the spell checker accepts almost any two letters.
    let core_valid = if core.len() < MIN_LEN {
        short_word(&typed, &c.lang)
    } else {
        word_ok(&typed, c.cyrillic) && valid(&typed, &c.lang)
    };
    if core_valid {
        return None;
    }
    let mut find = |core: &[Key], prefix: &str, suffix: &str, strict: bool| {
        let mut found: Vec<(usize, String)> = Vec::new();
        for (i, l) in layouts.iter().enumerate() {
            if l.cyrillic == c.cyrillic {
                continue;
            }
            if let Some(t) = translate(core, l) {
                // keys that are letters here but punctuation in the target ("руддщб" → "hello,")
                let w = if strict { t.as_str() } else { t.trim_matches(|ch: char| !ch.is_alphabetic()) };
                // a 2-letter core with punctuation ("ns," → "ты,") only into the common short words
                let ok = match w.chars().count() {
                    n if n >= MIN_LEN => word_ok(w, l.cyrillic) && valid(w, &l.lang),
                    2 => !strict && short_word(w, &l.lang),
                    _ => false,
                };
                if ok {
                    found.push((i, format!("{prefix}{t}{suffix}")));
                }
            }
        }
        found
    };
    // First every key as a letter of the target: "levf." is "думаю" (the . key is ю), not "дума" + ".".
    // Then punctuation kept as typed: "ghbdtn," → "привет,", "\"цщгдв\"" → "\"would\"", "руддщб" → "hello,".
    let mut found = find(keys, "", "", true);
    if found.is_empty() && core.len() >= 2 {
        found = find(core, &prefix, &suffix, false);
    }
    if let Some(p) = last_cyrillic {
        if let Some(hit) = found.iter().position(|(i, _)| *i == p) {
            return Some(found.swap_remove(hit));
        }
    }
    found.into_iter().next()
}

/// One edit apart (insert, delete, replace, or swap of two neighbours) or equal.
pub fn within_one(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (s, l) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if l.len() - s.len() > 1 {
        return false;
    }
    let i = s.iter().zip(l.iter()).take_while(|(x, y)| x == y).count();
    if i == s.len() {
        return true;
    }
    if s.len() < l.len() {
        return s[i..] == l[i + 1..];
    }
    s[i + 1..] == l[i + 1..] || (i + 1 < s.len() && s[i] == l[i + 1] && s[i + 1] == l[i] && s[i + 2..] == l[i + 2..])
}

pub const TYPO_MIN: usize = 5;

/// Wrong layout plus a typo ("ghbdtnn" → "приветт"): the typed text is nowhere near a word, while another
/// layout reads one edit away from a word. `dict(w, lang, false)`: w is a word or one edit from one;
/// `dict(w, lang, true)`: w is far from every word (the spell checker has nothing to suggest).
/// shortcut: punctuation around the word ("ghbdtnn,") is not split off; extend like `decide` if missed.
pub fn typo(
    keys: &[Key],
    layouts: &[Layout],
    cur: usize,
    last_cyrillic: Option<usize>,
    dict: &mut dyn FnMut(&str, &str, bool) -> bool,
) -> Option<(usize, String)> {
    let c = &layouts[cur];
    let typed = translate(keys, c)?;
    // "grafana" gets suggestions (griffin) and is left alone; "ghbdtnn", "cgfcb,jj" get none
    if keys.len() < TYPO_MIN || keys.iter().all(|k| k.1) || !dict(&typed, &c.lang, true) {
        return None;
    }
    let mut found: Vec<(usize, String)> = layouts.iter().enumerate()
        .filter(|(_, l)| l.cyrillic != c.cyrillic)
        .filter_map(|(i, l)| translate(keys, l).filter(|t| word_ok(t, l.cyrillic) && dict(t, &l.lang, false)).map(|t| (i, t)))
        .collect();
    if let Some(hit) = last_cyrillic.and_then(|p| found.iter().position(|(i, _)| *i == p)) {
        return Some(found.swap_remove(hit));
    }
    found.into_iter().next()
}

/// "ПРивет", "GHbdtn": Shift held a letter too long. Keys with only the first letter shifted.
pub fn fix_case(keys: &[Key]) -> Option<Vec<Key>> {
    let ok = keys.len() >= 5 && keys[0].1 && keys[1].1 && keys[2..].iter().all(|k| !k.1);
    ok.then(|| keys.iter().enumerate().map(|(i, &(k, s))| (k, s && i == 0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tiny layouts: keycodes 0..=5 → en "ghbdtn", ru "привет", uk "пріват"-ish (key 2 differs)
    fn layout(lang: &str, cyr: bool, chars: &str) -> Layout {
        let mut table = vec![None; 256];
        for (k, ch) in chars.chars().enumerate() {
            table[k * 2] = Some(ch);
            table[k * 2 + 1] = ch.to_uppercase().next();
        }
        Layout { id: lang.into(), lang: lang.into(), cyrillic: cyr, table }
    }

    fn setup() -> Vec<Layout> {
        vec![
            layout("en", false, "ghbdtn"),
            layout("ru", true, "привет"),
            layout("uk", true, "прівет"),
        ]
    }

    fn dict(words: &'static [&'static str]) -> impl FnMut(&str, &str) -> bool {
        move |w, _| words.contains(&w.to_lowercase().as_str())
    }

    const K: [Key; 6] = [(0, false), (1, false), (2, false), (3, false), (4, false), (5, false)];

    #[test]
    fn en_typed_meant_ru() {
        let mut v = dict(&["привет"]);
        assert_eq!(decide(&K, &setup(), 0, Some(2), &mut v), Some((1, "привет".into())));
    }

    #[test]
    fn valid_word_left_alone() {
        let mut v = dict(&["ghbdtn", "привет"]);
        assert_eq!(decide(&K, &setup(), 0, None, &mut v), None);
    }

    #[test]
    fn both_cyrillic_valid_prefers_last_used() {
        let mut v = dict(&["привет", "прівет"]);
        assert_eq!(decide(&K, &setup(), 0, Some(2), &mut v).unwrap().0, 2);
        assert_eq!(decide(&K, &setup(), 0, Some(1), &mut v).unwrap().0, 1);
    }

    #[test]
    fn cyrillic_typed_meant_en_and_case_kept() {
        let keys: Vec<Key> = K.iter().enumerate().map(|(i, k)| (k.0, i == 0)).collect();
        let mut v = dict(&["ghbdtn"]);
        assert_eq!(decide(&keys, &setup(), 1, None, &mut v), Some((0, "Ghbdtn".into())));
    }

    #[test]
    fn short_and_unknown_skipped() {
        let mut v = dict(&["при"]);
        assert_eq!(decide(&K[..2], &setup(), 0, None, &mut v), None);
        let mut none = dict(&[]);
        assert_eq!(decide(&K, &setup(), 0, None, &mut none), None);
    }

    // en "ghbdtn," / ru "приветб": key 6 is ',' in en and 'б' in ru
    fn setup_punct() -> Vec<Layout> {
        vec![
            layout("en", false, "ghbdtn,"),
            layout("ru", true, "приветб"),
        ]
    }
    const KP: [Key; 7] = [(0, false), (1, false), (2, false), (3, false), (4, false), (5, false), (6, false)];

    #[test]
    fn trailing_punct_in_typed_layout_kept() {
        let mut v = dict(&["привет"]);
        assert_eq!(decide(&KP, &setup_punct(), 0, None, &mut v), Some((1, "привет,".into())));
    }

    #[test]
    fn trailing_letter_that_is_punct_in_target() {
        let mut v = dict(&["ghbdtn"]);
        assert_eq!(decide(&KP, &setup_punct(), 1, None, &mut v), Some((0, "ghbdtn,".into())));
    }

    #[test]
    fn punct_key_that_is_a_letter_wins_when_whole_word_valid() {
        // en "ghbdtn," → ru "приветб": if "приветб" were a word it must win over "привет,"
        let mut v = dict(&["приветб", "привет"]);
        assert_eq!(decide(&KP, &setup_punct(), 0, None, &mut v), Some((1, "приветб".into())));
    }

    #[test]
    fn two_letter_words_only_from_the_list() {
        let mut ls = vec![layout("en", false, "nsdbvs"), layout("ru", true, "тывимы")];
        ls[1].lang = "ru".into();
        let k = |a: u16, b: u16| vec![(a, false), (b, false)];
        let mut none = dict(&[]);
        assert_eq!(decide(&k(0, 1), &ls, 0, None, &mut none), Some((1, "ты".into()))); // ns → ты
        assert_eq!(decide(&k(2, 3), &ls, 0, None, &mut none), None); // db → "ви" is not ru list
        assert_eq!(decide(&k(4, 5), &ls, 0, None, &mut none), None); // vs is kept
        let mut en_ns = dict(&["ns"]);
        assert_eq!(decide(&k(0, 1), &ls, 0, None, &mut en_ns), None); // valid where typed
    }

    #[test]
    fn inner_hyphen_and_apostrophe() {
        assert!(word_ok("как-то", true));
        assert!(word_ok("don't", false));
        assert!(!word_ok("-rf", false));
        assert!(!word_ok("it'", false));
    }

    #[test]
    fn caps_acronyms_and_kept_tokens_left_alone() {
        let ls = vec![layout("en", false, "kbctx"), layout("ru", true, "лисеч")];
        let mut any = |_: &str, l: &str| l == "ru";
        let caps: Vec<Key> = vec![(0, true), (1, true)];
        assert_eq!(decide(&caps, &ls, 0, None, &mut any), None); // KB
        let ctx: Vec<Key> = vec![(2, false), (3, false), (4, false)];
        assert_eq!(decide(&ctx, &ls, 0, None, &mut any), None); // ctx is kept though "сеч" is a word
    }

    #[test]
    fn leading_punct_kept() {
        // key 6 first: "," in en → ",ghbdtn" typed; ru reads "бпривет", so only the en-typed case has a prefix
        let keys: Vec<Key> = [6, 0, 1, 2, 3, 4, 5].iter().map(|&k| (k, false)).collect();
        let mut v = dict(&["привет"]);
        assert_eq!(decide(&keys, &setup_punct(), 0, None, &mut v), Some((1, ",привет".into())));
    }

    #[test]
    fn result_has_one_char_per_key() {
        let mut v = dict(&["привет", "ghbdtn"]);
        for cur in 0..2 {
            if let Some((_, t)) = decide(&KP, &setup_punct(), cur, None, &mut v) {
                assert_eq!(t.chars().count(), KP.len());
            }
        }
    }

    #[test]
    fn short_word_with_punct() {
        let ls = vec![layout("en", false, "ns,."), layout("ru", true, "тыбю")];
        let k = |v: &[u16]| v.iter().map(|&k| (k, false)).collect::<Vec<Key>>();
        let mut none = dict(&[]);
        assert_eq!(decide(&k(&[0, 1, 2]), &ls, 0, None, &mut none), Some((1, "ты,".into())));
        assert_eq!(decide(&k(&[0, 1, 3]), &ls, 0, None, &mut none), Some((1, "ты.".into())));
        let ls2 = vec![layout("en", false, "db,"), layout("ru", true, "вib")];
        assert_eq!(decide(&k(&[0, 1, 2]), &ls2, 0, None, &mut none), None); // db, is kept
    }

    #[test]
    fn one_edit() {
        assert!(within_one("привет", "приветт"));
        assert!(within_one("привет", "прибет"));
        assert!(within_one("привет", "пирвет"));
        assert!(within_one("привет", "првет"));
        assert!(!within_one("привет", "пирвте"));
        assert!(!within_one("привет", "привееет"));
    }

    #[test]
    fn typo_in_wrong_layout() {
        let ls = vec![layout("en", false, "ghbdtnx"), layout("ru", true, "приветч")];
        let keys: Vec<Key> = [0, 1, 2, 3, 4, 5, 5].iter().map(|&k| (k, false)).collect(); // ghbdtnn
        let mut near = |w: &str, _: &str, far: bool| within_one(w, "привет") != far;
        assert_eq!(typo(&keys, &ls, 0, None, &mut near), Some((1, "приветт".into())));
        let mut en_near = |w: &str, _: &str, far: bool| within_one(w, "ghbdtn") != far; // typo in the right layout: left alone
        assert_eq!(typo(&keys, &ls, 0, None, &mut en_near), None);
        assert_eq!(typo(&keys[..4], &ls, 0, None, &mut near), None); // too short
    }

    #[test]
    fn double_capital() {
        let k = |s: &[bool]| s.iter().enumerate().map(|(i, &s)| (i as u16, s)).collect::<Vec<Key>>();
        assert_eq!(fix_case(&k(&[true, true, false, false, false, false])), Some(k(&[true, false, false, false, false, false])));
        assert_eq!(fix_case(&k(&[true, true, true, false, false])), None); // SSHed
        assert_eq!(fix_case(&k(&[true, true, false, false])), None); // MBps
        assert_eq!(fix_case(&k(&[true, false, false, false, false])), None);
    }

    #[test]
    fn script_check() {
        assert!(word_ok("hello", false));
        assert!(!word_ok("hf,jnftn", false));
        assert!(word_ok("работает", true));
        assert!(!word_ok("руддщ1", true));
    }
}
