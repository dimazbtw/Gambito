//! Languages. The text in the code is Portuguese; each other language is one file under
//! assets/i18n/ with a line per text: `Portuguese<TAB>translation` (`\n` for a new line, `{}` for a
//! value). A text with no translation shows in Portuguese. To add a language: a new .tsv, a line in
//! LANGS and one in `table()`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// (code, name shown in the menu)
pub const LANGS: [(&str, &str); 2] = [("pt", "Português"), ("en", "English")];

static LANG: AtomicUsize = AtomicUsize::new(0);

pub fn lang() -> usize { LANG.load(Ordering::Relaxed) }

fn settings_path() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Gambito").join("settings.json"))
}

/// The language saved in the settings, if there is one.
pub fn saved() -> Option<String> {
    settings_path().and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["lang"].as_str().map(String::from))
}

/// The saved choice, else the language of Windows (Portuguese for pt-*, English for anything else).
pub fn init() {
    let code = saved().or_else(sys_locale::get_locale).unwrap_or_default().to_lowercase();
    LANG.store(if code.starts_with("pt") { 0 } else { 1 }, Ordering::Relaxed);
}

/// Changes the language for this run only (the installer saves it when it installs).
pub fn show(i: usize) { LANG.store(i.min(LANGS.len() - 1), Ordering::Relaxed); }

pub fn set(i: usize) {
    show(i);
    save_setting("lang", serde_json::json!(LANGS[lang()].0));
}

/// A value kept in settings.json (language, opponent strength, sounds, board colours, ...).
pub fn setting(key: &str) -> Option<serde_json::Value> {
    settings_path().and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| v[key].clone()).filter(|v| !v.is_null())
}

/// Changes one value of settings.json and keeps the others.
pub fn save_setting(key: &str, value: serde_json::Value) {
    let Some(p) = settings_path() else { return };
    let mut v = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v.is_object()).unwrap_or_else(|| serde_json::json!({}));
    v[key] = value;
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    let _ = std::fs::write(p, v.to_string());
}

fn unescape(s: &str) -> String { s.replace("\\n", "\n").replace("\\t", "\t") }

fn parse(tsv: &str) -> HashMap<String, &'static str> {
    tsv.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| {
        let (k, v) = l.split_once('\t')?;
        Some((unescape(k), &*Box::leak(unescape(v).into_boxed_str())))
    }).collect()
}

fn table(i: usize) -> &'static HashMap<String, &'static str> {
    static EN: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    static NONE: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    match i {
        1 => EN.get_or_init(|| parse(include_str!("../assets/i18n/en.tsv"))),
        _ => NONE.get_or_init(HashMap::new),
    }
}

/// The text in the chosen language.
pub fn tl(s: &str) -> &str {
    match lang() {
        0 => s,
        i => table(i).get(s).copied().unwrap_or(s),
    }
}

/// Puts the values into the `{}` of a translated text, in order.
pub fn fill(fmt: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut it = args.iter();
    let mut rest = fmt;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        match it.next() { Some(a) => out.push_str(&a.to_string()), None => out.push_str("{}") }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// `tr!("Lance {} de {}", n, total)`: translated, with the values put in.
#[macro_export]
macro_rules! tr {
    ($s:literal $(, $a:expr)* $(,)?) => { $crate::i18n::fill($crate::i18n::tl($s), &[$(&$a as &dyn std::fmt::Display),*]) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every literal given to tl("…") or tr!("…") in the code, as the compiler sees it.
    fn literals_in_code() -> Vec<(String, String)> {
        let mut out = vec![];
        for f in std::fs::read_dir("src").unwrap().flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if !name.ends_with(".rs") || name == "i18n.rs" { continue; }
            let src = std::fs::read_to_string(f.path()).unwrap();
            let src = src.split("#[cfg(test)]").next().unwrap(); // tests keep Portuguese
            for pat in ["tl(\"", "tr!(\""] {
                let mut rest = src;
                while let Some(i) = rest.find(pat) {
                    let before = rest[..i].chars().last();
                    rest = &rest[i + pat.len()..];
                    if before.map_or(false, |c| c.is_alphanumeric() || c == '_') { continue; }
                    let (mut s, mut chars) = (String::new(), rest.chars());
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => match chars.next() { Some('n') => s.push('\n'), Some('t') => s.push('\t'), Some(o) => s.push(o), None => {} },
                            _ => s.push(c),
                        }
                    }
                    out.push((name.clone(), s));
                }
            }
        }
        out
    }

    #[test]
    fn every_text_has_an_english_translation() {
        let en = table(1);
        let mut texts: Vec<String> = literals_in_code().into_iter().map(|(_, s)| s).collect();
        // the data texts too: lessons, puzzle themes, move ratings
        for l in crate::train::LESSONS { texts.extend([l.title, l.text, l.group].map(String::from)); }
        for p in crate::train::load_puzzles() { texts.extend(p.themes.iter().filter_map(|t| crate::train::theme_name(t)).map(String::from)); }
        texts.extend(crate::CLASSES.iter().map(|c| c.label().to_string()));
        for g in &crate::games::GAMES { texts.extend([g.name, g.blurb].map(String::from)); }
        texts.extend(crate::THEMES.iter().map(|t| t.0.to_string()));
        let mut missing: Vec<String> = texts.into_iter().filter(|s| !en.contains_key(s)).collect();
        missing.sort();
        missing.dedup();
        if let Ok(p) = std::env::var("I18N_MISSING") { // dev: write the missing ones out, to translate
            let _ = std::fs::write(p, missing.iter().map(|s| s.replace('\n', "\\n") + "\t\n").collect::<String>());
        }
        assert!(missing.is_empty(), "{} without translation:\n{}", missing.len(), missing.join("\n"));
    }

    #[test]
    fn translations_keep_the_values() {
        for (k, v) in table(1) {
            assert_eq!(k.matches("{}").count(), v.matches("{}").count(), "{k:?} -> {v:?}");
        }
    }

    #[test]
    fn fill_puts_values_in_order() {
        assert_eq!(fill("Lance {} de {}", &[&3, &"40"]), "Lance 3 de 40");
    }
}
