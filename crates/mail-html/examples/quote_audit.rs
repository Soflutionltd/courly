//! Run the quote detection over a folder of mail bodies, one `.html` file
//! each, and say where it missed the start of a quoted message.
//!
//! Prints counts and the markup around each miss, never the text of a mail.
//!
//! `cargo run --release -p mail-html --example quote_audit -- <folder>`

use std::collections::BTreeMap;
use std::path::PathBuf;

use mail_html::quote_audit::{Miss, audit};

fn main() -> Result<(), String> {
    let folder = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: quote_audit <folder of .html files>")?;
    // One file instead of a folder: every signal of that mail, explained.
    if folder.is_file() {
        let html = std::fs::read_to_string(&folder)
            .map_err(|error| format!("{}: {error}", folder.display()))?;
        for line in mail_html::quote_audit::explain(&html) {
            println!("{line}");
        }
        return Ok(());
    }
    let entries =
        std::fs::read_dir(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;

    let mut mails = 0_usize;
    let mut with_cards = 0_usize;
    let mut cards = 0_usize;
    let mut headers = 0_usize;
    let mut handled = 0_usize;
    let mut mails_with_misses = 0_usize;
    let mut misses: BTreeMap<Miss, (usize, String)> = BTreeMap::new();
    let mut headless: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().is_none_or(|extension| extension != "html") {
            continue;
        }
        let html = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let found = audit(&html);
        mails += 1;
        with_cards += usize::from(found.cards > 0);
        cards += found.cards;
        headers += found.headers;
        handled += found.handled;
        mails_with_misses += usize::from(!found.misses.is_empty());
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        for card in found.headless {
            let seen = headless.entry(card).or_insert((0, name.clone()));
            seen.0 += 1;
        }
        for miss in found.misses {
            let seen = misses.entry(miss).or_insert((0, name.clone()));
            seen.0 += 1;
        }
    }

    println!("mails: {mails}");
    println!("mails with quote cards: {with_cards}");
    println!("quote cards: {cards}, with a header read: {headers}");
    println!("signals answered by a quote: {handled}");
    println!("mails with a missed signal: {mails_with_misses}");
    let mut ranked: Vec<_> = misses.into_iter().collect();
    ranked.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for (miss, (count, example)) in ranked {
        println!(
            "{count:>6}  {:?}  {}  (e.g. {example})",
            miss.signal, miss.place
        );
    }
    println!("cards without a header read, by how they open:");
    let mut ranked: Vec<_> = headless.into_iter().collect();
    ranked.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for (card, (count, example)) in ranked.into_iter().take(40) {
        println!("{count:>6}  {card}  (e.g. {example})");
    }
    Ok(())
}
