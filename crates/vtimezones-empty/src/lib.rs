//! Empty stand-in for `vtimezones-rs` (GPL-3.0), patched in by the
//! workspace manifest.
//!
//! `caldata` reads `VTIMEZONES` only when a parser runs with
//! `ParserOptions { rfc7809: true }` (timezones by reference). Bifrost always
//! parses with the default `rfc7809: false`, so the table is never consulted
//! and an empty one keeps behaviour unchanged. With an empty table,
//! `IcalTimeZone::from_tzid` returns `None` and caldata keeps the calendar's
//! own VTIMEZONE components.

pub static VTIMEZONES: phf::Map<&'static str, &'static str> = phf::Map::new();

pub const IANA_TZDB_VERSION: &str = "none";

#[cfg(test)]
mod tests {
    #[test]
    fn table_is_empty() {
        assert!(super::VTIMEZONES.is_empty());
        assert!(super::VTIMEZONES.get("Europe/Paris").is_none());
    }
}
