//! Tabulates every wave in a JAudio folder (`SMS_ASSETS=<folder with AudioRes/>`), and with an
//! output directory writes the one-shot ones as WAVs named `<bank>_<group>_<index>`, to find
//! voice lines on a disc that has no name table (Luigi's Mansion):
//! `cargo run -p libsms-assets --example wave_survey -- [out dir]`.

use std::path::Path;

use libsms_assets::sound::SoundData;

fn write_wav(path: &Path, rate: u32, pcm: &[i16]) {
    let data = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
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
    let sd = SoundData::load(&assets).expect("sound data");
    let out = std::env::args().nth(1);
    if let Some(d) = &out {
        std::fs::create_dir_all(d).unwrap();
    }
    println!("bank group index rate seconds looping peak");
    for (i, wb) in sd.init.wave_banks.iter().enumerate() {
        for (g, grp) in wb.bank.groups.iter().enumerate() {
            for k in 0..grp.waves.len() {
                let Some(w) = sd.wave((i, g, k)) else { continue };
                let Ok(pcm) = sd.pcm((i, g, k)) else { continue };
                let secs = pcm.len() as f32 / w.sample_rate.max(1.0);
                let peak = pcm.iter().map(|s| (*s as i32).abs()).max().unwrap_or(0);
                println!("{i} {g} {k} {} {secs:.2} {} {peak}", w.sample_rate as u32, w.looping as u8);
                if let Some(d) = &out
                    && !w.looping
                    && (0.12..6.0).contains(&secs)
                {
                    write_wav(&Path::new(d).join(format!("{i}_{g}_{k:03}_{secs:.1}s.wav")), w.sample_rate as u32, &pcm);
                }
            }
        }
    }
}
