//! Two interface languages: German (the language of the strings in the code) and English.
//!
//! The German strings double as lookup keys: `tr!("Ordner öffnen …")` shows that exact text in
//! German and the dictionary's English translation otherwise. A missing translation falls back
//! to the German text, so the interface never breaks — it just stays German until translated.

mod en_app;
mod en_git;
mod en_gitui;
mod en_ide;
mod en_menu;
mod en_misc;
mod en_refsync;
mod en_screens;
mod en_zotero;

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// A language the interface can be shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    De,
    En,
}

impl Lang {
    /// The language's own name, as the settings show it.
    pub fn name(self) -> &'static str {
        match self {
            Self::De => "Deutsch",
            Self::En => "English",
        }
    }

    /// From a locale code ("de-DE", "en_US", "de"), accepting a few near misses.
    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code.split(['-', '_']).next()?.to_lowercase().as_str() {
            "de" | "ger" | "german" | "deutsch" => Self::De,
            "en" | "eng" | "english" => Self::En,
            _ => return None,
        })
    }
}

/// The active language. Only ever changed at startup and from the settings.
static LANG: AtomicU8 = AtomicU8::new(0); // 0 = Lang::De

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 { Lang::En } else { Lang::De }
}

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

/// The system's language, German when the locale is one Typisch doesn't know.
pub fn system_lang() -> Lang {
    sys_locale::get_locale().and_then(|l| Lang::from_code(&l)).unwrap_or(Lang::De)
}

/// The language the `TYPISCH_LANG` environment variable asks for (tests and screenshots).
pub fn env_lang() -> Option<Lang> {
    std::env::var("TYPISCH_LANG").ok().and_then(|l| Lang::from_code(&l))
}

/// Chosen once at startup: the `TYPISCH_LANG` environment variable, then the saved setting,
/// then the system language, then German.
pub fn init(saved: Option<Lang>) {
    set(env_lang().or(saved).unwrap_or_else(system_lang));
}

/// The English text for the German UI string, if it is translated.
pub fn en(de: &str) -> Option<&'static str> {
    EN.get(de).copied()
}

/// Translates a German UI string into the active language. In German the text is the key itself.
pub fn t(de: &'static str) -> &'static str {
    translate(lang(), de)
}

/// The English text for the German UI string `de`, or the German text itself as the fallback.
pub fn translate(lang: Lang, de: &'static str) -> &'static str {
    match lang {
        Lang::De => de,
        Lang::En => en(de).unwrap_or(de),
    }
}

/// Like [`t`] for a German word that needs different English words in different places:
/// `tc("Darstellung", "Menü")` looks up the entry `"Darstellung [Menü]"`.
pub fn tc(de: &'static str, context: &str) -> &'static str {
    match lang() {
        Lang::De => de,
        Lang::En => EN
            .get(format!("{de} [{context}]").as_str())
            .copied()
            .unwrap_or_else(|| translate(Lang::En, de)),
    }
}

/// A German month name in the active language: the release date in Cargo.toml is written in German.
pub fn month(de: &str) -> &str {
    if lang() == Lang::De {
        return de;
    }
    match de {
        "Januar" => "January",
        "Februar" => "February",
        "März" => "March",
        "Mai" => "May",
        "Juni" => "June",
        "Juli" => "July",
        "Oktober" => "October",
        "Dezember" => "December",
        same => same,
    }
}

/// Like [`t`], but fills `{name}` placeholders, because the word order differs between languages.
pub fn tf(de: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(lang(), de, args)
}

/// Like [`translate`], with `{name}` placeholders filled in.
pub fn fill(lang: Lang, de: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = translate(lang, de).to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

/// Translates a UI string: `tr!("Ordner öffnen …")`, with placeholders
/// `tr!("Verbinde mit {url} …", url: url)`. The German text is the key and the fallback.
#[macro_export]
macro_rules! tr {
    ($key:literal) => { $crate::i18n::t($key) };
    ($key:literal, $($name:ident : $value:expr),+ $(,)?) => {
        $crate::i18n::tf($key, &[$((stringify!($name), &$value)),+])
    };
}

type Entry = (&'static str, &'static str);

static EN: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| EN_GROUPS.iter().copied().flatten().copied().collect());

/// All dictionaries, for the map above and the tests below.
const EN_GROUPS: &[&[Entry]] = &[
    en_app::EN,
    en_git::EN,
    en_gitui::EN,
    en_ide::EN,
    en_menu::EN,
    en_misc::EN,
    en_refsync::EN,
    en_screens::EN,
    en_zotero::EN,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn german_shows_the_key_itself() {
        assert_eq!(translate(Lang::De, "Ordner öffnen …"), "Ordner öffnen …");
    }

    #[test]
    fn english_falls_back_to_german() {
        assert_eq!(
            translate(Lang::En, "Diese Zeichenkette ist garantiert nie übersetzt"),
            "Diese Zeichenkette ist garantiert nie übersetzt"
        );
    }

    #[test]
    fn placeholders_are_filled() {
        assert_eq!(fill(Lang::De, "Lade {n} Dateien", &[("n", &7)]), "Lade 7 Dateien");
        assert_eq!(fill(Lang::En, "Lade {n} Dateien", &[("n", &7)]), "Lade 7 Dateien");
    }

    #[test]
    fn locale_codes_are_understood() {
        assert_eq!(Lang::from_code("de-DE"), Some(Lang::De));
        assert_eq!(Lang::from_code("en_US"), Some(Lang::En));
        assert_eq!(Lang::from_code("fr"), None);
    }

    /// Every entry must be a real translation: non-empty, changed, with the same {placeholders}.
    #[test]
    fn entries_are_translations() {
        for group in EN_GROUPS {
            for &(de, en) in group.iter() {
                assert!(!de.is_empty() && !en.is_empty(), "empty entry {de:?}");
                assert_ne!(de, en, "unchanged entry {de:?}");
                assert_eq!(placeholders(de), placeholders(en), "different placeholders for {de:?}");
            }
        }
    }

    /// The same German text must always be translated the same way (the map would hide the conflict).
    #[test]
    fn entries_are_consistent() {
        let mut seen: HashMap<&'static str, &'static str> = HashMap::new();
        for group in EN_GROUPS {
            for &(de, en) in group.iter() {
                if let Some(prev) = seen.insert(de, en) {
                    assert_eq!(prev, en, "conflicting translations for {de:?}");
                }
            }
        }
    }

    /// Every `tr!("…")` and `tc("…", "…")` in the code has an English text, so nothing stays
    /// German by accident. A text that reads the same in both languages is not wrapped at all.
    #[test]
    fn every_string_in_the_code_is_translated() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut found = 0;
        for entry in std::fs::read_dir(&src).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let code = std::fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            for (marker, with_context) in [("tr!(", false), ("i18n::tc(", true)] {
                for (at, _) in code.match_indices(marker) {
                    let mut rest = code[at + marker.len()..].trim_start();
                    let Some(key) = literal(&mut rest) else { continue };
                    let key = match with_context {
                        false => key,
                        true => {
                            rest = rest.trim_start().trim_start_matches(',').trim_start();
                            format!("{key} [{}]", literal(&mut rest).expect("tc needs a literal context"))
                        }
                    };
                    found += 1;
                    if !EN.contains_key(key.as_str()) {
                        missing.push(format!("{name}: {key:?}"));
                    }
                }
            }
        }
        assert!(found > 300, "the scan found only {found} strings");
        assert!(missing.is_empty(), "without an English text:\n{}", missing.join("\n"));
    }

    /// Template names and descriptions are translated where they are shown (`i18n::t(t.name)`).
    #[test]
    fn templates_are_translated() {
        for t in crate::templates::TEMPLATES {
            assert!(en(t.description).is_some(), "no English description for {:?}", t.name);
            // "Flyer" reads the same in both languages.
            assert!(en(t.name).is_some() || t.name == "Flyer", "no English name for {:?}", t.name);
        }
    }

    /// Reads the string literal `rest` starts with and moves `rest` behind it.
    fn literal(rest: &mut &str) -> Option<String> {
        let mut chars = rest.strip_prefix('"')?.char_indices();
        let mut out = String::new();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    *rest = &rest[i + 2..];
                    return Some(out);
                }
                '\\' => match chars.next()?.1 {
                    // A backslash at the end of a line: the text goes on after the indentation.
                    '\n' => {
                        while chars.clone().next().is_some_and(|(_, c)| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    fn placeholders(s: &str) -> BTreeSet<String> {
        s.match_indices('{')
            .filter_map(|(i, _)| s[i..].find('}').map(|e| s[i + 1..i + e].to_string()))
            .filter(|p| !p.is_empty())
            .collect()
    }
}
