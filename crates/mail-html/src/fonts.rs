//! Fonts for mail written in Microsoft Office.
//!
//! Outlook mail names Office fonts (Aptos, Calibri, Cambria...) that macOS
//! does not ship. When Office is installed its own copies are read from the
//! app bundle, only for the families an email asks for. Otherwise open fonts
//! with the same metrics stand in under the Office name, so lines break and
//! space as they do in Outlook.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use blitz_dom::FontContext;
use parley::fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, SourceCache};

/// Where Office apps keep the fonts they load privately.
const OFFICE_FONT_DIRS: &[&str] = &[
    "/Applications/Microsoft Outlook.app/Contents/Resources/DFonts",
    "/Applications/Microsoft Word.app/Contents/Resources/DFonts",
    "/Applications/Microsoft Excel.app/Contents/Resources/DFonts",
    "/Applications/Microsoft PowerPoint.app/Contents/Resources/DFonts",
];

const CARLITO: &[&[u8]] = &[
    include_bytes!("../fonts/carlito/Carlito-Regular.ttf"),
    include_bytes!("../fonts/carlito/Carlito-Bold.ttf"),
    include_bytes!("../fonts/carlito/Carlito-Italic.ttf"),
    include_bytes!("../fonts/carlito/Carlito-BoldItalic.ttf"),
];
const CALADEA: &[&[u8]] = &[
    include_bytes!("../fonts/caladea/Caladea-Regular.ttf"),
    include_bytes!("../fonts/caladea/Caladea-Bold.ttf"),
    include_bytes!("../fonts/caladea/Caladea-Italic.ttf"),
    include_bytes!("../fonts/caladea/Caladea-BoldItalic.ttf"),
];
const SELAWIK: &[&[u8]] = &[
    include_bytes!("../fonts/selawik/selawk.ttf"),
    include_bytes!("../fonts/selawik/selawkb.ttf"),
];
const INCONSOLATA: &[&[u8]] = &[
    include_bytes!("../fonts/inconsolata/Inconsolata-Regular.ttf"),
    include_bytes!("../fonts/inconsolata/Inconsolata-Bold.ttf"),
];

/// Open stand-ins, by the Office family they replace. Aptos has no
/// metric-compatible open clone; Selawik is the closest humanist sans.
const SUBSTITUTES: &[(&str, &[&[u8]])] = &[
    ("Aptos", SELAWIK),
    ("Calibri", CARLITO),
    ("Cambria", CALADEA),
    ("Consolas", INCONSOLATA),
    ("Segoe UI", SELAWIK),
];

struct FontLibrary {
    ctx: FontContext,
    /// Lowercased family name to the Office files that hold it.
    office: HashMap<String, Vec<PathBuf>>,
    /// Lowercased families already settled (registered or found missing).
    settled: HashSet<String>,
}

static LIBRARY: LazyLock<Mutex<FontLibrary>> = LazyLock::new(|| Mutex::new(FontLibrary::new()));

/// A font context holding every family `html` names that this machine can
/// provide, shared across renders.
pub(crate) fn context_for(html: &str) -> FontContext {
    let mut library = LIBRARY.lock().unwrap_or_else(PoisonError::into_inner);
    for family in named_families(html) {
        library.provide(&family);
    }
    library.ctx.clone()
}

impl FontLibrary {
    fn new() -> Self {
        let mut ctx = FontContext {
            source_cache: SourceCache::new_shared(),
            collection: Collection::new(CollectionOptions {
                shared: true,
                system_fonts: true,
            }),
        };
        ctx.collection
            .register_fonts(Blob::new(Arc::new(blitz_dom::BULLET_FONT) as _), None);
        Self {
            ctx,
            office: index_office_fonts(),
            settled: HashSet::new(),
        }
    }

    /// Make `family` available: the installed font first, then Office's
    /// copy, then the open substitute.
    fn provide(&mut self, family: &str) {
        let key = family.to_lowercase();
        if !self.settled.insert(key.clone()) || self.ctx.collection.family_id(family).is_some() {
            return;
        }
        if let Some(paths) = self.office.get(&key) {
            let mut registered = false;
            for path in paths {
                match std::fs::read(path) {
                    Ok(bytes) => {
                        self.ctx
                            .collection
                            .register_fonts(Blob::new(Arc::new(bytes) as _), None);
                        registered = true;
                    }
                    Err(e) => log::warn!("Office font {} unreadable: {e}", path.display()),
                }
            }
            if registered {
                return;
            }
        }
        if let Some((name, faces)) = SUBSTITUTES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(family))
        {
            for face in *faces {
                self.ctx.collection.register_fonts(
                    Blob::new(Arc::new(*face) as _),
                    Some(FontInfoOverride {
                        family_name: Some(name),
                        ..FontInfoOverride::default()
                    }),
                );
            }
        }
    }
}

/// Family names in every Office font file, read once per process.
fn index_office_fonts() -> HashMap<String, Vec<PathBuf>> {
    let mut seen = HashSet::new();
    let mut probe = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let mut index: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for dir in OFFICE_FONT_DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_font = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc")
                });
            if !is_font || !seen.insert(entry.file_name()) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            for (family, _) in probe.register_fonts(Blob::new(Arc::new(bytes) as _), None) {
                if let Some(name) = probe.family_name(family) {
                    index
                        .entry(name.to_lowercase())
                        .or_default()
                        .push(path.clone());
                }
            }
        }
    }
    index
}

/// Font family names used anywhere in `html`: CSS `font-family` values and
/// `<font face>` attributes. Generic families are left out.
fn named_families(html: &str) -> HashSet<String> {
    let html = html.replace("&quot;", "\"").replace("&#39;", "'");
    let lower = html.to_ascii_lowercase();
    let mut families = HashSet::new();
    for marker in ["font-family", "face="] {
        for (found, _) in lower.match_indices(marker) {
            let value = &html[found + marker.len()..];
            let value = &value[..value.find([';', '}', '>', '\n']).unwrap_or(value.len())];
            for token in value.split(',') {
                let token = token.trim_start_matches([':', ' ', '\t']);
                let name = match token.chars().next() {
                    Some(quote @ ('"' | '\'')) => token[1..].split(quote).next(),
                    _ => token.split(['"', '\'']).next(),
                };
                let name = name.unwrap_or_default().trim();
                if !name.is_empty() && !is_generic(name) {
                    families.insert(name.to_string());
                }
            }
        }
    }
    families
}

fn is_generic(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "inherit"
            | "initial"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_read_from_styles_and_font_tags() {
        let families = named_families(
            r#"<div style="font-family: Aptos, &quot;Segoe UI&quot;, sans-serif; color:black">
            <font face="Cambria">x</font><style>p { font-family: 'Calibri' }</style>"#,
        );
        for name in ["Aptos", "Segoe UI", "Cambria", "Calibri"] {
            assert!(families.contains(name), "{name} missing from {families:?}");
        }
        assert!(!families.contains("sans-serif"));
    }

    #[test]
    fn missing_office_families_get_a_stand_in() {
        let mut library = FontLibrary::new();
        library.office.clear();
        library.provide("Aptos");
        assert!(library.ctx.collection.family_id("Aptos").is_some());
    }
}
