//! Plays sounds through the full system (SE sequence, music, voice) and writes a WAV.
//!
//! `cargo run -p libsms-audio --example play -- <out.wav> <seconds> [MSD_NAME[@second] ...]`
//! Sound effects play at the listener; `MSD_BGM_*` names start music.

use std::sync::Arc;

use libsms_assets::sound::SoundData;
use libsms_audio::SoundSystem;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or("play.wav".into());
    let secs: f32 = args.get(1).map_or(4.0, |s| s.parse().unwrap());
    let mut cues: Vec<(f32, String)> = args[2.min(args.len())..]
        .iter()
        .map(|a| match a.split_once('@') {
            Some((n, t)) => (t.parse().unwrap(), n.to_string()),
            None => (0.0, a.clone()),
        })
        .collect();
    cues.sort_by(|a, b| a.0.total_cmp(&b.0));
    let dir = libsms::assets::find(None).expect("no assets/");
    let data = Arc::new(SoundData::load(&dir).unwrap());
    let mut s = SoundSystem::new(data);
    let rate = 32000.0;
    let frame_len = (rate / 30.0) as usize;
    let mut pcm = Vec::new();
    let mut buf = vec![0f32; frame_len * 2];
    let mut peak = (0, 0);
    let frames = (secs * 30.0) as usize;
    for f in 0..frames {
        let t = f as f32 / 30.0;
        while cues.first().is_some_and(|c| c.0 <= t) {
            let (_, name) = cues.remove(0);
            let ok = if name.starts_with("MSD_SE_MV") {
                s.mario_voice(&name).is_some()
            } else if name.starts_with("MSD_STR") || name.ends_with(".afc") {
                s.play_stream("title.afc")
            } else {
                s.play(&name, None, f as u64).is_some() || s.bgm_playing().is_some()
            };
            println!("{t:5.2}s {name}: {}", if ok { "started" } else { "FAILED" });
        }
        s.frame();
        s.render(&mut buf, rate);
        peak = (peak.0.max(s.active_voices()), peak.1.max(s.active_sounds()));
        pcm.extend(buf.iter().map(|v| (v * 32767.0) as i16));
    }
    let rms =
        (pcm.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
    println!(
        "peak voices {}, peak sounds {}, rms {rms:.0}",
        peak.0, peak.1
    );
    let mut w = Vec::new();
    let data_len = (pcm.len() * 2) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [
        16u32.to_le_bytes(),
        [1, 0, 2, 0],
        32000u32.to_le_bytes(),
        128000u32.to_le_bytes(),
        [4, 0, 16, 0],
    ] {
        w.extend_from_slice(&v);
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for v in pcm {
        w.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(out, w).unwrap();
}
