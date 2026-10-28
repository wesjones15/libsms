//! `cargo run -p libsms-assets --example pianta_bas -- <assets>`: the sounds each Pianta clip triggers (`npc/montemcommon/bas`).
use libsms_assets::sound::{AnimSoundTable, SoundData};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("assets folder"));
    let data = SoundData::load(&dir).expect("sound data");
    let mut files: Vec<_> = std::fs::read_dir(dir.join("npc/montemcommon/bas")).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    for f in files {
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        if name.ends_with("_c") { continue }
        let table = AnimSoundTable::parse(&std::fs::read(&f).unwrap()).unwrap();
        println!("{name}:");
        for s in &table.sounds {
            println!("   {:>6.1}-{:<6.1} {:#x} {}", s.start_frame, s.end_frame, s.id, data.names.name(s.id).unwrap_or("?"));
        }
    }
}
