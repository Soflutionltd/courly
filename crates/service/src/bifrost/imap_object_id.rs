//! IMAP message object ids as bifrost-imap mints them.
//!
//! The wire shape is `imap2:<len>:<folder>:<uidvalidity>:<uid>`, where
//! `<folder>` is the mailbox's account id (the same string bifrost uses as the
//! container id) and `<len>` its byte length. bifrost still decodes the legacy
//! `imap1:` prefix, whose folder component is the literal mailbox name, so
//! both are accepted here. Every id ratatoskr hands back to bifrost is encoded
//! with the current `imap2:` prefix.

const CURRENT_PREFIX: &str = "imap2:";
const LEGACY_PREFIX: &str = "imap1:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedImapObjectId {
    pub(crate) folder: String,
    pub(crate) uidvalidity: u32,
    pub(crate) uid: u32,
}

/// Encode the object id bifrost-imap expects for `(folder, uidvalidity, uid)`.
/// `folder` is the stored `imap_folder`, which is the folder component a
/// previous decode produced.
pub(crate) fn encode_imap_object_id(folder: &str, uidvalidity: i64, uid: i64) -> String {
    format!(
        "{CURRENT_PREFIX}{}:{folder}:{uidvalidity}:{uid}",
        folder.len()
    )
}

pub(crate) fn decode_imap_object_id(id: &str) -> Option<DecodedImapObjectId> {
    let rest = id
        .strip_prefix(CURRENT_PREFIX)
        .or_else(|| id.strip_prefix(LEGACY_PREFIX))?;
    let (len_text, after_len) = rest.split_once(':')?;
    let len = len_text.parse::<usize>().ok()?;
    if after_len.len() < len {
        return None;
    }
    let folder = after_len.get(..len)?.to_string();
    let rest = after_len.get(len..)?.strip_prefix(':')?;
    let mut parts = rest.split(':');
    let uidvalidity = parts.next()?.parse::<u32>().ok()?;
    let uid = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(DecodedImapObjectId {
        folder,
        uidvalidity,
        uid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_and_legacy_prefixes_decode_to_the_same_coordinates() {
        let expected = DecodedImapObjectId {
            folder: "INBOX".to_string(),
            uidvalidity: 7,
            uid: 42,
        };
        assert_eq!(
            decode_imap_object_id("imap2:5:INBOX:7:42"),
            Some(expected.clone())
        );
        assert_eq!(decode_imap_object_id("imap1:5:INBOX:7:42"), Some(expected));
    }

    #[test]
    fn encode_uses_the_current_prefix_and_roundtrips() {
        let id = encode_imap_object_id("Archive:2026", 7, 42);
        assert_eq!(id, "imap2:12:Archive:2026:7:42");
        let decoded = decode_imap_object_id(&id).expect("roundtrip");
        assert_eq!(decoded.folder, "Archive:2026");
        assert_eq!((decoded.uidvalidity, decoded.uid), (7, 42));
    }

    #[test]
    fn foreign_and_malformed_ids_do_not_decode() {
        for id in [
            "imapthread2:5:INBOX:7:42",
            "imap3:5:INBOX:7:42",
            "imap2:9:INBOX:7:42",
            "imap2:5:INBOX:7",
            "imap2:5:INBOX:7:42:1",
            "m1",
        ] {
            assert_eq!(decode_imap_object_id(id), None, "{id}");
        }
    }
}
