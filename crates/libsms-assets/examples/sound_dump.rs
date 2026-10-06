//! Lists what the sound data holds and decodes waves, as a check of the readers.
//!
//! `cargo run -p libsms-assets --example sound_dump [-- <MSD_NAME> | wav <bank> <group> <wave> <out.wav>]`

use std::path::Path;

use libsms_assets::sound::{SoundData, bank::Program};

fn write_wav(path: &str, rate: u32, channels: u16, pcm: &[i16]) {
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    let data = (pcm.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

fn main() {
    let assets = libsms::assets::find(None).expect("no assets/ folder");
    let sd = SoundData::load(Path::new(&assets)).expect("sound data");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("wav") {
        let n = |i: usize| args[i].parse::<usize>().unwrap();
        let key = (n(1), n(2), n(3));
        let w = *sd.wave(key).expect("no such wave");
        let pcm = sd.pcm(key).unwrap();
        write_wav(&args[4], w.sample_rate as u32, 1, &pcm);
        println!("{w:?} -> {} samples", pcm.len());
        return;
    }
    if args.first().map(String::as_str) == Some("stream") {
        let s = sd.stream(&args[1]).unwrap();
        write_wav(&args[2], s.header.sample_rate as u32, 2, &s.stereo);
        println!("{:?}", s.header);
        return;
    }
    if let Some(name) = args.first() {
        let id = sd.names.id(name).expect("unknown name");
        println!("{name} = {id:#010x}: {:?}", sd.init.sound_table.info(id));
        return;
    }
    let t = &sd.init.sound_table;
    println!(
        "sound table formats {:?}, {} SE categories, counts {:?}",
        t.formats,
        t.se_categories(),
        t.categories.iter().map(Vec::len).collect::<Vec<_>>()
    );
    println!("{} names", sd.names.len());
    println!("sound scenes: {:?}", sd.init.scenes);
    for (i, b) in sd.init.banks.iter().enumerate() {
        let insts = b
            .bank
            .programs
            .iter()
            .filter(|p| matches!(p, Some(Program::Inst(_))))
            .count();
        let percs = b
            .bank
            .programs
            .iter()
            .filter(|p| matches!(p, Some(Program::Perc(_))))
            .count();
        println!(
            "bank {i}: virtual id {}, wave bank {}, {insts} instruments, {percs} drum sets",
            b.bank.virtual_id, b.wave_bank
        );
    }
    let mut decoded = 0;
    for (i, wb) in sd.init.wave_banks.iter().enumerate() {
        let waves: usize = wb.bank.groups.iter().map(|g| g.waves.len()).sum();
        println!(
            "wave bank {i}: timing {}, {} groups, {waves} waves, files {:?}",
            wb.load_timing,
            wb.bank.groups.len(),
            wb.bank
                .groups
                .iter()
                .map(|g| g.file.as_str())
                .collect::<Vec<_>>()
        );
        for (g, grp) in wb.bank.groups.iter().enumerate() {
            for k in 0..grp.waves.len() {
                sd.pcm((i, g, k))
                    .unwrap_or_else(|e| panic!("wave {i}/{g}/{k}: {e}"));
                decoded += 1;
            }
        }
    }
    println!("decoded {decoded} waves");
    println!("{} sequences:", sd.seqs.len());
    for n in 0..sd.seqs.len() {
        print!("{n}:{} ", sd.seqs.name(n).unwrap_or("?"));
    }
    println!();
    println!("streams {:?}", sd.init.streams);
}
