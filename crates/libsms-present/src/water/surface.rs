//! CPU counterpart of ModelWaterManager.cpp::drawSilhouette: sum the exact watermask
//! coverage, then shade its outer surface once. Hosts can use their existing textured
//! transparency/distortion materials, without compositing each droplet separately.
use super::{WaterTexture, WaterVertex};
use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Default)]
struct Sample {
    coverage: f32,
    depth: f32,
    weight: f32,
}

pub(super) fn merge(
    quads: &[WaterVertex],
    mask: &WaterTexture,
    eye: Vec3,
    right: Vec3,
    up: Vec3,
) -> Vec<WaterVertex> {
    if quads.is_empty() || mask.rgba.is_empty() {
        return quads.to_vec();
    }
    // WaterLook uses the host camera's left-handed basis: right = up x forward.
    // Increasing depth must face the eye, even when most water is behind a close camera.
    let normal = -right.cross(up).normalize_or_zero();
    let xy = |p: Vec3| Vec2::new(p.dot(right), p.dot(up));
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    let mut smallest = f32::INFINITY;
    for q in quads.chunks_exact(4) {
        for v in q {
            lo = lo.min(xy(v.pos));
            hi = hi.max(xy(v.pos));
        }
        smallest = smallest.min(q[1].pos.distance(q[3].pos));
    }
    // Bounded work. Hosts use 32-bit mesh indices for the worst-case grid.
    // Anchor to world coordinates so the lattice does not swim as the stream grows.
    let span = hi - lo;
    let step = (smallest / 12.0)
        .max(span.x / 300.0)
        .max(span.y / 120.0)
        .max(2.0);
    lo = (lo / step).floor() * step - Vec2::splat(step);
    hi = (hi / step).ceil() * step + Vec2::splat(step);
    let nx = ((hi.x - lo.x) / step).round() as usize + 1;
    let ny = ((hi.y - lo.y) / step).round() as usize + 1;
    let mut field = vec![Sample::default(); nx * ny];
    for q in quads.chunks_exact(4) {
        let p = q.map_positions(&xy);
        let qlo = p
            .iter()
            .copied()
            .fold(Vec2::splat(f32::INFINITY), Vec2::min);
        let qhi = p
            .iter()
            .copied()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
        let x0 = (((qlo.x - lo.x) / step).floor() as usize).min(nx - 1);
        let x1 = (((qhi.x - lo.x) / step).ceil() as usize).min(nx - 1);
        let y0 = (((qlo.y - lo.y) / step).floor() as usize).min(ny - 1);
        let y1 = (((qhi.y - lo.y) / step).ceil() as usize).min(ny - 1);
        let depth = q.iter().map(|v| v.pos.dot(normal)).sum::<f32>() * 0.25;
        if depth >= eye.dot(normal) - 1.0 {
            continue;
        }
        let thickness = q[1].pos.distance(q[3].pos).max(step);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let at = lo + Vec2::new(x as f32, y as f32) * step;
                let uv = [[0, 1, 2], [0, 2, 3]].into_iter().find_map(|ids| {
                    let [a, b, c] = ids;
                    let ab = p[b] - p[a];
                    let ac = p[c] - p[a];
                    let d = at - p[a];
                    let det = ab.perp_dot(ac);
                    if det.abs() < 0.001 {
                        return None;
                    }
                    let u = d.perp_dot(ac) / det;
                    let v = ab.perp_dot(d) / det;
                    (u >= -0.0001 && v >= -0.0001 && u + v <= 1.0001).then(|| {
                        Vec2::from(q[a].uv) * (1.0 - u - v)
                            + Vec2::from(q[b].uv) * u
                            + Vec2::from(q[c].uv) * v
                    })
                });
                let Some(uv) = uv else {
                    continue;
                };
                let coverage = alpha(mask, uv);
                if coverage <= 0.001 {
                    continue;
                }
                let s = &mut field[y * nx + x];
                // Only nearby water merges. Two streams crossing at different depths keep
                // the front surface rather than manufacturing a sheet between them.
                if s.weight > 0.0 && (depth - s.depth / s.weight).abs() > thickness {
                    if depth < s.depth / s.weight {
                        continue;
                    }
                    *s = Sample::default();
                }
                s.coverage += coverage;
                s.depth += depth * coverage;
                s.weight += coverage;
            }
        }
    }
    let coverage = |x: usize, y: usize| field[y * nx + x].coverage;
    let mut out = Vec::new();
    for y in 1..ny - 2 {
        for x in 1..nx - 2 {
            let ids = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)];
            // Keep the whole covered footprint, not just its dense core: the per-corner alpha
            // then fades to zero on the actual coverage contour instead of ending on a
            // stair-stepped grid threshold.
            if ids.iter().all(|&(x, y)| coverage(x, y) <= 0.002) {
                continue;
            }
            let fallback = ids
                .iter()
                .map(|&(x, y)| field[y * nx + x])
                .max_by(|a, b| a.weight.total_cmp(&b.weight))
                .unwrap();
            for (x, y) in ids {
                let s = field[y * nx + x];
                let t = (s.coverage / 0.48).clamp(0.0, 1.0);
                let opacity = t * t * (3.0 - 2.0 * t);
                let gradient = Vec2::new(
                    coverage(x + 1, y) - coverage(x - 1, y),
                    coverage(x, y + 1) - coverage(x, y - 1),
                );
                // Sphere-map coordinates from the *combined* coverage slope. The interior
                // stays neutral instead of restarting a sphere at every segment.
                let slope = -gradient / (gradient.length_squared() + 0.35 * 0.35).sqrt();
                let uv = Vec2::splat(0.5) + slope * 0.38;
                let depth = if s.weight > 0.0 {
                    s.depth / s.weight
                } else {
                    fallback.depth / fallback.weight
                };
                let at = lo + Vec2::new(x as f32, y as f32) * step;
                out.push(WaterVertex {
                    pos: right * at.x + up * at.y + normal * depth,
                    uv: uv.to_array(),
                    alpha: opacity,
                });
            }
        }
    }
    out
}

trait Positions {
    fn map_positions(&self, project: &impl Fn(Vec3) -> Vec2) -> [Vec2; 4];
}
impl Positions for [WaterVertex] {
    fn map_positions(&self, project: &impl Fn(Vec3) -> Vec2) -> [Vec2; 4] {
        [
            project(self[0].pos),
            project(self[1].pos),
            project(self[2].pos),
            project(self[3].pos),
        ]
    }
}

fn alpha(mask: &WaterTexture, uv: Vec2) -> f32 {
    let p = uv.clamp(Vec2::ZERO, Vec2::ONE)
        * Vec2::new((mask.width - 1) as f32, (mask.height - 1) as f32);
    let x = p.x as usize;
    let y = p.y as usize;
    let x1 = (x + 1).min(mask.width as usize - 1);
    let y1 = (y + 1).min(mask.height as usize - 1);
    let at = |x, y| mask.rgba[(y * mask.width as usize + x) * 4 + 3] as f32 / 255.0;
    let a = at(x, y) * (1.0 - p.fract().x) + at(x1, y) * p.fract().x;
    let b = at(x, y1) * (1.0 - p.fract().x) + at(x1, y1) * p.fract().x;
    a * (1.0 - p.fract().y) + b * p.fract().y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask() -> WaterTexture {
        let mut rgba = Vec::new();
        for y in 0..32 {
            for x in 0..32 {
                let radius = Vec2::new(x as f32 / 15.5 - 1.0, y as f32 / 15.5 - 1.0).length();
                rgba.extend_from_slice(&[255, 255, 255, ((1.0 - radius).max(0.0) * 255.0) as u8]);
            }
        }
        WaterTexture {
            width: 32,
            height: 32,
            rgba,
        }
    }
    fn quad(x: f32, z: f32) -> Vec<WaterVertex> {
        [
            (-50., -50., [0., 0.]),
            (50., -50., [1., 0.]),
            (50., 50., [1., 1.]),
            (-50., 50., [0., 1.]),
        ]
        .map(|(dx, y, uv)| WaterVertex {
            pos: Vec3::new(x + dx, y, z),
            uv,
            alpha: 1.0,
        })
        .to_vec()
    }
    fn opacity_at(mesh: &[WaterVertex], at: Vec2) -> f32 {
        for q in mesh.chunks_exact(4) {
            let a = q[0].pos.truncate();
            let b = q[2].pos.truncate();
            if at.cmpge(a).all() && at.cmple(b).all() {
                let t = (at - a) / (b - a);
                return q[0].alpha * (1. - t.x) * (1. - t.y)
                    + q[1].alpha * t.x * (1. - t.y)
                    + q[2].alpha * t.x * t.y
                    + q[3].alpha * (1. - t.x) * t.y;
            }
        }
        0.0
    }
    #[test]
    fn close_coverage_becomes_one_surface_but_distant_drops_stay_separate() {
        let mut drops = quad(-25., 0.);
        drops.extend(quad(25., 0.));
        let mesh = merge(&drops, &mask(), -Vec3::Z * 1000.0, Vec3::X, Vec3::Y);
        assert!(
            opacity_at(&mesh, Vec2::ZERO) > 0.95,
            "gap between close drops"
        );
        assert!(
            mesh.iter()
                .all(|v| v.alpha.is_finite() && (0.0..=1.0).contains(&v.alpha))
        );
        let mut distant = quad(-100., 0.);
        distant.extend(quad(100., 0.));
        let mesh = merge(&distant, &mask(), -Vec3::Z * 1000.0, Vec3::X, Vec3::Y);
        assert_eq!(
            opacity_at(&mesh, Vec2::ZERO),
            0.0,
            "bridged separated drops"
        );
    }
    #[test]
    fn crossing_water_uses_front_depth_instead_of_a_sheet_between_depths() {
        for reverse in [false, true] {
            let mut drops = quad(0., if reverse { 500. } else { 0. });
            drops.extend(quad(0., if reverse { 0. } else { 500. }));
            let mesh = merge(&drops, &mask(), -Vec3::Z * 1000.0, Vec3::X, Vec3::Y);
            assert!(!mesh.is_empty());
            assert!(mesh.iter().all(|v| v.pos.z.abs() < 0.001));
        }
    }

    #[test]
    fn reversed_camera_basis_keeps_front_water_and_rejects_water_behind_eye() {
        let mut drops = quad(0.0, 0.0);
        drops.extend(quad(0.0, 500.0));
        drops.extend(quad(0.0, 1500.0));
        let mesh = merge(&drops, &mask(), Vec3::Z * 1000.0, -Vec3::X, Vec3::Y);
        assert!(!mesh.is_empty());
        assert!(mesh.iter().all(|v| (v.pos.z - 500.0).abs() < 0.001));
    }
}
