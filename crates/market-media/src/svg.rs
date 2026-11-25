//! First-frame SVG export. Runs in the supervised Rust helper to isolate expensive parsing/rendering.
use kurbo::{Affine, Cap, Join, Shape, Stroke};
use peniko::{BlendMode, Brush, Compose, Extend, GradientKind, Mix, color::Srgb};
use std::fmt::Write;
use velato::{RenderSink, Renderer, model::ImageAsset};

#[derive(Default)]
struct SvgSink {
    defs: String,
    body: String,
    next: usize,
    layers: Vec<Layer>,
    warnings: Vec<String>,
    over_budget: bool,
    even_odd: bool,
}
struct Layer {
    backdrop: String,
    clip: usize,
    alpha: f32,
    blend: BlendMode,
    mode: &'static str,
}
fn matrix(transform: Affine) -> String {
    let [a, b, c, d, e, f] = transform.as_coeffs();
    format!("matrix({a} {b} {c} {d} {e} {f})")
}
fn path(shape: &impl Shape) -> String {
    shape.to_path(0.01).to_svg()
}
impl SvgSink {
    fn check_budget(&mut self) {
        let stored = self.defs.len()
            + self.body.len()
            + self
                .layers
                .iter()
                .map(|layer| layer.backdrop.len())
                .sum::<usize>();
        if stored > crate::MAX_PREVIEW_BYTES || self.next > 10_000 || self.layers.len() > 64 {
            self.over_budget = true;
            self.defs.clear();
            self.body.clear();
            self.layers.clear();
        }
    }
    fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }
    fn clip(&mut self, transform: Affine, shape: &impl Shape) -> usize {
        let id = self.id();
        write!(self.defs, "<clipPath id=\"c{id}\" clipPathUnits=\"userSpaceOnUse\"><path d=\"{}\" transform=\"{}\"/></clipPath>", path(shape), matrix(transform)).unwrap();
        id
    }
    fn paint(&mut self, brush: &Brush, attribute: &str, bounds: kurbo::Rect) -> String {
        match brush {
            Brush::Solid(color) => {
                let rgb = color.to_rgba8();
                format!(
                    "{attribute}=\"#{:02x}{:02x}{:02x}\" {attribute}-opacity=\"{}\"",
                    rgb.r, rgb.g, rgb.b, color.components[3]
                )
            }
            Brush::Gradient(gradient) => {
                let id = self.id();
                let spread = match gradient.extend {
                    Extend::Pad => "pad",
                    Extend::Repeat => "repeat",
                    Extend::Reflect => "reflect",
                };
                let header_start = self.defs.len();
                let close = match gradient.kind {
                    GradientKind::Linear(p) => {
                        write!(self.defs, "<linearGradient id=\"g{id}\" gradientUnits=\"userSpaceOnUse\" spreadMethod=\"{spread}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">", p.start.x,p.start.y,p.end.x,p.end.y).unwrap();
                        "linearGradient"
                    }
                    GradientKind::Radial(p) => {
                        write!(self.defs, "<radialGradient id=\"g{id}\" gradientUnits=\"userSpaceOnUse\" spreadMethod=\"{spread}\" cx=\"{}\" cy=\"{}\" r=\"{}\" fx=\"{}\" fy=\"{}\" fr=\"{}\">", p.end_center.x,p.end_center.y,p.end_radius,p.start_center.x,p.start_center.y,p.start_radius).unwrap();
                        "radialGradient"
                    }
                    GradientKind::Sweep(_) => {
                        self.warnings
                            .push("SVG sink does not implement sweep gradients".into());
                        return format!("{attribute}=\"none\"");
                    }
                };
                if gradient.interpolation_cs != peniko::color::ColorSpaceTag::Srgb {
                    self.warnings.push(format!(
                        "SVG sink does not implement gradient color space {:?}",
                        gradient.interpolation_cs
                    ));
                }
                let alpha_header = self.defs[header_start..]
                    .replace(&format!("id=\"g{id}\""), &format!("id=\"a{id}\""));
                for stop in gradient.stops.iter() {
                    let color = stop.color.to_alpha_color::<Srgb>();
                    let rgb = color.to_rgba8();
                    write!(
                        self.defs,
                        "<stop offset=\"{}\" stop-color=\"#{:02x}{:02x}{:02x}\"/>",
                        stop.offset, rgb.r, rgb.g, rgb.b
                    )
                    .unwrap();
                }
                write!(self.defs, "</{close}>").unwrap();
                let first_alpha = gradient.stops.first().map_or(1.0, |stop| {
                    stop.color.to_alpha_color::<Srgb>().components[3]
                });
                if gradient
                    .stops
                    .iter()
                    .all(|stop| stop.color.to_alpha_color::<Srgb>().components[3] == first_alpha)
                {
                    return format!(
                        "{attribute}=\"url(#g{id})\" {attribute}-opacity=\"{first_alpha}\""
                    );
                }
                self.defs.push_str(&alpha_header);
                for stop in gradient.stops.iter() {
                    write!(
                        self.defs,
                        r##"<stop offset="{}" stop-color="#ffffff" stop-opacity="{}"/>"##,
                        stop.offset,
                        stop.color.to_alpha_color::<Srgb>().components[3]
                    )
                    .unwrap();
                }
                write!(self.defs,r#"</{close}><mask id="a-mask{id}" maskUnits="userSpaceOnUse" x="{}" y="{}" width="{}" height="{}" mask-type="alpha"><rect x="{}" y="{}" width="{}" height="{}" fill="url(#a{id})"/></mask>"#,bounds.x0,bounds.y0,bounds.width(),bounds.height(),bounds.x0,bounds.y0,bounds.width(),bounds.height()).unwrap();
                format!("{attribute}=\"url(#g{id})\" mask=\"url(#a-mask{id})\"")
            }
            Brush::Image(_) => {
                self.warnings
                    .push("SVG sink does not implement raster brushes".into());
                format!("{attribute}=\"none\"")
            }
        }
    }
    fn finish(self) -> Result<String, crate::MediaError> {
        if self.over_budget || !self.layers.is_empty() {
            return Err(crate::MediaError::InvalidMedia);
        }
        Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"512\" height=\"512\" viewBox=\"0 0 512 512\"{}><defs>{}</defs>{}</svg>",
            if self.even_odd {
                " fill-rule=\"evenodd\""
            } else {
                ""
            },
            self.defs,
            self.body
        ))
    }
}
impl RenderSink for SvgSink {
    fn push_layer(
        &mut self,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        shape: &impl Shape,
    ) {
        if self.over_budget {
            return;
        }
        let blend = blend.into();
        if !matches!(
            blend.compose,
            Compose::SrcOver | Compose::SrcIn | Compose::SrcOut
        ) {
            self.warnings.push(format!(
                "SVG sink does not implement Porter-Duff {:?}",
                blend.compose
            ));
        }
        let mode = match blend.mix {
            Mix::Normal => "normal",
            Mix::Multiply => "multiply",
            Mix::Screen => "screen",
            Mix::Overlay => "overlay",
            Mix::Darken => "darken",
            Mix::Lighten => "lighten",
            Mix::ColorDodge => "color-dodge",
            Mix::ColorBurn => "color-burn",
            Mix::HardLight => "hard-light",
            Mix::SoftLight => "soft-light",
            Mix::Difference => "difference",
            Mix::Exclusion => "exclusion",
            Mix::Hue => "hue",
            Mix::Saturation => "saturation",
            Mix::Color => "color",
            Mix::Luminosity => "luminosity",
        };
        let id = self.clip(transform, shape);
        self.layers.push(Layer {
            backdrop: std::mem::take(&mut self.body),
            clip: id,
            alpha,
            blend,
            mode,
        });
        self.check_budget();
    }
    fn push_clip_layer(&mut self, transform: Affine, shape: &impl Shape) {
        if self.over_budget {
            return;
        }
        let id = self.clip(transform, shape);
        self.layers.push(Layer {
            backdrop: std::mem::take(&mut self.body),
            clip: id,
            alpha: 1.0,
            blend: BlendMode::default(),
            mode: "normal",
        });
        self.check_budget();
    }
    fn pop_layer(&mut self) {
        if self.over_budget {
            return;
        }
        let layer = self.layers.pop().expect("balanced layers");
        let source = format!(
            "<g clip-path=\"url(#c{})\" opacity=\"{}\" style=\"mix-blend-mode:{}\">{}</g>",
            layer.clip,
            layer.alpha,
            layer.mode,
            std::mem::take(&mut self.body)
        );
        if matches!(layer.blend.compose, Compose::SrcIn | Compose::SrcOut) {
            let id = self.id();
            if layer.blend.compose == Compose::SrcOut {
                write!(self.defs,r##"<filter id="black{id}"><feColorMatrix type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0"/></filter><mask id="m{id}" maskUnits="userSpaceOnUse" x="0" y="0" width="512" height="512" mask-type="luminance"><rect width="512" height="512" fill="#ffffff"/><g filter="url(#black{id})">{}</g></mask>"##,layer.backdrop).unwrap();
            } else {
                write!(self.defs,r#"<mask id="m{id}" maskUnits="userSpaceOnUse" x="0" y="0" width="512" height="512" mask-type="alpha">{}</mask>"#,layer.backdrop).unwrap();
            }
            write!(self.body, "<g mask=\"url(#m{id})\">{source}</g>").unwrap();
        } else {
            self.body = layer.backdrop;
            self.body.push_str(&source);
        }
        self.check_budget();
    }
    fn draw(
        &mut self,
        stroke: Option<&Stroke>,
        transform: Affine,
        brush: &Brush,
        shape: &impl Shape,
    ) {
        if self.over_budget {
            return;
        }
        let attribute = if stroke.is_some() { "stroke" } else { "fill" };
        let padding = stroke.map_or(1.0, |stroke| {
            stroke.width * stroke.miter_limit.max(1.0) + 1.0
        });
        let paint = self.paint(
            brush,
            attribute,
            shape.bounding_box().inflate(padding, padding),
        );
        write!(
            self.body,
            "<path d=\"{}\" transform=\"{}\" {paint}",
            path(shape),
            matrix(transform)
        )
        .unwrap();
        if let Some(stroke) = stroke {
            if stroke.start_cap != stroke.end_cap {
                self.warnings
                    .push("SVG sink requires equal stroke caps".into());
            }
            let cap = match stroke.start_cap {
                Cap::Butt => "butt",
                Cap::Round => "round",
                Cap::Square => "square",
            };
            let join = match stroke.join {
                Join::Miter => "miter",
                Join::Round => "round",
                Join::Bevel => "bevel",
            };
            write!(self.body," fill=\"none\" stroke-width=\"{}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\" stroke-miterlimit=\"{}\" stroke-dashoffset=\"{}\"",stroke.width,stroke.miter_limit,stroke.dash_offset).unwrap();
            if !stroke.dash_pattern.is_empty() {
                let values: Vec<_> = stroke
                    .dash_pattern
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                write!(self.body, " stroke-dasharray=\"{}\"", values.join(" ")).unwrap();
            }
        }
        self.body.push_str("/>");
        self.check_budget();
    }
    fn draw_image(&mut self, _: &ImageAsset, _: Affine, _: f64) {
        self.warnings
            .push("SVG sink does not implement image layers".into());
    }
}

/// Export compressed TGS or its JSON first frame as a self-contained, bounded SVG.
pub fn first_frame_svg(json: &[u8]) -> Result<Vec<u8>, crate::MediaError> {
    let invalid = crate::MediaError::InvalidMedia;
    let decoded;
    let json = if json.starts_with(&[0x1f, 0x8b]) {
        decoded = crate::tgs::decode(json)?;
        decoded.as_slice()
    } else {
        json
    };
    let json = crate::tgs::parse_json(json)?;
    // Velato's public sink omits fill rules. Preserve a uniform authored even-odd
    // rule at the SVG root, including compound paths such as the Telegram info icon.
    let even_odd = uniform_even_odd(&json);
    let animation = adapter::composition(json)?;
    let mut sink = SvgSink {
        even_odd,
        ..Default::default()
    };
    Renderer::new().append(
        &animation,
        0.0,
        Affine::scale_non_uniform(
            512.0 / animation.width as f64,
            512.0 / animation.height as f64,
        ),
        1.0,
        &mut sink,
    );
    if !sink.warnings.is_empty() {
        tracing::warn!(stage="svg_render", reason="unsupported_feature", feature=?sink.warnings.first(), "SVG export rejected");
        return Err(invalid);
    }
    let bytes = sink
        .finish()
        .inspect_err(|_| {
            tracing::warn!(
                stage = "svg_output",
                reason = "output_budget_or_unbalanced_layers",
                "SVG export rejected"
            )
        })?
        .into_bytes();
    crate::preview::validate(&bytes)
        .inspect_err(|_| tracing::warn!(stage = "svg_validation", "SVG export rejected"))?;
    Ok(bytes)
}

fn uniform_even_odd(source: &serde_json::Value) -> bool {
    fn visit(value: &serde_json::Value, fills: &mut usize, even_odd: &mut bool) {
        match value {
            serde_json::Value::Object(object) => {
                if matches!(
                    object.get("ty").and_then(serde_json::Value::as_str),
                    Some("fl" | "gf")
                ) {
                    *fills += 1;
                    *even_odd &= object.get("r").and_then(serde_json::Value::as_u64) == Some(2);
                }
                for child in object.values() {
                    visit(child, fills, even_odd);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    visit(item, fills, even_odd);
                }
            }
            _ => {}
        }
    }
    let mut fills = 0;
    let mut even_odd = true;
    visit(source, &mut fills, &mut even_odd);
    fills > 0 && even_odd
}

mod adapter;
mod budget;
mod gradient;
#[cfg(test)]
mod tests;
