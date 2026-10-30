//! Interface language. Follows the macOS preferred languages (including a
//! per-app language set in System Settings), falling back to English.

use rust_i18n::t;

/// Pick the interface language once, before the first frame.
pub fn init() {
    let available = rust_i18n::available_locales!();
    let available: Vec<&str> = available.iter().map(AsRef::as_ref).collect();
    let locale = resolve_locale(sys_locale::get_locales(), &available);
    rust_i18n::set_locale(locale);
    log::info!("Interface language: {locale}");
}

/// First preferred language that has a translation, matching the full tag
/// (`pt-BR`), then the language with its script (`zh-Hant`), then the bare
/// language (`fr` for `fr-CA`).
fn resolve_locale<'a>(preferred: impl Iterator<Item = String>, available: &[&'a str]) -> &'a str {
    for tag in preferred {
        let tag = tag.replace('_', "-");
        let mut parts = tag.split('-');
        let language = parts.next().unwrap_or_default().to_ascii_lowercase();
        let rest: Vec<&str> = parts.collect();
        let mut candidates = vec![tag.clone()];
        if language == "zh" {
            let traditional = rest
                .iter()
                .any(|p| p.eq_ignore_ascii_case("Hant") || ["TW", "HK", "MO"].contains(p));
            candidates.push(if traditional { "zh-TW" } else { "zh-CN" }.to_string());
        }
        if let Some(region) = rest.iter().find(|p| p.len() == 2) {
            candidates.push(format!("{language}-{}", region.to_ascii_uppercase()));
        }
        candidates.push(language);
        for candidate in candidates {
            if let Some(found) = available
                .iter()
                .find(|a| a.eq_ignore_ascii_case(&candidate))
            {
                return found;
            }
        }
    }
    "en"
}

/// Labels of the header block written above a quoted message.
pub struct QuoteLabels {
    pub from: String,
    pub sent: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
}

/// Each label carries its colon, spaced as the language wants ("De :").
pub fn quote_labels() -> QuoteLabels {
    let label = |name: std::borrow::Cow<'_, str>| t!("quote.label", label = name).into_owned();
    QuoteLabels {
        from: label(t!("quote.from")),
        sent: label(t!("quote.sent")),
        to: label(t!("quote.to")),
        cc: label(t!("quote.cc")),
        subject: label(t!("quote.subject")),
    }
}

/// Weekday, day, month, year and time of a Unix-millisecond timestamp in the
/// local time zone, e.g. "vendredi 30 octobre 2026 10:13".
pub fn format_quote_date(millis: i64) -> Option<String> {
    let zoned = jiff::Timestamp::from_millisecond(millis)
        .ok()?
        .to_zoned(jiff::tz::TimeZone::system());
    Some(long_date_time(&zoned))
}

pub fn long_date_time(zoned: &jiff::Zoned) -> String {
    t!(
        "date.long_date_time",
        weekday = weekday_name(zoned.weekday()),
        day = zoned.day(),
        month = month_name(zoned.month()),
        year = zoned.year(),
        time = time_of_day(zoned)
    )
    .into_owned()
}

/// "10:13" or "10:13 AM", depending on the language.
pub fn time_of_day(zoned: &jiff::Zoned) -> String {
    let hour = zoned.hour();
    let minute = format!("{:02}", zoned.minute());
    if t!("date.clock") == "12" {
        let (hour12, period) = match hour {
            0 => (12, t!("date.am")),
            1..=11 => (hour, t!("date.am")),
            12 => (12, t!("date.pm")),
            _ => (hour - 12, t!("date.pm")),
        };
        format!("{hour12}:{minute} {period}")
    } else {
        format!("{hour:02}:{minute}")
    }
}

pub fn weekday_name(weekday: jiff::civil::Weekday) -> String {
    match weekday {
        jiff::civil::Weekday::Monday => t!("date.monday"),
        jiff::civil::Weekday::Tuesday => t!("date.tuesday"),
        jiff::civil::Weekday::Wednesday => t!("date.wednesday"),
        jiff::civil::Weekday::Thursday => t!("date.thursday"),
        jiff::civil::Weekday::Friday => t!("date.friday"),
        jiff::civil::Weekday::Saturday => t!("date.saturday"),
        jiff::civil::Weekday::Sunday => t!("date.sunday"),
    }
    .into_owned()
}

pub fn month_name(month: i8) -> String {
    match month {
        1 => t!("date.january"),
        2 => t!("date.february"),
        3 => t!("date.march"),
        4 => t!("date.april"),
        5 => t!("date.may"),
        6 => t!("date.june"),
        7 => t!("date.july"),
        8 => t!("date.august"),
        9 => t!("date.september"),
        10 => t!("date.october"),
        11 => t!("date.november"),
        _ => t!("date.december"),
    }
    .into_owned()
}

/// "Mon", "lun.", by days since Monday (0 = Monday).
pub fn weekday_short(days_from_monday: u8) -> String {
    match days_from_monday {
        0 => t!("date.short.monday"),
        1 => t!("date.short.tuesday"),
        2 => t!("date.short.wednesday"),
        3 => t!("date.short.thursday"),
        4 => t!("date.short.friday"),
        5 => t!("date.short.saturday"),
        _ => t!("date.short.sunday"),
    }
    .into_owned()
}

/// "Mar", "mars", by month number (1 = January).
pub fn month_short(month: i8) -> String {
    match month {
        1 => t!("date.short.january"),
        2 => t!("date.short.february"),
        3 => t!("date.short.march"),
        4 => t!("date.short.april"),
        5 => t!("date.short.may"),
        6 => t!("date.short.june"),
        7 => t!("date.short.july"),
        8 => t!("date.short.august"),
        9 => t!("date.short.september"),
        10 => t!("date.short.october"),
        11 => t!("date.short.november"),
        _ => t!("date.short.december"),
    }
    .into_owned()
}

/// Command labels are English `&'static str` in the cmdk crate; their
/// translations are keyed `cmd.<English label>`.
pub fn command(label: &'static str) -> std::borrow::Cow<'static, str> {
    crate::_rust_i18n_try_translate(&rust_i18n::locale(), format!("cmd.{label}"))
        .unwrap_or(std::borrow::Cow::Borrowed(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(preferred: &[&str]) -> &'static str {
        const AVAILABLE: &[&str] = &["en", "fr", "pt", "pt-BR", "zh-CN", "zh-TW"];
        resolve_locale(preferred.iter().map(|s| (*s).to_string()), AVAILABLE)
    }

    #[test]
    fn regional_variant_falls_back_to_language() {
        assert_eq!(pick(&["fr-FR"]), "fr");
        assert_eq!(pick(&["fr_CA"]), "fr");
    }

    #[test]
    fn exact_region_wins_over_language() {
        assert_eq!(pick(&["pt-BR"]), "pt-BR");
        assert_eq!(pick(&["pt-PT"]), "pt");
    }

    #[test]
    fn chinese_script_selects_simplified_or_traditional() {
        assert_eq!(pick(&["zh-Hans-CN"]), "zh-CN");
        assert_eq!(pick(&["zh-Hant-TW"]), "zh-TW");
        assert_eq!(pick(&["zh-HK"]), "zh-TW");
    }

    #[test]
    fn unknown_languages_are_skipped_in_preference_order() {
        assert_eq!(pick(&["xx-YY", "fr-BE"]), "fr");
        assert_eq!(pick(&["xx"]), "en");
    }

    #[test]
    fn quote_date_is_written_out() {
        let zoned: jiff::Zoned = "2026-10-30T10:13:00[Europe/Paris]".parse().expect("date");
        assert_eq!(long_date_time(&zoned), "Friday, October 30, 2026 10:13 AM");
        let french = t!(
            "date.long_date_time",
            locale = "fr",
            weekday = t!("date.friday", locale = "fr"),
            day = 30,
            month = t!("date.october", locale = "fr"),
            year = 2026,
            time = "10:13"
        );
        assert_eq!(french, "vendredi 30 octobre 2026 10:13");
    }

    type Entries = std::collections::BTreeMap<String, String>;

    /// Every locale file is `locales/<lang>/<area>.<lang>.yml` with one
    /// `key: "value"` per line; rust-i18n takes the locale from the last
    /// dot segment of the file name.
    fn locales() -> std::collections::BTreeMap<String, Entries> {
        fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("locales dir") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    walk(&path, files);
                } else if path.extension().is_some_and(|ext| ext == "yml") {
                    files.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales"),
            &mut files,
        );
        let mut by_locale: std::collections::BTreeMap<String, Entries> = Default::default();
        for path in files {
            let stem = path.file_stem().and_then(|s| s.to_str()).expect("stem");
            let locale = stem.rsplit('.').next().expect("locale").to_string();
            let entries = by_locale.entry(locale).or_default();
            for line in std::fs::read_to_string(&path).expect("locale file").lines() {
                if line.trim().is_empty() || line.starts_with('#') || line.starts_with("_version") {
                    continue;
                }
                let (key, value) = line
                    .split_once(": ")
                    .unwrap_or_else(|| panic!("{}: malformed line {line:?}", path.display()));
                assert!(
                    entries
                        .insert(key.trim().to_string(), value.trim().to_string())
                        .is_none(),
                    "{}: duplicate key {key}",
                    path.display()
                );
            }
        }
        by_locale
    }

    fn placeholders(value: &str) -> std::collections::BTreeSet<&str> {
        value
            .split("%{")
            .skip(1)
            .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
            .collect()
    }

    #[test]
    fn every_locale_has_exactly_the_english_keys_and_placeholders() {
        let locales = locales();
        let english = locales.get("en").expect("English locale");
        assert!(locales.len() > 1);
        for (locale, entries) in &locales {
            let missing: Vec<_> = english
                .keys()
                .filter(|k| !entries.contains_key(*k))
                .collect();
            let extra: Vec<_> = entries
                .keys()
                .filter(|k| !english.contains_key(*k))
                .collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{locale}: missing {missing:?}, extra {extra:?}"
            );
            for (key, value) in entries {
                assert_eq!(
                    placeholders(value),
                    placeholders(&english[key]),
                    "{locale}: placeholders differ for {key}"
                );
                assert!(value.len() > 2, "{locale}: empty value for {key}");
            }
        }
    }
}
