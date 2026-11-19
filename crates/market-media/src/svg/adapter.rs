//! Compatibility at frame zero using Velato's public model. Geometry stays with Velato.
use super::gradient::gradient_stops;
use crate::MediaError;
use serde_json::{Value as Json, json};
use velato::{
    Composition,
    model::{self, Animated, Content, EasingHandle, Shape, Time, Tween, Value},
};

pub(super) fn composition(mut source: Json) -> Result<Composition, MediaError> {
    super::budget::validate_source(&source)?;
    defaults(&mut source);
    // Retain only paint patches before consuming JSON in Velato; geometry/keyframes aren't cloned.
    let layers = layer_styles(&source["layers"])?;
    let source_assets: std::collections::HashMap<_, _> = source
        .get("assets")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter(|asset| asset.get("layers").is_some())
        .map(|asset| {
            Ok((
                asset["id"]
                    .as_str()
                    .ok_or(MediaError::InvalidMedia)?
                    .to_owned(),
                layer_styles(&asset["layers"])?,
            ))
        })
        .collect::<Result<_, MediaError>>()?;
    let mut model = Composition::from_json(source).map_err(|_| MediaError::InvalidMedia)?;
    super::budget::validate(&model)?;
    let originals = std::mem::take(&mut model.assets);
    let mut context = Instances {
        originals: &originals,
        source: &source_assets,
        repaired: std::collections::HashMap::new(),
        result: std::collections::HashMap::new(),
        stack: Vec::new(),
        next: 0,
    };
    repair_layers(&layers, &mut model.layers, 0.0, &mut context)?;
    model.assets = context.result;
    Ok(model)
}
struct Instances<'a> {
    originals: &'a std::collections::HashMap<String, Vec<model::Layer>>,
    source: &'a std::collections::HashMap<String, Vec<Vec<Json>>>,
    repaired: std::collections::HashMap<(String, u64), String>,
    result: std::collections::HashMap<String, Vec<model::Layer>>,
    stack: Vec<String>,
    next: usize,
}
fn repair_layers(
    source: &[Vec<Json>],
    layers: &mut [model::Layer],
    frame: f64,
    context: &mut Instances,
) -> Result<(), MediaError> {
    if source.len() != layers.len() {
        return Err(MediaError::InvalidMedia);
    }
    for (source, layer) in source.iter().zip(layers) {
        if let Content::Instance { name, .. } = &mut layer.content {
            if context.stack.contains(name) || context.stack.len() >= 32 || context.next >= 1024 {
                return Err(MediaError::InvalidMedia);
            }
            let local = (frame - layer.start_frame) / layer.stretch;
            if !local.is_finite() {
                return Err(MediaError::InvalidMedia);
            }
            let identity = (name.clone(), if local == 0.0 { 0 } else { local.to_bits() });
            if let Some(repaired) = context.repaired.get(&identity) {
                *name = repaired.clone();
                continue;
            }
            let mut instance = context
                .originals
                .get(name)
                .ok_or(MediaError::InvalidMedia)?
                .clone();
            let raw = context.source.get(name).ok_or(MediaError::InvalidMedia)?;
            context.stack.push(name.clone());
            context.next += 1;
            let instance_name = format!("preview.{}", context.next);
            repair_layers(raw, &mut instance, local, context)?;
            context.stack.pop();
            context.result.insert(instance_name.clone(), instance);
            context.repaired.insert(identity, instance_name.clone());
            *name = instance_name;
        }
        if let Content::Shape(shapes) = &mut layer.content {
            let mut styles = source.iter();
            repair_shapes(shapes, &mut styles, frame)?;
            if styles.next().is_some() {
                return Err(MediaError::InvalidMedia);
            }
        }
    }
    Ok(())
}
fn layer_styles(layers: &Json) -> Result<Vec<Vec<Json>>, MediaError> {
    layers
        .as_array()
        .ok_or(MediaError::InvalidMedia)?
        .iter()
        .map(|layer| {
            let mut styles = Vec::new();
            if let Some(shapes) = layer.get("shapes").and_then(Json::as_array) {
                collect_styles(shapes, &mut styles);
            }
            Ok(styles)
        })
        .collect()
}
fn collect_styles(shapes: &[Json], out: &mut Vec<Json>) {
    for shape in shapes {
        match shape.get("ty").and_then(Json::as_str) {
            Some("gr") => {
                if let Some(items) = shape.get("it").and_then(Json::as_array) {
                    collect_styles(items, out);
                }
            }
            Some("gf" | "gs" | "fl" | "st") => out.push(shape.clone()),
            _ => {}
        }
    }
}
fn repair_shapes<'a>(
    shapes: &mut [Shape],
    styles: &mut impl Iterator<Item = &'a Json>,
    frame: f64,
) -> Result<(), MediaError> {
    for shape in shapes {
        match shape {
            Shape::Group(shapes, _) => repair_shapes(shapes, styles, frame)?,
            Shape::Draw(draw) => {
                let source = styles.next().ok_or(MediaError::InvalidMedia)?;
                let kind = source["ty"].as_str().ok_or(MediaError::InvalidMedia)?;
                if draw.stroke.is_some() != matches!(kind, "gs" | "st") {
                    return Err(MediaError::InvalidMedia);
                }
                if matches!(kind, "gf" | "gs") {
                    draw.opacity = Value::Fixed(scalar(source.get("o"), 100.0, frame)?);
                    let mut brush = draw.brush.evaluate(1.0, frame).into_owned();
                    let peniko::Brush::Gradient(gradient) = &mut brush else {
                        return Err(MediaError::InvalidMedia);
                    };
                    let count = source["g"]["p"].as_u64().ok_or(MediaError::InvalidMedia)? as usize;
                    let raw = channels(&source["g"]["k"], frame)?;
                    let stops = gradient_stops(&raw, count);
                    if stops.is_empty() {
                        return Err(MediaError::InvalidMedia);
                    }
                    gradient.stops = stops
                        .into_iter()
                        .map(|[offset, r, g, b, a]| {
                            (
                                offset as f32,
                                peniko::Color::new([r as f32, g as f32, b as f32, a as f32]),
                            )
                                .into()
                        })
                        .collect::<Vec<_>>()
                        .as_slice()
                        .into();
                    draw.brush = model::Brush::Fixed(brush);
                }
                if let Some(stroke) = &mut draw.stroke
                    && source.get("d").is_some()
                {
                    let mut fixed = stroke.evaluate(frame).into_owned();
                    fixed.dash_pattern.clear();
                    fixed.dash_offset = 0.0;
                    for dash in source
                        .get("d")
                        .and_then(Json::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let value = scalar(dash.get("v"), 0.0, frame)?;
                        match dash.get("n").and_then(Json::as_str) {
                            Some("o") => fixed.dash_offset = value,
                            Some("d" | "g") => fixed.dash_pattern.push(value),
                            _ => return Err(MediaError::InvalidMedia),
                        }
                    }
                    *stroke = model::Stroke::Fixed(fixed);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn defaults(source: &mut Json) {
    if let Json::Object(object) = source {
        // A duplicate endpoint prevents upstream's zero-duration terminal interpolation.
        if let Some(frames) = object
            .get_mut("k")
            .and_then(Json::as_array_mut)
            .filter(|v| v.first().is_some_and(|v| v.get("t").is_some()))
            && let Some(mut last) = frames.last().cloned()
        {
            if last.get("s").is_none()
                && let Some(value) = frames
                    .iter()
                    .rev()
                    .skip(1)
                    .find_map(|v| v.get("e").or_else(|| v.get("s")))
            {
                last["s"] = value.clone();
            }
            if let Some(time) = last["t"].as_f64() {
                last["t"] = json!(time + 1.0);
                frames.push(last);
            }
        }
        let is_layer = object.get("ty").is_some_and(Json::is_number);
        let is_transform = object.get("ty").is_some_and(|ty| ty == "tr");
        let transform = if is_layer {
            object.get_mut("ks").and_then(Json::as_object_mut)
        } else if is_transform {
            Some(&mut *object)
        } else {
            None
        };
        if let Some(transform) = transform {
            transform.entry("r").or_insert_with(|| json!({"a":0,"k":0}));
            transform
                .entry("p")
                .or_insert_with(|| json!({"a":0,"k":[0,0,0]}));
        }
        for value in object.values_mut() {
            defaults(value);
        }
    } else if let Json::Array(values) = source {
        for value in values {
            defaults(value);
        }
    }
}
/// Delegate temporal easing to the published evaluator; clamp singleton/end frames explicitly.
#[derive(Clone)]
struct Channels(Vec<f64>);
impl Tween for Channels {
    fn tween(&self, other: &Self, t: f64, easing: &model::Easing) -> Self {
        if !t.is_finite() || t <= 0.0 {
            return self.clone();
        }
        Self(
            self.0
                .iter()
                .zip(&other.0)
                .map(|(a, b)| a.tween(b, t, easing))
                .collect(),
        )
    }
}
fn numbers(value: &Json) -> Result<Vec<f64>, MediaError> {
    if let Some(value) = value.as_f64() {
        return Ok(vec![value]);
    }
    value
        .as_array()
        .ok_or(MediaError::InvalidMedia)?
        .iter()
        .map(|v| v.as_f64().ok_or(MediaError::InvalidMedia))
        .collect()
}
fn handle(value: &Json) -> Option<EasingHandle> {
    let component = |v: &Json| v.as_f64().or_else(|| v.as_array()?.first()?.as_f64());
    Some(EasingHandle {
        x: component(&value["x"])?,
        y: component(&value["y"])?,
    })
}
fn channels(property: &Json, frame: f64) -> Result<Vec<f64>, MediaError> {
    let k = &property["k"];
    let Some(frames) = k
        .as_array()
        .filter(|v| v.first().is_some_and(Json::is_object))
    else {
        return numbers(k);
    };
    let mut times = Vec::new();
    let mut values = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let value = frame
            .get("s")
            .or_else(|| {
                index
                    .checked_sub(1)
                    .and_then(|i| frames[i].get("e").or_else(|| frames[i].get("s")))
            })
            .ok_or(MediaError::InvalidMedia)?;
        let value = numbers(value)?;
        if values
            .first()
            .is_some_and(|v: &Channels| v.0.len() != value.len())
        {
            return Err(MediaError::InvalidMedia);
        }
        times.push(Time {
            frame: frame["t"].as_f64().ok_or(MediaError::InvalidMedia)?,
            in_tangent: handle(&frame["i"]),
            out_tangent: handle(&frame["o"]),
            hold: frame.get("h").is_some_and(|v| v == 1 || v == true),
        });
        values.push(Channels(value));
    }
    if values.is_empty() {
        return Err(MediaError::InvalidMedia);
    }
    let first = values[0].clone();
    Ok(Animated { times, values }.evaluate_or(frame, first).0)
}
fn scalar(property: Option<&Json>, default: f64, frame: f64) -> Result<f64, MediaError> {
    match property {
        Some(property) => channels(property, frame)?
            .first()
            .copied()
            .ok_or(MediaError::InvalidMedia),
        None => Ok(default),
    }
}
