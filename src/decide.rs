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

/// All letters, all from the layout's script.
pub fn word_ok(text: &str, cyrillic: bool) -> bool {
    !text.is_empty()
        && text.chars().all(|c| {
            if cyrillic {
                ('\u{400}'..='\u{4FF}').contains(&c) && c.is_alphabetic()
            } else {
                c.is_ascii_alphabetic()
            }
        })
}

// shortcut: words under 3 keys are skipped (too ambiguous: "z"/"я", "ye"/"ну"); add a common-short-words list if needed
pub const MIN_LEN: usize = 3;

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
    if core.len() < MIN_LEN {
        return None;
    }
    // ALL-CAPS short tokens are usually acronyms (API, SQL)
    if core.len() <= 4 && core.iter().all(|k| k.1) {
        return None;
    }
    let typed = translate(core, c)?;
    if word_ok(&typed, c.cyrillic) && valid(&typed, &c.lang) {
        return None;
    }
    let mut found: Vec<(usize, String)> = Vec::new();
    for (i, l) in layouts.iter().enumerate() {
        if l.cyrillic == c.cyrillic {
            continue;
        }
        if let Some(t) = translate(core, l) {
            // keys that are letters here but punctuation in the target ("руддщб" → "hello,")
            let w = t.trim_matches(|ch: char| !ch.is_alphabetic());
            if w.chars().count() >= MIN_LEN && word_ok(w, l.cyrillic) && valid(w, &l.lang) {
                found.push((i, format!("{prefix}{t}{suffix}")));
            }
        }
    }
    if let Some(p) = last_cyrillic {
        if let Some(hit) = found.iter().position(|(i, _)| *i == p) {
            return Some(found.swap_remove(hit));
        }
    }
    found.into_iter().next()
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
    fn script_check() {
        assert!(word_ok("hello", false));
        assert!(!word_ok("hf,jnftn", false));
        assert!(word_ok("работает", true));
        assert!(!word_ok("руддщ1", true));
    }
}
