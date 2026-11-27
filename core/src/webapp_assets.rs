use std::path::{Path, PathBuf};

use base64::Engine as _;
use sha2::{Digest, Sha256};

pub(crate) const WEBAPP_GENERATED_IMAGE_MAX_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const WEBAPP_GENERATED_IMAGE_URL_PREFIX: &str = "/webapp/vnext/generated-image/";

pub(crate) fn webapp_asset_owner_hash(owner: &str) -> String {
    let digest = Sha256::digest(owner.trim().to_ascii_lowercase().as_bytes());
    hex_prefix(&digest, 16)
}

pub(crate) fn webapp_generated_image_storage_dir(db_path: &str) -> PathBuf {
    if let Ok(path) = std::env::var("AEGIS_WEBAPP_GENERATED_IMAGE_DIR") {
        return path.into();
    }
    Path::new(db_path)
        .parent()
        .map(|parent| parent.join("webapp_generated").join("images"))
        .unwrap_or_else(|| PathBuf::from("webapp_generated/images"))
}

pub(crate) fn is_safe_generated_image_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 180
        && !filename.contains("..")
        && filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && matches!(
            filename.rsplit('.').next(),
            Some("png") | Some("jpg") | Some("jpeg") | Some("webp")
        )
}

pub(crate) fn generated_image_content_type(filename: &str) -> &'static str {
    match filename.rsplit('.').next() {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

pub(crate) fn store_generated_image_for_owner(
    db_path: &str,
    owner: &str,
    image_data: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, String> {
    let (bytes, extension) =
        decode_generated_image_data(image_data, WEBAPP_GENERATED_IMAGE_MAX_BYTES)?;
    let owner_hash = webapp_asset_owner_hash(owner);
    let image_hash = hex_prefix(&Sha256::digest(&bytes), 16);
    let filename = format!(
        "image-{}-{}-{}.{}",
        owner_hash,
        now.timestamp_millis(),
        image_hash,
        extension
    );
    let dir = webapp_generated_image_storage_dir(db_path);
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Unable to prepare generated image storage: {err}"))?;
    let path = dir.join(&filename);
    std::fs::write(&path, bytes).map_err(|err| format!("Unable to save generated image: {err}"))?;
    Ok(format!("{WEBAPP_GENERATED_IMAGE_URL_PREFIX}{filename}"))
}

fn decode_generated_image_data(
    value: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, &'static str), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max_bytes.saturating_mul(2) {
        return Err("Generated image is empty or too large".into());
    }
    let (encoded, extension) = if let Some((prefix, encoded)) = trimmed.split_once(',') {
        let extension = match prefix.to_ascii_lowercase().as_str() {
            "data:image/png;base64" => "png",
            "data:image/jpeg;base64" | "data:image/jpg;base64" => "jpg",
            "data:image/webp;base64" => "webp",
            _ => return Err("Generated image must be PNG, JPEG, or WebP".into()),
        };
        (encoded, extension)
    } else {
        (trimmed, "png")
    };
    if encoded.is_empty()
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err("Generated image data is invalid".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "Generated image data is invalid".to_string())?;
    if bytes.len() > max_bytes {
        return Err("Generated image is too large".into());
    }
    Ok((bytes, extension))
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| {
            let high = byte >> 4;
            let low = byte & 0x0f;
            [hex_digit(high), hex_digit(low)]
        })
        .take(chars)
        .collect()
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + (value - 10)) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_image_storage_returns_owner_scoped_url() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db_path = tmp.path().join("aegis.sqlite");
        let tiny_png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let url = store_generated_image_for_owner(
            db_path.to_str().unwrap(),
            "did:aegis:telegram:123",
            tiny_png,
            chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        )
        .expect("stored");
        assert!(url.starts_with(WEBAPP_GENERATED_IMAGE_URL_PREFIX));
        let filename = url
            .strip_prefix(WEBAPP_GENERATED_IMAGE_URL_PREFIX)
            .expect("filename");
        assert!(is_safe_generated_image_filename(filename));
        assert!(filename.starts_with(&format!(
            "image-{}-",
            webapp_asset_owner_hash("did:aegis:telegram:123")
        )));
        assert!(
            webapp_generated_image_storage_dir(db_path.to_str().unwrap())
                .join(filename)
                .exists()
        );
    }
}
