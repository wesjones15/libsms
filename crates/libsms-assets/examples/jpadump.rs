//! Prints a particle effect's shape and colour settings.
//!
//!     cargo run -p libsms-assets --example jpadump -- ms_dmg_c

use libsms_assets::jpa::Jpa;

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "ms_dmg_c".into());
    let dir = libsms::assets::find(None).expect("no assets/ folder");
    let data = std::fs::read(dir.join("particle").join(format!("{name}.jpa"))).expect("read");
    let j = Jpa::parse(&data).expect("parse");
    let b = &j.base;
    println!("kind {} color_mode {} blend {}/{}/{} tev {:?}", b.kind, b.color_mode, b.blend_mode, b.blend_src, b.blend_dst, j.base.tev());
    println!("prm_on {} prm_anm {} prm {:?}", b.prm_on, b.prm_anm, b.prm);
    println!("env_on {} env_anm {} env {:?}", b.env_on, b.env_anm, b.env);
    println!("col_loop {} global_col_anm {} col_anm_max {}", b.col_loop, b.global_col_anm, b.col_anm_max);
    println!("prm_table {:?}", b.prm_table);
    println!("env_table {:?}", b.env_table);
    println!("textures {}", j.textures.len());
    for t in &j.textures {
        println!("  {}x{} fmt {}", t.image.width, t.image.height, t.format);
        let w = t.image.width as usize;
        for y in (0..t.image.height as usize).step_by(4) {
            let mut l = String::new();
            let mut a = String::new();
            for x in (0..w).step_by(2) {
                let p = &t.image.rgba[(y * w + x) * 4..][..4];
                l.push(b" .:-=+*#%@"[(p[0] as usize * 9 / 255).min(9)] as char);
                a.push(b" .:-=+*#%@"[(p[3] as usize * 9 / 255).min(9)] as char);
            }
            println!("  {l} | {a}");
        }
    }
}
