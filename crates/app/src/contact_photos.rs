//! The photos the user picked for their correspondents.
//!
//! A photo is a small square PNG in the data directory, named after a hash
//! of the address it belongs to. It is kept out of `contacts.avatar_url` on
//! purpose: contact sync owns that column and rewrites it with whatever the
//! address book publishes, and a photo keyed on the address also serves a
//! sender who is in no address book.
//!
//! Views read the photos from memory: the files are read once at boot and
//! after each change, never while drawing.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, PoisonError, RwLock};

use iced::widget::image::Handle;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

const DIRECTORY: &str = "contact_photos";
const EXTENSION: &str = "png";
/// Side of a stored photo, in pixels: twice the largest avatar on a
/// double-density display.
const PHOTO_SIDE: u32 = 256;

/// The image files the picker offers.
pub const PICKABLE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

static PHOTOS: LazyLock<RwLock<HashMap<String, Handle>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// The file name stem of an address's photo. `None` for an empty address.
fn key_of(address: &str) -> Option<String> {
    let normalized = address.trim().to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(blake3::hash(normalized.as_bytes()).to_hex().to_string())
}

fn photo_file(app_data_dir: &Path, key: &str) -> PathBuf {
    app_data_dir
        .join(DIRECTORY)
        .join(format!("{key}.{EXTENSION}"))
}

/// Read every stored photo into memory. Returns how many there are.
pub fn load(app_data_dir: &Path) -> Result<usize, String> {
    let directory = app_data_dir.join(DIRECTORY);
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("reading {}: {error}", directory.display())),
    };
    let mut loaded = HashMap::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("reading {}: {error}", directory.display()))?
            .path();
        if path.extension().and_then(|ext| ext.to_str()) != Some(EXTENSION) {
            continue;
        }
        let Some(key) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let bytes =
            std::fs::read(&path).map_err(|error| format!("reading {}: {error}", path.display()))?;
        loaded.insert(key.to_owned(), Handle::from_bytes(bytes));
    }
    let count = loaded.len();
    *PHOTOS.write().unwrap_or_else(PoisonError::into_inner) = loaded;
    Ok(count)
}

/// The photo the user picked for this address.
pub fn photo_of(address: &str) -> Option<Handle> {
    let photos = PHOTOS.read().unwrap_or_else(PoisonError::into_inner);
    if photos.is_empty() {
        return None;
    }
    photos.get(&key_of(address)?).cloned()
}

/// Make `source` the photo of `address`: cropped to a centered square,
/// reduced, and stored in the data directory. Blocking.
pub fn set_from_file(app_data_dir: &Path, address: &str, source: &Path) -> Result<(), String> {
    let key = key_of(address).ok_or("a photo needs an email address")?;
    let png = square_png(source)?;

    let destination = photo_file(app_data_dir, &key);
    let directory = app_data_dir.join(DIRECTORY);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("creating {}: {error}", directory.display()))?;
    // Written beside the destination, then renamed: a crash never leaves a
    // half-written photo under the final name.
    let staging = destination.with_extension("tmp");
    std::fs::write(&staging, &png)
        .map_err(|error| format!("writing {}: {error}", staging.display()))?;
    std::fs::rename(&staging, &destination)
        .map_err(|error| format!("writing {}: {error}", destination.display()))?;

    PHOTOS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, Handle::from_bytes(png));
    Ok(())
}

/// Forget the photo of `address`. Not an error when it has none.
pub fn remove(app_data_dir: &Path, address: &str) -> Result<(), String> {
    let Some(key) = key_of(address) else {
        return Ok(());
    };
    let file = photo_file(app_data_dir, &key);
    match std::fs::remove_file(&file) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("removing {}: {error}", file.display())),
    }
    PHOTOS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&key);
    Ok(())
}

/// Decode an image file, turn it the way its EXIF orientation says (phone
/// photos are stored sideways), and encode a `PHOTO_SIDE` square PNG.
fn square_png(source: &Path) -> Result<Vec<u8>, String> {
    // The file's name, not its path: the message is shown to the user.
    let name = source
        .file_name()
        .map_or_else(|| source.to_string_lossy(), |name| name.to_string_lossy());
    let unreadable = |error: &dyn std::fmt::Display| format!("{name}: {error}");
    let mut decoder = ImageReader::open(source)
        .map_err(|error| unreadable(&error))?
        .with_guessed_format()
        .map_err(|error| unreadable(&error))?
        .into_decoder()
        .map_err(|error| unreadable(&error))?;
    let orientation = decoder.orientation().map_err(|error| unreadable(&error))?;
    let mut picture = DynamicImage::from_decoder(decoder).map_err(|error| unreadable(&error))?;
    picture.apply_orientation(orientation);

    let square = picture.resize_to_fill(PHOTO_SIDE, PHOTO_SIDE, FilterType::Lanczos3);
    let mut png = Cursor::new(Vec::new());
    square
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| format!("encoding the photo: {error}"))?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide_picture(directory: &Path) -> PathBuf {
        let path = directory.join("wide.png");
        image::RgbImage::from_pixel(600, 200, image::Rgb([12, 127, 125]))
            .save(&path)
            .expect("write the source picture");
        path
    }

    #[test]
    fn a_picked_photo_is_stored_square_and_found_whatever_the_case_of_the_address() {
        let data = tempfile::tempdir().expect("temp dir");
        let source = wide_picture(data.path());

        set_from_file(data.path(), " Stored.Square@Example.com ", &source).expect("set photo");

        let key = key_of("stored.square@example.com").expect("key");
        let stored = image::open(photo_file(data.path(), &key)).expect("stored photo decodes");
        assert_eq!((stored.width(), stored.height()), (PHOTO_SIDE, PHOTO_SIDE));
        assert!(photo_of("stored.square@example.com").is_some());
    }

    #[test]
    fn a_removed_photo_is_gone_from_disk_and_from_memory() {
        let data = tempfile::tempdir().expect("temp dir");
        let source = wide_picture(data.path());
        set_from_file(data.path(), "removed@example.com", &source).expect("set photo");

        remove(data.path(), "removed@example.com").expect("remove photo");

        let key = key_of("removed@example.com").expect("key");
        assert!(!photo_file(data.path(), &key).exists());
        assert!(photo_of("removed@example.com").is_none());
        remove(data.path(), "removed@example.com").expect("removing twice is not an error");
    }

    #[test]
    fn a_file_that_is_not_an_image_is_refused_and_stores_nothing() {
        let data = tempfile::tempdir().expect("temp dir");
        let source = data.path().join("notes.png");
        std::fs::write(&source, b"not an image").expect("write the source file");

        let refused = set_from_file(data.path(), "refused@example.com", &source);

        assert!(refused.is_err());
        assert!(photo_of("refused@example.com").is_none());
        assert!(!data.path().join(DIRECTORY).exists());
    }

    #[test]
    fn an_empty_address_has_no_photo() {
        assert!(key_of("   ").is_none());
        assert!(photo_of("").is_none());
    }
}
