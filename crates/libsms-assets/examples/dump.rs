//! Prints a model's joints, shapes and materials (TEV programs), or a texture pattern.
//!
//!     cargo run -p libsms-assets --example dump -- bmd/ma_mdl1.bmd

use libsms_assets::{Bmd, btp::TexPattern};

fn main() {
    let rel = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bmd/ma_mdl1.bmd".into());
    let dir = libsms::assets::find(None).expect("no assets/ folder");
    let data = std::fs::read(dir.join(&rel)).expect("read");
    if rel.ends_with(".btp") {
        let p = TexPattern::parse(&data).expect("parse");
        println!("loop {} length {}", p.loop_mode, p.length);
        for t in &p.tracks {
            println!("  {} map {} {:?}", t.material, t.tex_map, t.frames);
        }
        return;
    }
    let m = Bmd::parse(&data).expect("parse");
    println!(
        "{} joints, {} draw mtx, {} envelopes, {} shapes, {} materials, {} textures",
        m.joints.len(),
        m.draw_mtx.len(),
        m.envelopes.len(),
        m.shapes.len(),
        m.materials.len(),
        m.textures.len()
    );
    for (i, j) in m.joints.iter().enumerate() {
        println!(
            "  joint {i} {} parent {:?} t {:?}",
            j.name, j.parent, j.translation
        );
    }
    for t in &m.textures {
        println!(
            "  tex {} fmt {} {}x{} wrap {:?}/{:?}",
            t.name, t.format, t.image.width, t.image.height, t.wrap_s, t.wrap_t
        );
    }
    let batches = m.batches().expect("batches");
    for b in &batches {
        let mat = &m.materials[b.material];
        println!(
            "  draw shape {} ({} tris, mtx type {}) mat {} {} mode {} blend {:?} cull {:?} tex {:?}",
            b.shape,
            b.indices.len() / 3,
            m.shapes[b.shape].mtx_type,
            b.material,
            mat.name,
            mat.mode,
            mat.blend(),
            mat.cull,
            mat.textures
        );
    }
    for (i, mat) in m.materials.iter().enumerate() {
        println!(
            "-- material {i} {} chans {} {:?} mat {:?} amb {:?}",
            mat.name, mat.chan_num, mat.chans[0], mat.mat_colors, mat.amb_colors
        );
        println!("   z test {} write {}", mat.z_test, mat.z_write);
        println!("   chans {:?}", mat.chans);
        println!("   gens {:?}", mat.tex_gens);
        for (k, t) in mat.tex_mtx.iter().enumerate() {
            if let Some(t) = t {
                println!(
                    "   texmtx {k} proj {} mode {} srt {:?}",
                    t.projection,
                    t.info,
                    t.srt_rows()
                );
            }
        }
        println!(
            "   regs {:?} konst {:?} alpha {:?} blend {:?} swap {:?}",
            mat.tev_colors, mat.konst, mat.alpha_compare, mat.blend_mode, mat.swap_tables
        );
        for s in &mat.stages {
            println!(
                "   stage coord {} map {} chan {} k {}/{} swap {}/{} C {:?} A {:?}",
                s.tex_coord,
                s.tex_map,
                s.channel,
                s.kcolor_sel,
                s.kalpha_sel,
                s.ras_swap,
                s.tex_swap,
                s.color,
                s.alpha
            );
        }
    }
}
