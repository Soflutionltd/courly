//! Images held by the document: the bytes behind a `data:` source, the
//! pixel dimensions read from a file header, and the size an image is shown
//! and resized at.

use base64::Engine as _;

/// Shortest side an image can be resized to (px).
pub const MIN_IMAGE_SIDE: u32 = 16;
/// Longest side an image can be given (px).
pub const MAX_IMAGE_SIDE: u32 = 8192;

/// The bytes of a base64 `data:` image source, `None` for any other source.
pub fn data_uri_bytes(src: &str) -> Option<Vec<u8>> {
    let scheme = src.get(..5)?;
    if !scheme.eq_ignore_ascii_case("data:") {
        return None;
    }
    let (meta, payload) = src[5..].split_once(',')?;
    if !meta.to_ascii_lowercase().ends_with(";base64") {
        return None;
    }
    let payload: String = payload
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(&payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&payload))
        .ok()
}

/// A base64 `data:` source holding `bytes`.
pub fn data_uri(mime: &str, bytes: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

/// Width and height in pixels of a PNG, GIF, JPEG or WebP file, read from
/// its header.
pub fn pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    png_size(bytes)
        .or_else(|| gif_size(bytes))
        .or_else(|| jpeg_size(bytes))
        .or_else(|| webp_size(bytes))
        .filter(|(width, height)| *width > 0 && *height > 0)
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u32> {
    let pair = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_be_bytes([pair[0], pair[1]])))
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u32> {
    let pair = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_le_bytes([pair[0], pair[1]])))
}

fn le_u24(bytes: &[u8], at: usize) -> Option<u32> {
    let triple = bytes.get(at..at + 3)?;
    Some(u32::from_le_bytes([triple[0], triple[1], triple[2], 0]))
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let field = |at: usize| {
        let four = bytes.get(at..at + 4)?;
        Some(u32::from_be_bytes([four[0], four[1], four[2], four[3]]))
    };
    Some((field(16)?, field(20)?))
}

fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a") {
        return None;
    }
    Some((le_u16(bytes, 6)?, le_u16(bytes, 8)?))
}

/// Walk the JPEG segments up to the start-of-frame one, which holds the size.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        match marker {
            // Fill byte before a marker.
            0xFF => at += 1,
            // Markers without a length.
            0x01 | 0xD0..=0xD8 => at += 2,
            // Start of frame, except the tables sharing its range.
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                return Some((be_u16(bytes, at + 7)?, be_u16(bytes, at + 5)?));
            }
            _ => {
                let length = usize::try_from(be_u16(bytes, at + 2)?).ok()?;
                at += 2 + length;
            }
        }
    }
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(b"RIFF") || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    match bytes.get(12..16)? {
        b"VP8X" => Some((le_u24(bytes, 24)? + 1, le_u24(bytes, 27)? + 1)),
        b"VP8 " => {
            if bytes.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            Some((le_u16(bytes, 26)? & 0x3FFF, le_u16(bytes, 28)? & 0x3FFF))
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2F {
                return None;
            }
            let four = bytes.get(21..25)?;
            let bits = u32::from_le_bytes([four[0], four[1], four[2], four[3]]);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        _ => None,
    }
}

/// The size an image is shown at: the size it was given, completed from its
/// own proportions when only one side was, its own size otherwise, and in
/// every case scaled down to fit `available_width`.
pub fn display_size(
    width: Option<u32>,
    height: Option<u32>,
    natural: (u32, u32),
    available_width: f32,
) -> (f32, f32) {
    let natural_width = natural.0.max(1) as f32;
    let natural_height = natural.1.max(1) as f32;
    let (shown_width, shown_height) = match (width, height) {
        (Some(width), Some(height)) => (width as f32, height as f32),
        (Some(width), None) => (width as f32, width as f32 * natural_height / natural_width),
        (None, Some(height)) => (
            height as f32 * natural_width / natural_height,
            height as f32,
        ),
        (None, None) => (natural_width, natural_height),
    };
    let shown_width = shown_width.max(1.0);
    let shown_height = shown_height.max(1.0);
    if available_width >= 1.0 && shown_width > available_width {
        let scale = available_width / shown_width;
        return (available_width, (shown_height * scale).max(1.0));
    }
    (shown_width, shown_height)
}

/// The size an image shown at `start` takes when its corner is dragged by
/// `delta`: its proportions are kept, between [`MIN_IMAGE_SIDE`] and
/// `max_width`.
pub fn resized(start: (f32, f32), delta: (f32, f32), max_width: f32) -> (u32, u32) {
    let start_width = start.0.max(1.0);
    let start_height = start.1.max(1.0);
    let scale =
        ((start_width + delta.0) / start_width).max((start_height + delta.1) / start_height);
    let shortest = MIN_IMAGE_SIDE as f32;
    let min_scale = (shortest / start_width).max(shortest / start_height);
    let max_scale = (max_width / start_width).max(min_scale);
    let scale = scale.clamp(min_scale, max_scale);
    (
        whole_pixels(start_width * scale),
        whole_pixels(start_height * scale),
    )
}

/// `length` rounded to a whole number of pixels, between 1 and
/// [`MAX_IMAGE_SIDE`]. Found by bisection, as the whole numbers in that
/// range all convert to `f32` exactly.
pub fn whole_pixels(length: f32) -> u32 {
    let target = length.round();
    let (mut low, mut high) = (1_u32, MAX_IMAGE_SIDE);
    while low < high {
        let middle = low + (high - low) / 2;
        if (middle as f32) < target {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uri_round_trips_and_other_sources_are_refused() {
        let source = data_uri("image/png", &[0x89, b'P', b'N', b'G']);
        assert_eq!(source, "data:image/png;base64,iVBORw==");
        assert_eq!(data_uri_bytes(&source), Some(vec![0x89, b'P', b'N', b'G']));
        assert_eq!(
            data_uri_bytes("DATA:image/png;BASE64,iVBO\n Rw"),
            Some(vec![0x89, b'P', b'N', b'G'])
        );
        assert_eq!(data_uri_bytes("cid:logo@mail"), None);
        assert_eq!(data_uri_bytes("data:image/svg+xml,%3Csvg%3E"), None);
        assert_eq!(data_uri_bytes("d"), None);
    }

    #[test]
    fn png_and_gif_headers_give_the_size() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&300_u32.to_be_bytes());
        png.extend_from_slice(&120_u32.to_be_bytes());
        assert_eq!(pixel_size(&png), Some((300, 120)));

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&64_u16.to_le_bytes());
        gif.extend_from_slice(&48_u16.to_le_bytes());
        assert_eq!(pixel_size(&gif), Some((64, 48)));
    }

    #[test]
    fn jpeg_size_is_read_past_the_leading_segments() {
        let mut jpeg = vec![0xFF, 0xD8];
        // An application segment of 4 bytes, then a quantization table.
        jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x03, 0x00]);
        // Start of frame: length, precision, height 90, width 160.
        jpeg.extend_from_slice(&[0xFF, 0xC2, 0x00, 0x11, 0x08, 0x00, 0x5A, 0x00, 0xA0]);
        assert_eq!(pixel_size(&jpeg), Some((160, 90)));
        assert_eq!(pixel_size(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]), None);
    }

    #[test]
    fn webp_extended_header_gives_the_size() {
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        webp.extend_from_slice(&[0x8F, 0x01, 0x00]);
        webp.extend_from_slice(&[0xC7, 0x00, 0x00]);
        assert_eq!(pixel_size(&webp), Some((400, 200)));
    }

    #[test]
    fn unknown_bytes_have_no_size() {
        assert_eq!(pixel_size(b"not an image"), None);
        assert_eq!(pixel_size(&[]), None);
    }

    #[test]
    fn display_size_completes_a_missing_side_and_fits_the_width() {
        assert_eq!(display_size(None, None, (200, 100), 600.0), (200.0, 100.0));
        assert_eq!(
            display_size(Some(100), None, (200, 100), 600.0),
            (100.0, 50.0)
        );
        assert_eq!(
            display_size(None, Some(25), (200, 100), 600.0),
            (50.0, 25.0)
        );
        assert_eq!(
            display_size(Some(80), Some(80), (200, 100), 600.0),
            (80.0, 80.0)
        );
        assert_eq!(display_size(None, None, (1200, 600), 300.0), (300.0, 150.0));
    }

    #[test]
    fn resizing_keeps_the_proportions_inside_the_limits() {
        assert_eq!(resized((200.0, 100.0), (100.0, 0.0), 600.0), (300, 150));
        assert_eq!(resized((200.0, 100.0), (-100.0, -50.0), 600.0), (100, 50));
        // Never under the shortest side, never wider than the editor.
        assert_eq!(resized((200.0, 100.0), (-500.0, -500.0), 600.0), (32, 16));
        assert_eq!(resized((200.0, 100.0), (900.0, 0.0), 600.0), (600, 300));
    }

    #[test]
    fn whole_pixels_rounds_inside_the_range() {
        assert_eq!(whole_pixels(149.6), 150);
        assert_eq!(whole_pixels(0.2), 1);
        assert_eq!(whole_pixels(-40.0), 1);
        assert_eq!(whole_pixels(90_000.0), MAX_IMAGE_SIDE);
    }
}
