// Copyright 2024 the Velato Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// Preserve the breakpoints of independently interpolated color and opacity functions.
/// Repeated offsets retain their authored discontinuities.
pub(super) fn gradient_stops(raw: &[f64], count: usize) -> Vec<[f64; 5]> {
    if count > 1024 || raw.len() > 6144 || raw.iter().any(|value| !value.is_finite()) {
        return vec![];
    }
    let Some(color_len) = count.checked_mul(4).filter(|len| *len <= raw.len()) else {
        return vec![];
    };
    let mut colors: Vec<_> = raw[..color_len]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| (v[0], [v[1], v[2], v[3]]))
        .collect();
    if colors.is_empty() {
        return vec![];
    }
    let mut opacity: Vec<_> = raw[color_len..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| (v[0], [v[1]]))
        .collect();
    if !(raw.len() - color_len).is_multiple_of(2) {
        return vec![];
    }
    // Lottie emits authored stops in order. SVG clamps a descending offset to
    // its predecessor; sorting would change the colors and hard edges.
    clamp_offsets(&mut colors);
    clamp_offsets(&mut opacity);
    let mut offsets: Vec<_> = colors
        .iter()
        .map(|p| p.0)
        .chain(opacity.iter().map(|p| p.0))
        .collect();
    offsets.sort_by(f64::total_cmp);
    offsets.dedup();
    let mut result = Vec::new();
    for offset in offsets {
        let repeats = (colors.partition_point(|p| p.0 <= offset)
            - colors.partition_point(|p| p.0 < offset))
        .max(
            opacity.partition_point(|p| p.0 <= offset) - opacity.partition_point(|p| p.0 < offset),
        );
        for repeat in 0..repeats {
            let rgb = sample(&colors, offset, repeat);
            let alpha = if opacity.is_empty() {
                1.0
            } else {
                sample(&opacity, offset, repeat)[0]
            };
            result.push([offset, rgb[0], rgb[1], rgb[2], alpha]);
        }
    }
    result
}
fn clamp_offsets<const N: usize>(points: &mut [(f64, [f64; N])]) {
    let mut previous = 0.0_f64;
    for (offset, _) in points {
        *offset = offset.clamp(0.0, 1.0).max(previous);
        previous = *offset;
    }
}
fn sample<const N: usize>(points: &[(f64, [f64; N])], x: f64, repeat: usize) -> [f64; N] {
    let first = points.partition_point(|p| p.0 < x);
    let end = points.partition_point(|p| p.0 <= x);
    if first < end {
        return points[(first + repeat).min(end - 1)].1;
    }
    if first == 0 {
        return points[0].1;
    }
    if first == points.len() {
        return points[first - 1].1;
    }
    let (a, av) = points[first - 1];
    let (b, bv) = points[first];
    let t = (x - a) / (b - a);
    std::array::from_fn(|i| av[i] + (bv[i] - av[i]) * t)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descending_offsets_clamp_in_authored_order_instead_of_sorting() {
        let result = gradient_stops(
            &[
                0.0, 1.0, 0.0, 0.0, 0.8, 0.0, 1.0, 0.0, 0.2, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0,
                0.0, 1.0, 0.4, 0.5, 0.3, 0.0,
            ],
            4,
        );
        assert_eq!(result[1], [0.4, 0.5, 0.5, 0.0, 0.5]);
        assert_eq!(result[2], [0.4, 0.5, 0.5, 0.0, 0.0]);
        assert_eq!(result[3], [0.8, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(result[4], [0.8, 0.0, 0.0, 1.0, 0.0]);
    }
    #[test]
    fn independent_opacity_breakpoints_and_boundaries_are_retained() {
        let raw = [
            0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.2, 0.0, 0.5, 1.0, 0.8, 0.0,
        ];
        let result = gradient_stops(&raw, 2);
        assert_eq!(result.len(), 5);
        assert_eq!(result[0], [0.0, 1.0, 0.0, 0.0, 0.0]);
        assert_eq!(result[2], [0.5, 0.5, 0.0, 0.5, 1.0]);
        assert_eq!(result[4], [1.0, 0.0, 0.0, 1.0, 0.0]);
    }
    #[test]
    fn duplicate_offsets_preserve_hard_edges() {
        let result = gradient_stops(
            &[
                0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0, 0.0, 0.5, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0,
            ],
            4,
        );
        assert_eq!(result[1], [0.5, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(result[2], [0.5, 0.0, 0.0, 1.0, 1.0]);
    }
}
