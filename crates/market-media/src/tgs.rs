//! Validate the compressed animation before caching or passing it to a browser renderer.
use crate::{MAX_ANIMATION_BYTES, MAX_TGS_JSON_BYTES, MediaError};
use std::io::Read;
pub(crate) fn validate(bytes: &[u8]) -> Result<(), MediaError> {
    let json = decode(bytes)?;
    parse_json(&json).map(|_| ())
}
pub(crate) fn decode(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    let invalid = MediaError::InvalidMedia;
    if bytes.len() > MAX_ANIMATION_BYTES || !bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(invalid);
    }
    let mut json = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .take(MAX_TGS_JSON_BYTES as u64 + 1)
        .read_to_end(&mut json)
        .map_err(|_| invalid)?;
    if json.len() > MAX_TGS_JSON_BYTES {
        return Err(invalid);
    }
    Ok(json)
}
pub(crate) fn parse_json(json: &[u8]) -> Result<serde_json::Value, MediaError> {
    let invalid = MediaError::InvalidMedia;
    if json.len() > MAX_TGS_JSON_BYTES {
        return Err(invalid);
    }
    let data: serde_json::Value = serde_json::from_slice(json).map_err(|_| invalid)?;
    let number = |key: &str| data[key].as_f64().ok_or(invalid);
    let (width, height, fps, start, end) = (
        number("w")?,
        number("h")?,
        number("fr")?,
        number("ip")?,
        number("op")?,
    );
    if !(1.0..=1024.0).contains(&width)
        || !(1.0..=1024.0).contains(&height)
        || !(1.0..=120.0).contains(&fps)
        || start < 0.0
        || end <= start
        || end - start > fps * 10.0
        || !data["v"].is_string()
        || !data["layers"].is_array()
    {
        return Err(invalid);
    }
    // Telegram TGS uses self-contained vector shapes. No external assets, fonts or expressions.
    let mut pending = vec![&data];
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Object(object) => {
                if ["p", "u", "x"]
                    .iter()
                    .any(|key| object.get(*key).is_some_and(serde_json::Value::is_string))
                    || object.contains_key("fonts")
                    || object.contains_key("chars")
                {
                    return Err(invalid);
                }
                pending.extend(object.values());
            }
            serde_json::Value::Array(array) => pending.extend(array),
            _ => {}
        }
    }
    Ok(data)
}
