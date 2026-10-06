//! Renders a sequence from `sequence.arc` straight through the engine to a WAV file.
//!
//! `cargo run -p libsms-audio --example render_seq -- <seq number> <seconds> <out.wav>`

use std::sync::Arc;

use libsms_assets::sound::SoundData;
use libsms_audio::calc::{DAC_RATE, SUBFRAME};
use libsms_audio::{Engine, NoSync};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = args.first().map_or(1, |s| s.parse().unwrap());
    let secs: f32 = args.get(1).map_or(10.0, |s| s.parse().unwrap());
    let out = args.get(2).cloned().unwrap_or("seq.wav".into());
    let dir = libsms::assets::find(None).expect("no assets/");
    let data = Arc::new(SoundData::load(&dir).unwrap());
    let seq: Arc<[u8]> = data.seqs.seq(n).expect("no such sequence").into();
    println!("{}", data.seqs.name(n).unwrap_or("?"));
    let mut e = Engine::new(data);
    let h = e.set_seq_data(seq).expect("no root");
    e.start_seq(h);
    let frames = (secs * DAC_RATE / SUBFRAME as f32) as usize;
    let mut pcm = Vec::with_capacity(frames * SUBFRAME * 2);
    let mut buf = [0f32; SUBFRAME * 2];
    let mut peak_voices = 0;
    for _ in 0..frames {
        e.subframe(&mut NoSync, &mut buf);
        peak_voices = peak_voices.max(e.active_voices());
        pcm.extend(buf.iter().map(|s| (s * 32767.0) as i16));
    }
    let rms = (pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
    let clip = pcm.iter().filter(|s| s.unsigned_abs() >= 32767).count();
    println!("peak voices {peak_voices}, rms {rms:.0}, clipped {clip}");
    let mut w = Vec::new();
    let rate = DAC_RATE as u32;
    let data_len = (pcm.len() * 2) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&out, w).unwrap();
}
