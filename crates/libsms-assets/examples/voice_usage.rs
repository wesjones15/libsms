//! Which Mario voice cues the animations key: every `bas/ma_*.bas` entry whose sound is a
//! voice (`MSD_SE_MV*`, `MSD_SE_MA_VO*`), by name, with the clips that use it.
//! `cargo run -p libsms-assets --example voice_usage`.

use std::collections::BTreeMap;

use libsms_assets::sound::{SoundData, bas::AnimSoundTable};

fn main() {
    let dir = libsms::assets::find(None).expect("no assets/");
    let sd = SoundData::load(&dir).expect("sound data");
    let mut used: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut files = 0;
    for e in std::fs::read_dir(dir.join("bas")).expect("bas/").flatten() {
        let p = e.path();
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        if p.extension().is_none_or(|x| x != "bas") || !stem.starts_with("ma_") {
            continue;
        }
        files += 1;
        let table = AnimSoundTable::parse(&std::fs::read(&p).unwrap()).unwrap();
        for s in table.sounds {
            if let Some(name) = sd.names.name(s.id)
                && (name.starts_with("MSD_SE_MV") || name.starts_with("MSD_SE_MA_VO"))
            {
                let clips = used.entry(name.to_string()).or_default();
                if !clips.contains(&stem) {
                    clips.push(stem.clone());
                }
            }
        }
    }
    println!("{files} Mario .bas files; {} distinct voice cues keyed", used.len());
    for (name, clips) in &used {
        println!("{:<34} {} clips: {}", name.trim_start_matches("MSD_SE_"), clips.len(), clips.join(" "));
    }
}
