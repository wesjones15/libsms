//! Every non-voice sound the Mario and FLUDD animations key (`bas/*.bas`), by name, with how many
//! clips use it: the list of sounds a replacement ("no-asset") sound pack has to cover besides
//! the voice cues. `cargo run -p libsms-assets --example sound_inventory`.

use std::collections::BTreeMap;

use libsms_assets::sound::{SoundData, bas::AnimSoundTable};

fn main() {
    let dir = libsms::assets::find(None).expect("no assets/");
    let sd = SoundData::load(&dir).expect("sound data");
    let mut used: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in std::fs::read_dir(dir.join("bas")).expect("bas/").flatten() {
        let p = e.path();
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        if p.extension().is_none_or(|x| x != "bas") {
            continue;
        }
        let table = AnimSoundTable::parse(&std::fs::read(&p).unwrap()).unwrap();
        for s in table.sounds {
            let name = sd.names.name(s.id).map(str::to_string).unwrap_or_else(|| format!("id_{:#x}", s.id));
            if name.starts_with("MSD_SE_MV") || name.starts_with("MSD_SE_MA_VO") {
                continue;
            }
            let clips = used.entry(name).or_default();
            if !clips.contains(&stem) {
                clips.push(stem.clone());
            }
        }
    }
    println!("{} distinct non-voice sounds keyed by animations", used.len());
    for (name, clips) in &used {
        println!("{:<40} {:>3} clips  e.g. {}", name.trim_start_matches("MSD_SE_"), clips.len(), clips.iter().take(3).cloned().collect::<Vec<_>>().join(" "));
    }
}
