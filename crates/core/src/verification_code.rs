//! Finds the one-time code in a verification email, so the reader can copy
//! it with one click.
//!
//! The rule is deliberately strict: a wrong number pasted into a login form
//! is worse than no button. A code is reported only when the email names a
//! code, speaks of signing in or verifying, and one number stands clearly
//! closest to the word.

/// How far after the word "code" its number may sit, in bytes.
const REACH_AFTER: usize = 120;
/// How far before it ("482913 is your code").
const REACH_BEFORE: usize = 40;
/// Two different numbers this close in distance cannot be told apart.
const TIE_MARGIN: usize = 8;
/// Words on each side of "code" read for what kind of code it is.
const NEIGHBOURS: usize = 2;

/// Compounds ending in "code" that are not codes to type.
const NOT_A_CODE: &[&str] = &[
    "barcode",
    "qrcode",
    "unicode",
    "postcode",
    "zipcode",
    "promocode",
];

/// A code named next to one of these is a voucher, an address or a
/// reference, not a one-time code.
const OTHER_KINDS: &[&str] = &[
    "promo",
    "promotion",
    "promotionnel",
    "postal",
    "postale",
    "zip",
    "réduction",
    "reduction",
    "coupon",
    "discount",
    "voucher",
    "sconto",
    "fiscale",
    "fiscal",
    "descuento",
    "rabatt",
    "gutschein",
    "parrainage",
    "referral",
    "client",
    "suivi",
    "tracking",
    "barres",
];

/// Stems that say the email is about proving who is signing in.
const CONTEXT_STEMS: &[&str] = &[
    "vérif",
    "verif",
    "authenti",
    "autenti",
    "connexion",
    "login",
    "accesso",
    "anmeld",
    "sécurité",
    "securit",
    "sicurezza",
    "seguridad",
    "sicherheit",
    "confirm",
    "conferm",
    "bestätig",
    "identi",
    "einmal",
    "2fa",
    "otp",
    "passcode",
];

struct Word<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

fn words(text: &str) -> Vec<Word<'_>> {
    let mut found = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        match (character.is_alphanumeric(), start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                found.push(Word {
                    start: from,
                    end: index,
                    text: &text[from..index],
                });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        found.push(Word {
            start: from,
            end: text.len(),
            text: &text[from..],
        });
    }
    found
}

/// Whether a lowercased word names a code to type.
fn is_code_word(word: &str) -> bool {
    matches!(
        word,
        "codes" | "codice" | "código" | "codigo" | "otp" | "pin"
    ) || (word.ends_with("code") && !NOT_A_CODE.contains(&word))
}

/// Whether a lowercased word says the email is about signing in. A compound
/// such as "Sicherheitscode" carries its own context.
fn is_context_word(word: &str) -> bool {
    CONTEXT_STEMS.iter().any(|stem| word.starts_with(stem))
        || (word.ends_with("code") && word.len() > "code".len() && !NOT_A_CODE.contains(&word))
}

fn is_digits(word: &str) -> bool {
    word.bytes().all(|byte| byte.is_ascii_digit())
}

/// A mixed code such as `G7K2Q9`: capitals and digits, both present.
fn is_mixed_code(word: &str) -> bool {
    (6..=8).contains(&word.len())
        && word
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase())
        && word.bytes().filter(u8::is_ascii_digit).count() >= 2
        && word.bytes().any(|byte| byte.is_ascii_uppercase())
}

fn is_year(digits: &str) -> bool {
    digits.len() == 4
        && digits
            .parse::<u16>()
            .is_ok_and(|year| (1900..=2100).contains(&year))
}

/// Whether the text around `start..end` makes it part of something else: an
/// amount, a decimal, a time, an address or a link.
fn is_embedded(text: &str, start: usize, end: usize) -> bool {
    let mut before = text[..start].chars().rev();
    let previous = before.next();
    let earlier = before.next();
    let mut after = text[end..].chars();
    let next = after.next();
    let later = after.next();

    let glued_before = match previous {
        Some('.' | ',' | ':') => earlier.is_some_and(|c| c.is_ascii_digit()),
        Some(c) => "/+-_=#@€$£".contains(c),
        None => false,
    };
    let glued_after = match next {
        Some('.' | ',' | ':') => later.is_some_and(|c| c.is_ascii_digit()),
        Some(' ') => later.is_some_and(|c| "€$£%".contains(c)),
        Some(c) => "/_=@€$£%-".contains(c),
        None => false,
    };
    let money_before = previous == Some(' ') && earlier.is_some_and(|c| "€$£".contains(c));

    let chunk_start = text[..start]
        .rfind(char::is_whitespace)
        .map_or(0, |index| index + 1);
    let chunk_end = text[end..]
        .find(char::is_whitespace)
        .map_or(text.len(), |index| end + index);
    let chunk = &text[chunk_start..chunk_end];
    let in_link = chunk.contains("://") || chunk.contains("www.") || chunk.contains('@');

    glued_before || glued_after || money_before || in_link
}

struct Candidate {
    start: usize,
    end: usize,
    code: String,
}

/// Every number or mixed code that could be a one-time code.
fn candidates(text: &str, words: &[Word<'_>]) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        if !is_digits(word.text) {
            if is_mixed_code(word.text) && !is_embedded(text, word.start, word.end) {
                found.push(Candidate {
                    start: word.start,
                    end: word.end,
                    code: word.text.to_string(),
                });
            }
            index += 1;
            continue;
        }
        // A run of digit groups joined by single spaces or hyphens: one group
        // is a plain code, two equal groups a split code ("123 456"), and
        // anything longer a phone number, a date or a card number.
        let mut last = index;
        while let Some(following) = words.get(last + 1)
            && is_digits(following.text)
            && matches!(&text[words[last].end..following.start], " " | "-")
        {
            last += 1;
        }
        let run = &words[index..=last];
        let code = match run {
            [single] if (4..=8).contains(&single.text.len()) && !is_year(single.text) => {
                Some(single.text.to_string())
            }
            [left, right]
                if left.text.len() == right.text.len() && matches!(left.text.len(), 3 | 4) =>
            {
                Some(format!("{}{}", left.text, right.text))
            }
            _ => None,
        };
        let (start, end) = (run[0].start, run[run.len() - 1].end);
        if let Some(code) = code
            && !is_embedded(text, start, end)
        {
            found.push(Candidate { start, end, code });
        }
        index = last + 1;
    }
    found
}

/// The one-time code of a verification email, without separators, or `None`
/// when the text is not clearly such an email.
pub fn find(text: &str) -> Option<String> {
    let words = words(text);
    let lowered: Vec<String> = words.iter().map(|word| word.text.to_lowercase()).collect();
    if !lowered.iter().any(|word| is_context_word(word)) {
        return None;
    }
    let anchors: Vec<&Word<'_>> = lowered
        .iter()
        .enumerate()
        .filter(|(index, word)| {
            let from = index.saturating_sub(NEIGHBOURS);
            let to = (index + NEIGHBOURS + 1).min(lowered.len());
            is_code_word(word)
                && !lowered[from..to]
                    .iter()
                    .any(|near| OTHER_KINDS.contains(&near.as_str()))
        })
        .map(|(index, _)| &words[index])
        .collect();

    // Each candidate's distance to the nearest word naming a code.
    let mut ranked: Vec<(usize, String)> = Vec::new();
    for candidate in candidates(text, &words) {
        let distance = anchors
            .iter()
            .filter_map(|anchor| {
                if candidate.start >= anchor.end {
                    Some(candidate.start - anchor.end).filter(|gap| *gap <= REACH_AFTER)
                } else if anchor.start >= candidate.end {
                    Some(anchor.start - candidate.end).filter(|gap| *gap <= REACH_BEFORE)
                } else {
                    None
                }
            })
            .min();
        let Some(distance) = distance else {
            continue;
        };
        match ranked.iter_mut().find(|(_, code)| *code == candidate.code) {
            Some(entry) => entry.0 = entry.0.min(distance),
            None => ranked.push((distance, candidate.code)),
        }
    }
    ranked.sort_by_key(|(distance, _)| *distance);
    let mut ranked = ranked.into_iter();
    let (best, code) = ranked.next()?;
    match ranked.next() {
        Some((runner_up, _)) if runner_up <= best + TIE_MARGIN => None,
        _ => Some(code),
    }
}

#[cfg(test)]
mod tests {
    use super::find;

    #[test]
    fn finds_the_code_of_a_french_verification_email() {
        let text = "Bonjour,\n\nVotre code de vérification est : 482913\n\n\
                    Il expire dans 10 minutes.";
        assert_eq!(find(text).as_deref(), Some("482913"));
    }

    #[test]
    fn finds_the_code_in_other_languages() {
        assert_eq!(
            find("Your verification code is 731204. Do not share it.").as_deref(),
            Some("731204")
        );
        assert_eq!(
            find("Il tuo codice di verifica è 905512.").as_deref(),
            Some("905512")
        );
        assert_eq!(
            find("Tu código de verificación: 118734").as_deref(),
            Some("118734")
        );
        assert_eq!(
            find("Ihr Sicherheitscode lautet 660391.").as_deref(),
            Some("660391")
        );
    }

    #[test]
    fn reads_a_code_written_before_the_word() {
        let text = "Connexion à votre compte\n\n204817\n\nCe code est valable 5 minutes.";
        assert_eq!(find(text).as_deref(), Some("204817"));
    }

    #[test]
    fn joins_a_code_split_in_two_groups() {
        assert_eq!(find("Your login code: 123 456").as_deref(), Some("123456"));
        assert_eq!(
            find("Sign-in verification code 4821-9930").as_deref(),
            Some("48219930")
        );
    }

    #[test]
    fn reads_a_mixed_letter_and_digit_code() {
        assert_eq!(
            find("Enter this security code to confirm: G7K2Q9").as_deref(),
            Some("G7K2Q9")
        );
    }

    #[test]
    fn takes_the_number_next_to_the_word_over_a_later_one() {
        let text = "Votre code de connexion : 558201\n\n\
                    Besoin d'aide ? Rappelez la référence de dossier 99120044 à notre équipe \
                    d'assistance, disponible du lundi au vendredi.";
        assert_eq!(find(text).as_deref(), Some("558201"));
    }

    #[test]
    fn ignores_emails_that_are_not_about_signing_in() {
        assert_eq!(find("Votre commande 482913 a été expédiée."), None);
        assert_eq!(
            find("Facture 20261118 : merci de régler 1250 € sous 30 jours."),
            None
        );
    }

    #[test]
    fn ignores_vouchers_and_postal_codes() {
        assert_eq!(
            find("Confirmez votre panier avec le code promo 202511 avant dimanche."),
            None
        );
        assert_eq!(
            find("Vérifiez votre adresse : code postal 75011, Paris."),
            None
        );
    }

    #[test]
    fn ignores_years_amounts_times_phones_and_links() {
        assert_eq!(find("Code de vérification envoyé en 2026."), None);
        assert_eq!(find("Sécurité : code facturé 1299,00 EUR."), None);
        assert_eq!(find("Verification code sent at 14:3055."), None);
        assert_eq!(find("Login code? Call 01 23 45 67 89."), None);
        assert_eq!(
            find("Confirm your code at https://example.com/verify/482913"),
            None
        );
    }

    #[test]
    fn gives_up_when_two_numbers_are_equally_close() {
        assert_eq!(find("Verification code\n482913\n771204"), None);
    }
}
