use super::*;
use serde_json::{Value, json};
fn fixture() -> Value {
    json!({"v":"5.7.4","w":512,"h":512,"fr":60,"ip":0,"op":60,"assets":[],"layers":[{"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{},"shapes":[{"ty":"rc","p":{"a":0,"k":[256,256]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0}},{"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1}]}]})
}
#[test]
fn omitted_transforms_match_authored_identity_without_normalizing_the_input() {
    let omitted = fixture();
    let mut explicit = omitted.clone();
    explicit["layers"][0]["ks"] = json!({"r":{"a":0,"k":0},"p":{"a":0,"k":[0,0,0]}});
    assert_eq!(
        first_frame_svg(&serde_json::to_vec(&omitted).unwrap()).unwrap(),
        first_frame_svg(&serde_json::to_vec(&explicit).unwrap()).unwrap()
    );
}
#[test]
fn renderer_rejects_external_assets_expressions_and_oversized_json() {
    for (key, value) in [
        (
            "assets",
            json!([{"id":"image","p":"http://external.test/file.png"}]),
        ),
        ("fonts", json!({})),
        ("x", json!("expression")),
    ] {
        let mut data = fixture();
        data[key] = value;
        assert_eq!(
            first_frame_svg(&serde_json::to_vec(&data).unwrap()).unwrap_err(),
            crate::MediaError::InvalidMedia
        );
    }
    assert_eq!(
        first_frame_svg(&vec![b' '; crate::MAX_TGS_JSON_BYTES + 1]).unwrap_err(),
        crate::MediaError::InvalidMedia
    );
}

#[test]
fn adapter_retains_style_opacity_and_animated_stroke_dashes() {
    let mut input = fixture();
    input["layers"][0]["shapes"][1] = json!({"ty":"gs","o":{"a":0,"k":25},"w":{"a":0,"k":4},"t":1,"s":{"a":0,"k":[0,0]},"e":{"a":0,"k":[100,0]},"g":{"p":2,"k":{"a":0,"k":[0,1,0,0,1,0,0,1]}},"d":[{"n":"d","v":{"a":1,"k":[{"t":0,"s":[3]},{"t":10,"s":[7]}]}},{"n":"g","v":{"a":0,"k":2}},{"n":"o","v":{"a":0,"k":1}}]});
    let composition = adapter::composition(input).unwrap();
    let velato::model::Content::Shape(shapes) = &composition.layers[0].content else {
        panic!("shape layer")
    };
    let velato::model::Shape::Draw(draw) = &shapes[1] else {
        panic!("gradient stroke")
    };
    assert_eq!(draw.opacity.evaluate(0.0), 25.0);
    let stroke = draw.stroke.as_ref().unwrap().evaluate(0.0).into_owned();
    assert_eq!(stroke.dash_pattern.as_slice(), &[3.0, 2.0]);
    assert_eq!(stroke.dash_offset, 1.0);
}
#[test]
fn singleton_and_terminal_keyframes_match_static_properties() {
    for times in [vec![0.0], vec![-10.0, 0.0]] {
        let mut animated = fixture();
        let mut explicit = fixture();
        animated["layers"][0]["ks"]["r"] =
            json!({"a":1,"k":times.iter().map(|t|json!({"t":t,"s":[30]})).collect::<Vec<_>>()});
        explicit["layers"][0]["ks"]["r"] = json!({"a":0,"k":30});
        assert_eq!(
            first_frame_svg(&serde_json::to_vec(&animated).unwrap()).unwrap(),
            first_frame_svg(&serde_json::to_vec(&explicit).unwrap()).unwrap()
        );
    }
}

#[test]
fn reused_precomposition_samples_each_instances_local_frame() {
    let mut source = fixture();
    let mut child = source["layers"][0].clone();
    child["shapes"][1] = json!({"ty":"gf","o":{"a":0,"k":100},"t":1,"s":{"a":0,"k":[0,0]},"e":{"a":0,"k":[100,0]},"g":{"p":2,"k":{"a":0,"k":[0,1,0,0,1,0,0,1]}}});
    child["shapes"][1]["o"] = json!({"a":1,"k":[{"t":0,"s":[10]},{"t":10,"s":[80]}]});
    source["assets"] = json!([{"id":"shared","layers":[child]}]);
    source["layers"] = json!([
        {"ty":0,"w":512,"h":512,"ind":1,"refId":"shared","ip":0,"op":60,"st":0,"sr":1,"ks":{}},
        {"ty":0,"w":512,"h":512,"ind":2,"refId":"shared","ip":0,"op":60,"st":-10,"sr":1,"ks":{}}
    ]);
    let repeated = source["layers"][0].clone();
    source["layers"].as_array_mut().unwrap().push(repeated);
    let composition = adapter::composition(source).unwrap();
    let values: Vec<_> = composition
        .layers
        .iter()
        .map(|layer| {
            let velato::model::Content::Instance { name, .. } = &layer.content else {
                panic!("precomposition")
            };
            let velato::model::Content::Shape(shapes) = &composition.assets[name][0].content else {
                panic!("shape")
            };
            let velato::model::Shape::Draw(draw) = &shapes[1] else {
                panic!("fill")
            };
            draw.opacity.evaluate(0.0)
        })
        .collect();
    assert_eq!(values, vec![10.0, 80.0, 10.0]);
    assert_eq!(
        composition.assets.len(),
        2,
        "equal local frames share one repaired asset"
    );
}

#[test]
fn compressed_sources_match_json_and_expansion_is_rejected_before_rendering() {
    use std::io::Write;
    let source = serde_json::to_vec(&fixture()).unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&source).unwrap();
    assert_eq!(
        first_frame_svg(&gzip.finish().unwrap()).unwrap(),
        first_frame_svg(&source).unwrap()
    );
    for kind in ["rp", "sr"] {
        let mut source = fixture();
        source["layers"][0]["shapes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"ty":kind,"c":{"a":0,"k":1_000_000},"pt":{"a":0,"k":1_000_000}}));
        assert_eq!(
            first_frame_svg(&serde_json::to_vec(&source).unwrap()).unwrap_err(),
            crate::MediaError::InvalidMedia
        );
    }
}
#[test]
fn nested_repeater_work_and_parent_cycles_are_bounded() {
    let repeater = json!({"ty":"rp","c":{"a":0,"k":100},"o":{"a":0,"k":0},"tr":{"a":{"a":0,"k":[0,0]},"p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"so":{"a":0,"k":100},"eo":{"a":0,"k":100}}});
    let mut source = fixture();
    source["layers"][0]["shapes"]
        .as_array_mut()
        .unwrap()
        .extend([repeater.clone(), repeater]);
    assert_eq!(
        first_frame_svg(&serde_json::to_vec(&source).unwrap()).unwrap_err(),
        crate::MediaError::InvalidMedia
    );
    let mut source = fixture();
    source["layers"][0]["parent"] = json!(1);
    assert_eq!(
        first_frame_svg(&serde_json::to_vec(&source).unwrap()).unwrap_err(),
        crate::MediaError::InvalidMedia
    );
}
#[test]
fn svg_output_stops_growing_when_its_budget_is_exhausted() {
    let mut sink = SvgSink::default();
    let brush = Brush::Solid(peniko::color::palette::css::WHITE);
    for _ in 0..20_000 {
        sink.draw(
            None,
            Affine::IDENTITY,
            &brush,
            &kurbo::Rect::new(0.0, 0.0, 10.0, 10.0),
        );
    }
    assert!(sink.over_budget);
    assert!(sink.body.len() + sink.defs.len() <= crate::MAX_PREVIEW_BYTES);
    assert!(sink.finish().is_err());
}
