//! Loads the user's extracted sound data. Skips when `assets/` isn't there.

use libsms_assets::sound::{SoundData, aaf::id, bank::Program};

fn load() -> Option<SoundData> {
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("skipped: no assets/");
        return None;
    };
    Some(SoundData::load(&dir).expect("sound data"))
}

#[test]
fn every_instrument_wave_resolves() {
    let Some(sd) = load() else { return };
    assert_eq!(sd.init.banks.len(), 13);
    let mut regions = 0;
    let mut missing = Vec::new();
    for b in &sd.init.banks {
        let waves = &sd.init.wave_banks[b.wave_bank].bank;
        for p in b.bank.programs.iter().flatten() {
            let vels: Vec<_> = match p {
                Program::Inst(i) => i.keys.iter().flat_map(|k| k.vels.iter()).collect(),
                Program::Perc(p) => p
                    .keys
                    .iter()
                    .flatten()
                    .flat_map(|k| k.vels.iter())
                    .collect(),
            };
            for v in vels {
                regions += 1;
                if waves.find(v.wave_id, None).is_none() {
                    missing.push((b.bank.virtual_id, v.wave_id));
                }
            }
        }
    }
    assert!(regions > 1000, "{regions}");
    // Two of bank 1's regions name waves the disc doesn't have; the game's `noteOn` gets no
    // wave handle for them and plays nothing.
    assert!(
        missing.len() <= 2,
        "{} of {regions}: {missing:?}",
        missing.len()
    );
}

#[test]
fn names_and_sound_table_agree() {
    let Some(sd) = load() else { return };
    assert_eq!(sd.names.id("MSD_SE_SEQ"), Some(0x8000_0800));
    let dolpic = sd.names.id("MSD_BGM_DOLPIC").unwrap();
    assert_eq!(dolpic & id::TYPE_MASK, id::TYPE_SEQUENCE);
    assert_eq!(
        sd.seqs.name((dolpic & id::INDEX_MASK) as usize),
        Some("k_dolpic.com")
    );
    let jump = sd
        .names
        .id("MSD_SE_MA_JUMP")
        .or(sd.names.id("MSD_SE_MA_ROLL"))
        .unwrap();
    assert_eq!(id::category(jump), 1);
    assert!(sd.init.sound_table.info(jump).is_some());
    // Every named SE has a table entry.
    for (name, sid) in sd.names.iter() {
        if sid & id::TYPE_MASK == id::TYPE_SE {
            assert!(sd.init.sound_table.info(sid).is_some(), "{name} {sid:#x}");
        }
    }
}

#[test]
fn title_stream_decodes() {
    let Some(sd) = load() else { return };
    let s = sd.stream(&sd.init.streams[0]).unwrap();
    assert_eq!(s.header.sample_rate, 32000);
    assert_eq!(s.stereo.len(), s.header.samples as usize * 2);
}
