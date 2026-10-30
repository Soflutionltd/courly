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

pub fn quote_labels() -> QuoteLabels {
    QuoteLabels {
        from: t!("quote.from").into_owned(),
        sent: t!("quote.sent").into_owned(),
        to: t!("quote.to").into_owned(),
        cc: t!("quote.cc").into_owned(),
        subject: t!("quote.subject").into_owned(),
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

    fn locale_keys(path: &std::path::Path) -> std::collections::BTreeSet<String> {
        std::fs::read_to_string(path)
            .expect("locale file")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.starts_with("_version"))
            .filter_map(|line| line.split_once(':').map(|(key, _)| key.trim().to_string()))
            .filter(|key| !key.is_empty() && !key.contains(' '))
            .collect()
    }

    #[test]
    fn every_locale_has_exactly_the_english_keys() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let english = locale_keys(&dir.join("en.yml"));
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("locales dir") {
            let path = entry.expect("entry").path();
            let keys = locale_keys(&path);
            let missing: Vec<_> = english.difference(&keys).collect();
            let extra: Vec<_> = keys.difference(&english).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{}: missing {missing:?}, extra {extra:?}",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 1);
    }
}
