//! A disc-derived Delfino Plaza still, including the stage's separately placed props.
//! TActor::load / TPlacement::load define scene.bin transforms; MapObjInit.cpp
//! maps Palm -> palmNormal/palmLeaf and Manhole -> manhole.bmd.
use crate::tev::{self, Inputs, Tex};
use glam::{Mat4, Vec2, Vec3};
use libsms_assets::{Bmd, Error, Image, Pose};
use std::path::Path;

pub struct Prop {
    pub resource: String,
    pub transform: Mat4,
}
/// Find bounded, typed JDrama records; never interpret a text occurrence as an actor.
pub fn props(data: &[u8]) -> Vec<Prop> {
    let mut out = Vec::new();
    let u16at = |p: usize| {
        data.get(p..p + 2)
            .map(|v| u16::from_be_bytes(v.try_into().unwrap()) as usize)
    };
    let u32at = |p: usize| {
        data.get(p..p + 4)
            .map(|v| u32::from_be_bytes(v.try_into().unwrap()) as usize)
    };
    for i in 0..data.len().saturating_sub(60) {
        let Some(size) = u32at(i) else { continue };
        if !(60..=220).contains(&size) || i + size > data.len() {
            continue;
        }
        let Some(n) = u16at(i + 6) else { continue };
        let Some(kind) = data.get(i + 8..i + 8 + n) else {
            continue;
        };
        let key = kind
            .iter()
            .fold(0u16, |h, &v| h.wrapping_mul(3).wrapping_add(v as u16));
        if u16at(i + 4) != Some(key as usize) {
            continue;
        }
        if kind != b"Palm"
            && kind != b"Manhole"
            && kind != b"MonumentShine"
            && kind != b"MapObjBase"
        {
            continue;
        }
        let at = i + 8 + n;
        let Some(name_len) = u16at(at + 2) else {
            continue;
        };
        let pos = at + 4 + name_len;
        if pos + 36 > i + size {
            continue;
        }
        let tail = &data[pos + 36..i + size];
        let tail_lower = tail.to_ascii_lowercase();
        let resource = [
            "palmnormal",
            "palmleaf",
            "manhole",
            "monumentshine",
            "dokangate",
        ]
        .into_iter()
        .find(|r| tail_lower.ends_with(r.as_bytes()));
        let Some(resource) = resource else { continue };
        let values: Vec<f32> = (0..9)
            .map(|j| f32::from_be_bytes(data[pos + j * 4..pos + j * 4 + 4].try_into().unwrap()))
            .collect();
        if !values.iter().all(|v| v.is_finite()) {
            continue;
        }
        let v = |j| Vec3::new(values[j], values[j + 1], values[j + 2]);
        let transform =
            libsms_assets::pose::srt_matrix(v(6), v(3) * std::f32::consts::PI / 180., v(0));
        out.push(Prop {
            resource: resource.to_lowercase(),
            transform,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plaza_scene_restores_props_and_isolates_statue_with_its_pedestal() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        let Ok(data) = std::fs::read(dir.join("portal/scene.bin")) else {
            eprintln!("SKIP: Plaza assets absent");
            return;
        };
        let props = props(&data);
        for (resource, count) in [
            ("palmnormal", 18),
            ("palmleaf", 5),
            ("manhole", 13),
            ("monumentshine", 1),
            ("dokangate", 2),
        ] {
            assert_eq!(
                props.iter().filter(|p| p.resource == resource).count(),
                count,
                "{resource}"
            );
        }
        let model = Bmd::parse(&std::fs::read(dir.join("portal/plaza.bmd")).unwrap()).unwrap();
        let statue = statue_batches(&model).unwrap();
        assert!(
            statue
                .iter()
                .any(|b| model.materials[b.material].name == "_king_storn_1")
        );
        let min_y = statue
            .iter()
            .flat_map(|b| &b.vertices)
            .map(|v| v.position.y)
            .fold(f32::INFINITY, f32::min);
        assert!(
            (min_y - 300.).abs() < 0.1,
            "pedestal's foot must be present"
        );
        for b in &statue {
            assert!(b.indices.iter().all(|&i| (i as usize) < b.vertices.len()));
        }
    }
}

pub fn plaza_still(dir: &Path, size: usize) -> Result<Image, Error> {
    let model = Bmd::parse(&std::fs::read(dir.join("portal/plaza.bmd"))?)?;
    let placements = props(&std::fs::read(dir.join("portal/scene.bin"))?);
    let resources = [
        "palmnormal",
        "palmleaf",
        "manhole",
        "monumentshine",
        "efdokangate",
    ]
    .map(|name| Bmd::parse(&std::fs::read(dir.join(format!("portal/{name}.bmd")))?));
    let mut models = resources.into_iter().collect::<Result<Vec<_>, Error>>()?;
    // TMonumentShine::initMapObj/hitByWater: K0 alpha is the remaining dirt.
    for m in &mut models[3].materials {
        m.konst[0][3] = 0;
    }
    let mut items = vec![(&model, Mat4::IDENTITY)];
    for p in &placements {
        let index = match p.resource.as_str() {
            "palmnormal" => 0,
            "palmleaf" => 1,
            "manhole" => 2,
            "monumentshine" => 3,
            _ => 4,
        };
        items.push((&models[index], p.transform));
    }
    Ok(render(
        &items,
        Vec3::new(0., 1800., 10000.),
        Vec3::new(0., 900., -1000.),
        size,
        size,
        |_| true,
    ))
}

pub fn render(
    items: &[(&Bmd, Mat4)],
    eye: Vec3,
    target: Vec3,
    w: usize,
    h: usize,
    shown: impl Fn(usize) -> bool,
) -> Image {
    render_filtered(items, eye, target, w, h, shown, false)
}

/// The statue and its pedestal are baked into the map. Select their bounded
/// triangles, preserving their original materials, UVs and vertex colours.
pub fn statue_batches(model: &Bmd) -> Result<Vec<libsms_assets::Batch>, Error> {
    let lo = Vec3::new(-650., 299.9, 3900.);
    let hi = Vec3::new(750., 2000., 5100.);
    let mut selected = Vec::new();
    for mut b in model.batches()? {
        let indices: Vec<_> = b
            .indices
            .chunks_exact(3)
            .filter(|tri| {
                tri.iter().all(|&i| {
                    let p = b.vertices[i as usize].position;
                    p.cmpge(lo).all() && p.cmple(hi).all()
                })
            })
            .flatten()
            .copied()
            .collect();
        if indices.is_empty() {
            continue;
        }
        let mut remap = std::collections::HashMap::new();
        let mut vertices = Vec::new();
        b.indices = indices
            .into_iter()
            .map(|old| {
                *remap.entry(old).or_insert_with(|| {
                    let id = vertices.len() as u32;
                    vertices.push(b.vertices[old as usize]);
                    id
                })
            })
            .collect();
        b.vertices = vertices;
        selected.push(b);
    }
    Ok(selected)
}

pub fn render_statue(model: &Bmd, eye: Vec3, target: Vec3, w: usize, h: usize) -> Image {
    render_filtered(
        &[(model, Mat4::IDENTITY)],
        eye,
        target,
        w,
        h,
        |_| true,
        true,
    )
}

fn render_filtered(
    items: &[(&Bmd, Mat4)],
    eye: Vec3,
    target: Vec3,
    w: usize,
    h: usize,
    shown: impl Fn(usize) -> bool,
    statue_only: bool,
) -> Image {
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let vp = Mat4::perspective_rh_gl(50f32.to_radians(), w as f32 / h as f32, 10., 70000.) * view;
    let mut pixels = vec![[135u8, 202, 237, 255]; w * h];
    let mut depth = vec![f32::INFINITY; w * h];
    for &(model, root) in items {
        let pose = Pose::new(model, root, &model.bind_world());
        let textures: Vec<_> = model
            .textures
            .iter()
            .map(|t| Some(Tex::new(t.image.clone(), t.wrap_s, t.wrap_t)))
            .collect();
        let channels: Vec<_> = model
            .materials
            .iter()
            .map(|m| {
                std::array::from_fn(|i| {
                    let ch = m.chans[i * 2];
                    let alpha = m.chans[i * 2 + 1];
                    let mut c = if ch.mat_from_vertex {
                        [1.; 4]
                    } else {
                        m.mat_colors[i].map(|v| v as f32 / 255.)
                    };
                    // The specular channel starts at zero, not full white. Its full-white
                    // stand-in saturated the pipe's TEV output and erased its red paint.
                    if ch.lighting && ch.attn_fn == 0 {
                        c[..3].fill(0.);
                    }
                    c[3] = if alpha.mat_from_vertex {
                        1.
                    } else {
                        m.mat_colors[i][3] as f32 / 255.
                    };
                    c
                })
            })
            .collect();
        let source = tev::Source {
            bmd: model,
            textures: &textures,
            channels: &channels,
        };
        let bakes: Vec<_> = (0..model.materials.len())
            .map(|m| source.bake(m, &Inputs::default(), 512))
            .collect();
        let batches = if statue_only {
            statue_batches(model).unwrap()
        } else {
            model.batches().unwrap()
        };
        for b in batches.iter().filter(|b| shown(b.shape)) {
            let mat = &model.materials[b.material];
            let Some(tex) = &bakes[b.material] else {
                continue;
            };
            for tri in b.indices.chunks_exact(3) {
                let verts = [tri[0], tri[1], tri[2]].map(|i| b.vertices[i as usize]);
                let p = verts.map(|v| vp * pose.skin(&v).0.extend(1.));
                if p.iter().any(|v| v.w <= 0.) {
                    continue;
                }
                let xy = p.map(|v| {
                    Vec2::new(
                        (v.x / v.w + 1.) * 0.5 * w as f32,
                        (1. - v.y / v.w) * 0.5 * h as f32,
                    )
                });
                let edge = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
                let area = edge(xy[0], xy[1], xy[2]);
                if area.abs() < 0.001 {
                    continue;
                }
                let min = xy
                    .into_iter()
                    .fold(Vec2::splat(f32::INFINITY), Vec2::min)
                    .max(Vec2::ZERO);
                let max = xy
                    .into_iter()
                    .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max)
                    .min(Vec2::new(w as f32 - 1., h as f32 - 1.));
                for y in min.y as usize..=max.y.max(0.) as usize {
                    for x in min.x as usize..=max.x.max(0.) as usize {
                        if x >= w || y >= h {
                            continue;
                        }
                        let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let bary = [
                            edge(xy[1], xy[2], q) / area,
                            edge(xy[2], xy[0], q) / area,
                            edge(xy[0], xy[1], q) / area,
                        ];
                        if bary.iter().any(|&v| v < 0.) {
                            continue;
                        }
                        let z = (0..3).map(|i| bary[i] * p[i].z / p[i].w).sum::<f32>();
                        let at = y * w + x;
                        if z >= depth[at] {
                            continue;
                        }
                        let weights = bary.map(|v| v);
                        let denom = (0..3).map(|i| weights[i] / p[i].w).sum::<f32>();
                        let uv = if let Some(matrix) = tex.env {
                            let normal = (0..3)
                                .map(|i| {
                                    view.transform_vector3(pose.skin(&verts[i]).1)
                                        * (weights[i] / p[i].w / denom)
                                })
                                .sum::<Vec3>()
                                .normalize_or_zero();
                            Vec2::from_array(tev::env_uv(normal.x, normal.y, matrix))
                        } else {
                            (0..3)
                                .map(|i| verts[i].uv[0] * (weights[i] / p[i].w / denom))
                                .sum::<Vec2>()
                        };
                        let fix = |v: f32, n: u32, wrap: u8| -> usize {
                            let t = (v * n as f32).floor() as i32;
                            if wrap == 1 {
                                t.clamp(0, n as i32 - 1) as usize
                            } else {
                                t.rem_euclid(n as i32) as usize
                            }
                        };
                        let t = (fix(uv.y, tex.height, tex.wrap[1]) * tex.width as usize
                            + fix(uv.x, tex.width, tex.wrap[0]))
                            * 4;
                        let c = &tex.rgba[t..t + 4];
                        if c[3] < 128 {
                            continue;
                        }
                        let vc = (0..3)
                            .map(|i| {
                                Vec3::new(
                                    verts[i].colors[0][0] as f32,
                                    verts[i].colors[0][1] as f32,
                                    verts[i].colors[0][2] as f32,
                                ) * (weights[i] / p[i].w / denom / 255.)
                            })
                            .sum::<Vec3>();
                        for k in 0..3 {
                            pixels[at][k] = (c[k] as f32
                                * if mat.chans[0].mat_from_vertex {
                                    vc[k]
                                } else {
                                    1.
                                }) as u8;
                        }
                        depth[at] = z;
                    }
                }
            }
        }
    }
    Image {
        width: w as u32,
        height: h as u32,
        rgba: pixels.into_iter().flatten().collect(),
    }
}
