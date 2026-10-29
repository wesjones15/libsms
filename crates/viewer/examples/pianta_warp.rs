//! Headless Pianta asset proof. No window or graphics device is opened.
//! Source: NpcInitPrg.cpp::setMtxEffect_ (koshi_null, type 0),
//! MtxUtil.cpp::TMtxTimeLag::calc, NpcAnm.cpp::requestTalkAnm_.
use glam::{Mat4, Vec3};
use libsms_present::body::Pianta;
use libsms_present::mtx_effect::TimeLag;
fn main() {
    let dir = libsms::assets::find(None).expect("Sunshine assets");
    let p = Pianta::load(&dir, false, 1).unwrap();
    let bytes = std::fs::read(dir.join("params_npc/montem/mtxeffect1.prm")).unwrap();
    let prm = libsms::prm::Prm::parse(&bytes).unwrap();
    for key in [
        "mPosAccel",
        "mPosBrake",
        "mPosLimit",
        "mQuatAccel",
        "mQuatBrake",
    ] {
        println!("{key} = {:?}", prm.f32(key));
    }
    for (j, name) in p.rig.names.iter().enumerate() {
        println!("joint {j} {name}, parent {:?}", p.rig.parents[j]);
    }
    for name in ["wait", "talk"] {
        for frame in [0., 10., 20., 40.] {
            let pose = p.pose_own(name, frame).unwrap();
            for joint in ["yashi_jnt", "leafL_jnt", "leafR_jnt"] {
                let j = p.rig.find(joint).unwrap();
                let local =
                    p.rig.parents[j].map_or(pose[j], |parent| pose[parent].inverse() * pose[j]);
                println!(
                    "{name} {frame} {joint}: {:?}",
                    local.to_scale_rotation_translation()
                );
            }
        }
    }
    if let Some(out) = std::env::args().nth(1) {
        capture(&p, &out);
    }
}

/// Small orthographic software renderer. Rows: source clip, source waist lag,
/// source waist lag plus talk plant. Columns: walk frames 0, 10, 20, 30.
fn capture(p: &Pianta, out: &str) {
    let (w, h) = (1000usize, 900usize);
    let mut rgba = [28, 33, 45, 255].repeat(w * h);
    let mut depth = vec![f32::NEG_INFINITY; w * h];
    let mut state = TimeLag::default();
    for frame in 0..=30 {
        let t = frame as f32 / 30.0;
        let base = p.pose_own("walk", frame as f32).unwrap();
        let root = Mat4::from_rotation_translation(
            glam::Quat::from_rotation_y(t * 0.8),
            Vec3::Z * t * 150.0,
        );
        let mut lagged = base.clone();
        p.apply_waist_lag(&mut lagged, root, t, &mut state);
        if frame % 10 != 0 {
            continue;
        }
        let mut talking = lagged.clone();
        p.talk_plant(&mut talking, frame as f32, 1.0);
        for (row, joints) in [&base, &lagged, &talking].into_iter().enumerate() {
            let pose = libsms_assets::Pose::new(&p.model.bmd, Mat4::from_rotation_y(-0.4), joints);
            let col = frame / 10;
            for (bi, batch) in p.model.batches.iter().enumerate() {
                let vertices: Vec<_> = batch
                    .vertices
                    .iter()
                    .map(|v| {
                        let (at, normal) = pose.skin(v);
                        let light = 0.65
                            + 0.35
                                * normal
                                    .normalize_or_zero()
                                    .dot(Vec3::new(-0.3, 0.8, 0.6).normalize())
                                    .max(0.0);
                        (
                            Vec3::new(
                                125.0 + col as f32 * 250.0 + at.x * 1.08,
                                280.0 + row as f32 * 300.0 - at.y * 1.08,
                                at.z,
                            ),
                            v.uv[0],
                            light,
                        )
                    })
                    .collect();
                let id = p.model.batch_texture[bi];
                let tex = (id >= 0).then(|| &p.textures.list[id as usize]);
                for tri in batch.indices.chunks_exact(3) {
                    let [a, b, c] = [
                        vertices[tri[0] as usize],
                        vertices[tri[1] as usize],
                        vertices[tri[2] as usize],
                    ];
                    let edge = |v: Vec3, u: Vec3, x: f32, y: f32| {
                        (u.x - v.x) * (y - v.y) - (u.y - v.y) * (x - v.x)
                    };
                    let area = edge(a.0, b.0, c.0.x, c.0.y);
                    if area.abs() < 1e-5 {
                        continue;
                    }
                    let lo = a.0.min(b.0).min(c.0).max(Vec3::ZERO);
                    let hi = a.0.max(b.0).max(c.0).min(Vec3::new(
                        w as f32 - 1.,
                        h as f32 - 1.,
                        f32::INFINITY,
                    ));
                    for y in lo.y as usize..=hi.y.max(0.) as usize {
                        for x in lo.x as usize..=hi.x.max(0.) as usize {
                            let weights = [
                                edge(b.0, c.0, x as f32 + 0.5, y as f32 + 0.5) / area,
                                edge(c.0, a.0, x as f32 + 0.5, y as f32 + 0.5) / area,
                            ];
                            let [u, v] = weights;
                            let q = 1. - u - v;
                            if u < 0. || v < 0. || q < 0. {
                                continue;
                            }
                            let index = y * w + x;
                            let z = u * a.0.z + v * b.0.z + q * c.0.z;
                            if z < depth[index] {
                                continue;
                            }
                            let uv = a.1 * u + b.1 * v + c.1 * q;
                            let color = tex.map_or([200, 200, 200, 255], |tex| {
                                let coord = |f: f32, size: u32, wrap: u8| {
                                    let f = match wrap {
                                        1 => f.clamp(0., 1.),
                                        2 => 1. - (f.rem_euclid(2.) - 1.).abs(),
                                        _ => f.rem_euclid(1.),
                                    };
                                    ((f * size as f32) as usize).min(size as usize - 1)
                                };
                                let tx = coord(uv.x, tex.width, tex.wrap[0]);
                                let ty = coord(uv.y, tex.height, tex.wrap[1]);
                                tex.rgba[(ty * tex.width as usize + tx) * 4..][..4]
                                    .try_into()
                                    .unwrap()
                            });
                            if color[3] < 128 {
                                continue;
                            }
                            let light = a.2 * u + b.2 * v + c.2 * q;
                            for k in 0..3 {
                                rgba[index * 4 + k] = (color[k] as f32 * light) as u8;
                            }
                            depth[index] = z;
                        }
                    }
                }
            }
        }
    }
    // Image::export_png expects the OpenGL bottom-up row convention.
    let rgba = rgba.chunks_exact(w * 4).rev().flatten().copied().collect();
    macroquad::prelude::Image {
        bytes: rgba,
        width: w as u16,
        height: h as u16,
    }
    .export_png(out);
    std::fs::write(format!("{out}.txt"), "Pianta: npc/montem/mom_model.bmd, koshi_null=1, mino_null=2.\nColumns: walk frames 0,10,20,30. Rows: rigid/source clip; TMtxTimeLag; TMtxTimeLag + mom_talk plant.\nSource: NpcInitPrg.cpp::setMtxEffect_, MtxUtil.cpp::TMtxTimeLag::calc, NpcAnm.cpp::requestTalkAnm_, NpcInitAnmData.cpp::sMonteM_AnmIndex[6].\nParameters: params_npc/montem/mtxeffect1.prm. No procedural panel guessing or hand-authored plant pose.\n").unwrap();
    println!("Headless capture: {out}");
}
