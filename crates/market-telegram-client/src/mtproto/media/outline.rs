//! Telegram's compact photoPathSize encoding; no file download or animation renderer.
use grammers_client::tl;
use market_media::{MediaAsset, MediaError, MediaResult};
pub(super) fn from_document(document: &tl::enums::Document) -> MediaResult {
    let tl::enums::Document::Document(document) = document else {
        return Err(MediaError::NotFound);
    };
    let outline = document
        .thumbs
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find_map(|size| {
            if let tl::enums::PhotoSize::PhotoPathSize(outline) = size {
                Some(outline)
            } else {
                None
            }
        })
        .ok_or(MediaError::NotFound)?;
    MediaAsset::placeholder(&decode(&outline.bytes)?)
}
fn decode(bytes: &[u8]) -> Result<String, MediaError> {
    if bytes.is_empty() {
        return Err(MediaError::NotFound);
    }
    if bytes.len() > 1024 {
        return Err(MediaError::InvalidMedia);
    }
    // https://core.telegram.org/api/files#vector-thumbnails
    const LOOKUP: &[u8; 64] = b"AACAAAAHAAALMAAAQASTAVAAAZaacaaaahaaalmaaaqastava.az0123456789-,";
    let mut path = String::from("M");
    for &byte in bytes {
        if byte >= 192 {
            path.push(char::from(LOOKUP[usize::from(byte - 192)]));
        } else {
            if byte >= 128 {
                path.push(',');
            } else if byte >= 64 {
                path.push('-');
            }
            // Coordinates are decimal numbers, not character codes.
            path.push_str(&(byte & 63).to_string());
        }
    }
    path.push('z');
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_numbers_signs_separators_and_commands_decode_as_decimal_paths() {
        // M12,34 l-5,6 z
        let packed = [12, 128 + 34, 192 + 37, 64 + 5, 128 + 6];
        assert_eq!(decode(&packed).unwrap(), "M12,34l-5,6z");
        let mask = MediaAsset::placeholder(&decode(&packed).unwrap()).unwrap();
        assert!(std::str::from_utf8(mask.bytes()).unwrap().contains("<path"));
        let document = tl::types::Document {
            id: 42,
            access_hash: 0,
            file_reference: vec![],
            date: 0,
            mime_type: "application/x-tgsticker".into(),
            size: 1,
            dc_id: 2,
            attributes: vec![],
            video_thumbs: None,
            thumbs: Some(vec![
                tl::types::PhotoPathSize {
                    r#type: "j".into(),
                    bytes: packed.to_vec(),
                }
                .into(),
            ]),
        };
        assert_eq!(
            from_document(&document.into()).unwrap().bytes(),
            mask.bytes()
        );
        assert_eq!(decode(&[]).unwrap_err(), MediaError::NotFound);
        assert_eq!(
            decode(&vec![0; 1025]).unwrap_err(),
            MediaError::InvalidMedia
        );
    }
    #[test]
    fn absent_outlines_remain_optional() {
        assert_eq!(
            from_document(&tl::types::DocumentEmpty { id: 42 }.into()).unwrap_err(),
            MediaError::NotFound
        );
    }
}
