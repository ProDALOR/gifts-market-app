//! Validate bounded vector animations and discard expression metadata, never execute it.
use crate::{MAX_ANIMATION_BYTES, MAX_TGS_JSON_BYTES, MediaError};
use serde_json::Value;
use std::io::{Read, Write};

fn invalid(reason: &'static str) -> MediaError {
    tracing::warn!(stage = "tgs_validation", reason, "animation rejected");
    MediaError::InvalidMedia
}

/// Keep ordinary originals byte-for-byte. Remove expression metadata beside baked
/// keyframes and supply empty shape arrays before caching or serving any client.
pub(crate) fn normalize(bytes: Vec<u8>) -> Result<Vec<u8>, MediaError> {
    let (data, changes) = parse(&decode(&bytes)?)?;
    if changes == 0 {
        return Ok(bytes);
    }
    let json = serde_json::to_vec(&data).map_err(|_| invalid("encoding"))?;
    if json.len() > MAX_TGS_JSON_BYTES {
        return Err(invalid("expanded_size"));
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&json).map_err(|_| invalid("encoding"))?;
    let bytes = encoder.finish().map_err(|_| invalid("encoding"))?;
    if bytes.len() > MAX_ANIMATION_BYTES {
        return Err(invalid("compressed_size"));
    }
    tracing::debug!(
        normalized_fields = changes,
        "using normalized animation keyframes and layer defaults"
    );
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    if bytes.len() > MAX_ANIMATION_BYTES {
        return Err(invalid("compressed_size"));
    }
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(invalid("gzip"));
    }
    let mut json = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .take(MAX_TGS_JSON_BYTES as u64 + 1)
        .read_to_end(&mut json)
        .map_err(|_| invalid("gzip"))?;
    if json.len() > MAX_TGS_JSON_BYTES {
        return Err(invalid("expanded_size"));
    }
    Ok(json)
}

pub(crate) fn parse_json(json: &[u8]) -> Result<Value, MediaError> {
    parse(json).map(|(data, _)| data)
}

fn baked_value(value: &Value) -> bool {
    value.is_number()
        || value.as_array().is_some_and(|values| !values.is_empty())
        || value.as_object().is_some_and(|values| !values.is_empty())
}
fn baked_property(property: &serde_json::Map<String, Value>) -> bool {
    let Some(value) = property.get("k") else {
        return false;
    };
    match property.get("a").and_then(Value::as_u64) {
        Some(0) => baked_value(value),
        Some(1) => value.as_array().is_some_and(|frames| {
            frames
                .first()
                .and_then(|frame| frame.get("s"))
                .is_some_and(baked_value)
                && frames
                    .iter()
                    .all(|frame| frame.get("t").is_some_and(Value::is_number))
        }),
        _ => false,
    }
}

fn parse(json: &[u8]) -> Result<(Value, usize), MediaError> {
    if json.len() > MAX_TGS_JSON_BYTES {
        return Err(invalid("expanded_size"));
    }
    let mut data: Value = serde_json::from_slice(json).map_err(|_| invalid("json"))?;
    let number = |key: &str| data[key].as_f64().ok_or_else(|| invalid("composition"));
    let (width, height, fps, start, end) = (
        number("w")?,
        number("h")?,
        number("fr")?,
        number("ip")?,
        number("op")?,
    );
    if !(1.0..=1024.0).contains(&width) || !(1.0..=1024.0).contains(&height) {
        return Err(invalid("dimensions"));
    }
    if !(1.0..=120.0).contains(&fps) {
        return Err(invalid("frame_rate"));
    }
    if start < 0.0 || end <= start || end - start > fps * 10.0 {
        return Err(invalid("frame_range"));
    }
    if !data["v"].is_string() || !data["layers"].is_array() {
        return Err(invalid("composition"));
    }
    let mut changes = 0;
    let mut pending = vec![&mut data];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(object) => {
                if ["p", "u"]
                    .iter()
                    .any(|key| object.get(*key).is_some_and(Value::is_string))
                {
                    return Err(invalid("external_asset"));
                }
                if object.contains_key("fonts") || object.contains_key("chars") {
                    return Err(invalid("fonts"));
                }
                if object.get("x").is_some_and(Value::is_string) {
                    // This is an animated Lottie property, not arbitrary script-bearing data.
                    // A numeric/vector/path fallback is required; expression-only input stays invalid.
                    if !baked_property(object) {
                        return Err(invalid("expression_without_keyframes"));
                    }
                    object.remove("x");
                    changes += 1;
                }
                // Retain empty shape layers as transform parents. Both Velato and
                // Lottie-web require an explicit array even when there is no artwork.
                if object.get("ty").is_some_and(|ty| ty == 4)
                    && object.contains_key("ks")
                    && !object.contains_key("shapes")
                {
                    object.insert("shapes".into(), Value::Array(Vec::new()));
                    changes += 1;
                }
                pending.extend(object.values_mut());
            }
            Value::Array(array) => pending.extend(array),
            _ => {}
        }
    }
    Ok((data, changes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MediaAsset, first_frame_svg};
    use serde_json::json;
    use std::io::Write;

    fn compressed(data: &Value) -> Vec<u8> {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&serde_json::to_vec(data).unwrap()).unwrap();
        gzip.finish().unwrap()
    }
    fn fixture() -> Value {
        json!({"v":"5.5.2","w":512,"h":512,"fr":60,"ip":0,"op":180,"assets":[],"layers":[{
            "ty":4,"ind":1,"ip":0,"op":180,"st":0,"ks":{},"shapes":[
                {"ty":"sh","ks":{"a":1,"k":[{"t":0,"s":[{"i":[[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0]],"v":[[100,100],[300,100],[300,300]],"c":true}]}],
                    "x":"var $bm_rt; $bm_rt = thisComp.layer('klub').content('Group 5').content('Path 1').path;"}},
                {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1}]}]})
    }
    #[test]
    fn empty_shape_layers_are_normalized_once_without_dropping_parent_identity() {
        let mut source = fixture();
        source["layers"][0]["shapes"][0]["ks"]
            .as_object_mut()
            .unwrap()
            .remove("x");
        source["layers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"ty":4,"ind":2,"ip":0,"op":180,"ks":{"p":{"a":0,"k":[10,20]}}}));
        let bytes = compressed(&source);
        let normalized = normalize(bytes.clone()).unwrap();
        assert_ne!(bytes, normalized);
        source["layers"][1]["shapes"] = json!([]);
        assert_eq!(
            serde_json::from_slice::<Value>(&decode(&normalized).unwrap()).unwrap(),
            source
        );
        assert_eq!(normalize(normalized.clone()).unwrap(), normalized);
        assert!(first_frame_svg(&normalized).is_ok());
    }
    #[test]
    fn baked_wise_owl_path_removes_expression_without_altering_keyframes() {
        let source = fixture();
        let mut expected = source.clone();
        expected["layers"][0]["shapes"][0]["ks"]
            .as_object_mut()
            .unwrap()
            .remove("x");
        let raw = compressed(&source);
        let asset = MediaAsset::animation(raw.clone()).unwrap();
        assert_ne!(asset.bytes(), raw);
        assert_eq!(
            parse_json(&decode(asset.bytes()).unwrap()).unwrap(),
            expected
        );
        assert!(
            !String::from_utf8(decode(asset.bytes()).unwrap())
                .unwrap()
                .contains("$bm_rt")
        );
        assert_eq!(
            normalize(asset.bytes().to_vec()).unwrap(),
            asset.bytes(),
            "Normalization is idempotent"
        );
        let svg = first_frame_svg(&raw).unwrap();
        assert_eq!(svg, first_frame_svg(asset.bytes()).unwrap());
        assert!(std::str::from_utf8(&svg).unwrap().contains("<path"));
    }
    #[test]
    fn ordinary_tgs_keeps_original_bytes_and_numeric_x_coordinates() {
        let mut source = fixture();
        source["layers"][0]["shapes"][0]["ks"]
            .as_object_mut()
            .unwrap()
            .remove("x");
        source["layers"][0]["ks"]["p"] = json!({"s":true,"x":{"a":0,"k":10},"y":{"a":0,"k":20}});
        let bytes = compressed(&source);
        assert_eq!(normalize(bytes.clone()).unwrap(), bytes);
    }
    #[test]
    fn normalization_rechecks_the_expanded_size_after_serializing_numbers() {
        let mut json = serde_json::to_string(&fixture()).unwrap();
        json.pop();
        json.push_str(",\"padding\":[");
        json.push_str(&vec!["1e9"; 200_000].join(","));
        json.push_str("]}");
        assert!(json.len() < MAX_TGS_JSON_BYTES);
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(json.as_bytes()).unwrap();
        assert_eq!(
            normalize(gzip.finish().unwrap()).unwrap_err(),
            MediaError::InvalidMedia
        );
    }
    #[test]
    fn expression_only_and_external_data_stay_rejected_even_with_baked_properties() {
        for property in [
            json!({"x":"expression"}),
            json!({"a":1,"k":[],"x":"expression"}),
            json!({"a":1,"k":[{"t":0}],"x":"expression"}),
            json!({"a":0,"k":"expression","x":"expression"}),
            json!({"a":1,"k":null,"x":"expression"}),
            json!({"a":9,"k":[1],"x":"expression"}),
            json!({"a":0,"k":[1],"x":"expression","p":"https://example.test/image"}),
        ] {
            let mut source = fixture();
            source["layers"][0]["shapes"][0]["ks"] = property;
            assert_eq!(
                normalize(compressed(&source)).unwrap_err(),
                MediaError::InvalidMedia
            );
        }
        let mut source = fixture();
        source["assets"] = json!([{"id":"image","u":"https://example.test/","p":"x.png"}]);
        assert_eq!(
            normalize(compressed(&source)).unwrap_err(),
            MediaError::InvalidMedia
        );
    }

    #[test]
    fn failure_logs_include_validation_stage_and_reason_without_payloads() {
        // Isolate tracing's process-wide callsite/level cache from parallel tests
        // that deliberately reject media without installing a subscriber.
        if std::env::var_os("MARKET_MEDIA_LOG_TEST_CHILD").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tgs::tests::failure_logs_include_validation_stage_and_reason_without_payloads",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("MARKET_MEDIA_LOG_TEST_CHILD", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        #[derive(Clone, Default)]
        struct Log(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for Log {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let log = Log::default();
        let output = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(move || output.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "media",
                document_id = 5413686816387672057_i64,
                variant = "Animation"
            );
            let _entered = span.enter();
            let mut source = fixture();
            source["layers"][0]["shapes"][0]["ks"] = json!({"x":"SECRET_EXPRESSION"});
            assert!(normalize(compressed(&source)).is_err());
        });
        let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        for expected in [
            "5413686816387672057",
            "Animation",
            "tgs_validation",
            "expression_without_keyframes",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        assert!(!text.contains("SECRET_EXPRESSION"));
    }
}
