//! `cargo run -p libsms-assets --example p2_aaf -- <AudioRes folder>`: parses Pikmin 2's `PSound.aaf` with the Sunshine
//! parser and prints what it found (the two games share JAudio's init data layout).
use libsms_assets::sound::InitData;

fn main() {
    let dir = std::env::args().nth(1).expect("AudioRes folder");
    let data = std::fs::read(std::path::Path::new(&dir).join("PSound.aaf")).expect("PSound.aaf");
    match InitData::parse(&data) {
        Ok(init) => {
            println!("banks {} wave banks {} scenes {} fx {} streams {}", init.banks.len(), init.wave_banks.len(),
                init.scenes.len(), init.fx_scenes.len(), init.streams.len());
            println!("formats {:?}", init.sound_table.formats);
            for (c, v) in init.sound_table.categories.iter().enumerate() {
                if !v.is_empty() { println!("category {c}: {} sounds", v.len()); }
            }
            for (i, w) in init.wave_banks.iter().enumerate() {
                println!("wave bank {i}: timing {} groups {} waves {}", w.load_timing, w.bank.groups.len(),
                    w.bank.groups.iter().map(|g| g.waves.len()).sum::<usize>());
                for g in w.bank.groups.iter().take(3) { println!("   group file {} waves {}", g.file, g.waves.len()); }
            }
            println!("seq header {} bytes", init.seq_header.len());
        }
        Err(e) => println!("parse failed: {e}"),
    }
}
