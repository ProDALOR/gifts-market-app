//! Bound expansion before Velato allocates repeater batches or the adapter clones instances.
use crate::MediaError;
use serde_json::Value as Json;
use velato::{
    Composition,
    model::{Content, Geometry, Layer, Shape},
};

const MAX_WORK: usize = 200_000;
const MAX_SOURCE_NODES: usize = 500_000;
fn invalid() -> MediaError {
    MediaError::InvalidMedia
}

pub(super) fn validate_source(source: &Json) -> Result<(), MediaError> {
    let mut pending = vec![source];
    let mut count = 0;
    while let Some(value) = pending.pop() {
        count += 1;
        if count > MAX_SOURCE_NODES {
            return Err(invalid());
        }
        match value {
            Json::Object(object) => {
                // Import may eagerly evaluate fixed stars, so reject expansion before import.
                let property = match object.get("ty").and_then(Json::as_str) {
                    Some("sr") => object.get("pt"),
                    Some("rp") => object.get("c"),
                    _ => None,
                };
                if let Some(property) = property {
                    let mut values = vec![property];
                    while let Some(value) = values.pop() {
                        match value {
                            Json::Number(n) if n.as_f64().is_none_or(|n| n.abs() > 256.0) => {
                                return Err(invalid());
                            }
                            Json::Object(v) => values.extend(
                                v.iter()
                                    .filter(|(k, _)| matches!(k.as_str(), "k" | "s" | "e"))
                                    .map(|(_, v)| v),
                            ),
                            Json::Array(v) => values.extend(v),
                            _ => {}
                        }
                    }
                }
                pending.extend(object.values());
            }
            Json::Array(array) => pending.extend(array),
            Json::Number(n) if n.as_f64().is_none_or(|n| n.abs() > 1_000_000.0) => {
                return Err(invalid());
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn validate(model: &Composition) -> Result<(), MediaError> {
    let mut remaining = MAX_WORK;
    layers(model, &model.layers, 0.0, 0, &mut remaining)
}
fn spend(remaining: &mut usize, cost: usize) -> Result<(), MediaError> {
    *remaining = remaining.checked_sub(cost).ok_or_else(invalid)?;
    Ok(())
}
fn layers(
    model: &Composition,
    list: &[Layer],
    frame: f64,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), MediaError> {
    if depth > 32 {
        return Err(invalid());
    }
    for layer in list {
        spend(remaining, 1)?;
        // Parenting is followed in a loop upstream; a cycle would never reach the sink.
        let mut parent = layer.parent;
        for hop in 0..=32 {
            let Some(index) = parent else {
                break;
            };
            if hop == 32 {
                return Err(invalid());
            }
            parent = list.get(index).ok_or_else(invalid)?.parent;
        }
        match &layer.content {
            Content::Instance { name, .. } => {
                let frame = (frame - layer.start_frame) / layer.stretch;
                if !frame.is_finite() {
                    return Err(invalid());
                }
                layers(
                    model,
                    model.assets.get(name).ok_or_else(invalid)?,
                    frame,
                    depth + 1,
                    remaining,
                )?;
            }
            Content::Shape(list) => {
                spend(remaining, shapes(list, frame, depth)?.cost()?)?;
            }
            _ => {}
        }
    }
    Ok(())
}
#[derive(Default)]
struct Work {
    vertices: usize,
    draws: usize,
    painted: usize,
}
impl Work {
    fn cost(&self) -> Result<usize, MediaError> {
        self.vertices
            .checked_add(self.draws)
            .and_then(|v| v.checked_add(self.painted))
            .filter(|cost| *cost <= MAX_WORK)
            .ok_or_else(invalid)
    }
}
fn shapes(list: &[Shape], frame: f64, depth: usize) -> Result<Work, MediaError> {
    if depth > 32 {
        return Err(invalid());
    }
    let mut work = Work::default();
    for shape in list {
        match shape {
            Shape::Group(list, _) => {
                let child = shapes(list, frame, depth + 1)?;
                work.vertices = work
                    .vertices
                    .checked_add(child.vertices)
                    .ok_or_else(invalid)?;
                work.draws = work.draws.checked_add(child.draws).ok_or_else(invalid)?;
                work.painted = work
                    .painted
                    .checked_add(child.painted)
                    .ok_or_else(invalid)?;
            }
            Shape::Geometry(geometry) => {
                if let Geometry::Star(star) = geometry
                    && !(0.0..=256.0).contains(&star.points.evaluate(frame))
                {
                    return Err(invalid());
                }
                let mut path = Vec::new();
                geometry.evaluate(frame, &mut path);
                work.vertices = work.vertices.checked_add(path.len()).ok_or_else(invalid)?;
            }
            Shape::Draw(_) => {
                work.draws += 1;
                work.painted = work
                    .painted
                    .checked_add(work.vertices)
                    .ok_or_else(invalid)?;
            }
            Shape::Repeater(repeater) => {
                let copies = repeater.evaluate(frame).as_ref().copies;
                if copies > 256 {
                    return Err(invalid());
                }
                work.vertices = work.vertices.checked_mul(copies).ok_or_else(invalid)?;
                work.draws = work.draws.checked_mul(copies).ok_or_else(invalid)?;
                work.painted = work.painted.checked_mul(copies).ok_or_else(invalid)?;
            }
            Shape::Trim(_) => {
                work.vertices = work.vertices.checked_mul(3).ok_or_else(invalid)?;
                work.painted = work.painted.checked_mul(3).ok_or_else(invalid)?;
            }
        }
        work.cost()?;
    }
    Ok(work)
}
