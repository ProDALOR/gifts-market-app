use super::*;
use serde_json::{Value, json};
fn fixture() -> Value {
    json!({"v":"5.7.4","w":512,"h":512,"fr":60,"ip":0,"op":60,"assets":[],"layers":[{"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{},"shapes":[{"ty":"rc","p":{"a":0,"k":[256,256]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0}},{"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1}]}]})
}
#[test]
fn uniform_even_odd_compound_paths_keep_transparent_holes() {
    let mut source = fixture();
    let mut inner = source["layers"][0]["shapes"][0].clone();
    inner["s"]["k"] = json!([50, 50]);
    source["layers"][0]["shapes"]
        .as_array_mut()
        .unwrap()
        .insert(1, inner);
    for (rule, expected_alpha) in [(1, 255), (2, 0)] {
        source["layers"][0]["shapes"][2]["r"] = json!(rule);
        let svg = first_frame_svg(&serde_json::to_vec(&source).unwrap()).unwrap();
        let tree = resvg::usvg::Tree::from_data(&svg, &Default::default()).unwrap();
        let mut pixels = resvg::tiny_skia::Pixmap::new(512, 512).unwrap();
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::identity(),
            &mut pixels.as_mut(),
        );
        assert_eq!(
            pixels.pixel(256, 256).unwrap().alpha(),
            expected_alpha,
            "fill rule {rule}"
        );
        assert_eq!(pixels.pixel(215, 256).unwrap().alpha(), 255);
    }
    source["layers"]
        .as_array_mut()
        .unwrap()
        .push(fixture()["layers"][0].clone());
    assert!(
        !uniform_even_odd(&source),
        "mixed paint rules cannot become a document-wide rule"
    );
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
fn descending_gradient_stops_match_svg_clamping_without_reordering_colors() {
    let mut source = fixture();
    source["layers"][0]["shapes"][1] = json!({"ty":"gf","o":{"a":0,"k":100},"t":1,
        "s":{"a":0,"k":[0,0]},"e":{"a":0,"k":[100,0]},
        "g":{"p":4,"k":{"a":0,"k":[0,1,0,0,0.75,0,1,0,0.25,0,0,1,1,1,1,1]}}});
    let descending = first_frame_svg(&serde_json::to_vec(&source).unwrap()).unwrap();
    source["layers"][0]["shapes"][1]["g"]["k"]["k"][8] = json!(0.75);
    assert_eq!(
        descending,
        first_frame_svg(&serde_json::to_vec(&source).unwrap()).unwrap()
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

fn grouped_fixture() -> Value {
    let mut source = fixture();
    let mut shapes = source["layers"][0]["shapes"].as_array().unwrap().clone();
    shapes.push(json!({"ty":"tr","a":{"a":0,"k":[13,17]},"p":{"a":0,"k":[42,73]},"s":{"a":0,"k":[120,80]},"r":{"a":0,"k":25},"o":{"a":0,"k":100}}));
    source["layers"][0]["shapes"] = json!([{"ty":"gr","it":shapes}]);
    source
}
#[test]
fn split_shape_positions_match_vector_positions_with_anchor_scale_and_rotation() {
    let expected = grouped_fixture();
    for position in [
        json!({"s":true,"x":{"a":0,"k":42},"y":{"a":0,"k":73}}),
        json!({"s":1,"x":{"a":1,"k":[{"t":0,"s":[42]}]},"y":{"a":1,"k":[{"t":-10,"s":[73]},{"t":0}]}}),
    ] {
        let mut split = expected.clone();
        split["layers"][0]["shapes"][0]["it"][2]["p"] = position;
        assert_eq!(
            first_frame_svg(&serde_json::to_vec(&split).unwrap()).unwrap(),
            first_frame_svg(&serde_json::to_vec(&expected).unwrap()).unwrap()
        );
    }
}
#[test]
fn split_shape_axes_sample_each_shared_precomposition_instance_at_its_local_frame() {
    let mut source = grouped_fixture();
    let child = &mut source["layers"][0];
    child["shapes"][0]["it"][2] = json!({"ty":"tr","p":{"s":true,
        "x":{"a":1,"k":[{"t":0,"s":[10],"h":1},{"t":5,"s":[20],"h":1},{"t":15,"s":[40]}]},
        "y":{"a":1,"k":[{"t":-10,"s":[30],"h":1},{"t":10,"s":[50]}]}}});
    source["assets"] = json!([{"id":"shared","layers":[child.clone()]}]);
    source["layers"] = json!([
        {"ty":0,"w":512,"h":512,"ind":1,"refId":"shared","ip":0,"op":60,"st":0,"sr":1,"ks":{}},
        {"ty":0,"w":512,"h":512,"ind":2,"refId":"shared","ip":0,"op":60,"st":-10,"sr":1,"ks":{}}
    ]);
    let composition = adapter::composition(source.clone()).unwrap();
    let mut positions = Vec::new();
    for layer in &composition.layers {
        let velato::model::Content::Instance { name, .. } = &layer.content else {
            panic!()
        };
        let velato::model::Content::Shape(shapes) = &composition.assets[name][0].content else {
            panic!()
        };
        let velato::model::Shape::Group(_, Some(transform)) = &shapes[0] else {
            panic!()
        };
        positions.push(transform.transform.evaluate(0.0).into_owned() * kurbo::Point::ZERO);
    }
    assert_eq!(
        positions,
        [kurbo::Point::new(10.0, 30.0), kurbo::Point::new(20.0, 50.0)]
    );
    assert!(first_frame_svg(&serde_json::to_vec(&source).unwrap()).is_ok());
}
#[test]
fn empty_and_nested_groups_keep_patches_aligned() {
    let mut source = grouped_fixture();
    let shapes = source["layers"][0]["shapes"].as_array_mut().unwrap();
    shapes.insert(
        0,
        json!({"ty":"gr","it":[{"ty":"tr","p":{"s":true,"x":{"a":0,"k":1},"y":{"a":0,"k":2}}}]}),
    );
    let nested = shapes.pop().unwrap();
    shapes.push(json!({"ty":"gr","it":[nested,{"ty":"tr","p":{"s":true,"x":{"a":0,"k":3},"y":{"a":0,"k":4}}}]}));
    assert!(first_frame_svg(&serde_json::to_vec(&source).unwrap()).is_ok());
}

#[test]
fn omitted_empty_shape_layers_remain_transform_parents() {
    let mut explicit = fixture();
    explicit["layers"][0]["parent"] = json!(2);
    explicit["layers"].as_array_mut().unwrap().push(
        json!({"ty":4,"ind":2,"ip":0,"op":60,"st":0,"ks":{"p":{"a":0,"k":[50,70]}},"shapes":[]}),
    );
    let mut omitted = explicit.clone();
    omitted["layers"][1]
        .as_object_mut()
        .unwrap()
        .remove("shapes");
    let expected = first_frame_svg(&serde_json::to_vec(&explicit).unwrap()).unwrap();
    let actual = first_frame_svg(&serde_json::to_vec(&omitted).unwrap()).unwrap();
    assert_eq!(actual, expected);
    assert_ne!(
        actual,
        first_frame_svg(&serde_json::to_vec(&fixture()).unwrap()).unwrap()
    );
}
#[test]
fn malformed_split_axes_are_rejected_instead_of_silently_defaulting_to_zero() {
    for x in [
        json!(null),
        json!({"a":0,"k":[1,2]}),
        json!({"a":0,"k":"bad"}),
    ] {
        let mut source = grouped_fixture();
        source["layers"][0]["shapes"][0]["it"][2]["p"] = json!({"s":true,"x":x,"y":{"a":0,"k":5}});
        assert_eq!(
            first_frame_svg(&serde_json::to_vec(&source).unwrap()),
            Err(crate::MediaError::InvalidMedia)
        );
    }
}
