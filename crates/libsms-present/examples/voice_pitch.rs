//! Luigi's voice experiment: take one of Mario's voice cues from the disc, render it, and
//! write it again pitched up 14% with the *same duration* (resample for the pitch, then a
//! WSOLA overlap-add pass for the length). Writes both to `target/` so they can be compared.
//!
//! Run from `libsms/` (the extracted `assets/` folder must exist):
//!   cargo run -p libsms-present --example voice_pitch -- [cue]
//!
//! Default cue: `MSD_SE_MV24_JUMP_SPECIAL_01` (the "wahoo" jump voice).

use std::sync::Arc;

use libsms_audio::jai::SoundSystem;
use libsms_assets::sound::SoundData;

const RATE: f32 = 32000.0;
const FRAME: usize = 32000 / 30; // JAI runs at 30 Hz

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::var("SMS_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("assets"));
    let cue = std::env::args().nth(1).unwrap_or_else(|| "MSD_SE_MV24_JUMP_SPECIAL_01".into());
    let data = Arc::new(SoundData::load(&dir)?);
    let mut sys = SoundSystem::new(data);
    if sys.play(&cue, None, 0x4D41_5249).is_none() {
        return Err(format!("could not start {cue}").into());
    }

    // Stereo interleaved, 30 frames (one second) is plenty for a bark.
    let mut stereo = Vec::new();
    let mut buf = vec![0.0f32; FRAME * 2];
    for _ in 0..36 {
        sys.frame();
        sys.render(&mut buf, RATE);
        stereo.extend_from_slice(&buf);
    }
    // Trim the trailing silence so the two files line up and stay short.
    let peak = stereo.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 1e-4 {
        return Err(format!("{cue} rendered silence").into());
    }
    let last = stereo
        .chunks_exact(2)
        .rposition(|f| f[0].abs().max(f[1].abs()) > peak * 0.01)
        .unwrap_or(0);
    stereo.truncate((last + 1).min(stereo.len() / 2) * 2);
    let len_frames = stereo.len() / 2;

    let l: Vec<f32> = stereo.chunks_exact(2).map(|f| f[0]).collect();
    let r: Vec<f32> = stereo.chunks_exact(2).map(|f| f[1]).collect();
    let (pl, pr) = (pitch_up(&l, 1.14), pitch_up(&r, 1.14));

    let out = std::path::Path::new("target");
    std::fs::create_dir_all(out)?;
    let base = "luigi_voice";
    std::fs::write(out.join(format!("{base}_orig.wav")), wav(&stereo, len_frames))?;
    let mut shifted = Vec::with_capacity(stereo.len());
    for (a, b) in pl.iter().zip(&pr) {
        shifted.push(*a);
        shifted.push(*b);
    }
    std::fs::write(
        out.join(format!("{base}_114.wav")),
        wav(&shifted, shifted.len() / 2),
    )?;
    println!(
        "wrote target/{base}_orig.wav and target/{base}_114.wav ({:.2} s each, cue {cue})",
        len_frames as f32 / RATE
    );
    Ok(())
}

/// Pitch by `ratio` with the length kept: resample up (shorter), then WSOLA back to the
/// original length. Plain overlap-add with a small correlation search for the analysis offset.
fn pitch_up(x: &[f32], ratio: f32) -> Vec<f32> {
    if ratio <= 0.0 {
        return x.to_vec();
    }
    // Resample: read the source `ratio` samples per output sample (linear interpolation).
    let n = (x.len() as f32 / ratio).floor() as usize;
    let mut y = Vec::with_capacity(n);
    let mut t = 0.0f32;
    for _ in 0..n {
        let i = t as usize;
        if i + 1 >= x.len() {
            break;
        }
        let f = t - i as f32;
        y.push(x[i] * (1.0 - f) + x[i + 1] * f);
        t += ratio;
    }
    wsola(&y, x.len())
}

fn wsola(y: &[f32], target: usize) -> Vec<f32> {
    const WIN: usize = 512;
    const HOP: usize = WIN / 2;
    const SEARCH: usize = 96;
    let win: Vec<f32> = (0..WIN)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WIN as f32).cos())
        .collect();
    let analyse_hop = HOP as f32 * (y.len() as f32 / target as f32);
    let mut out = vec![0.0f32; target + WIN];
    let mut wsum = vec![0.0f32; target + WIN];
    let mut pos = 0.0f32;
    let mut o = 0usize;
    let mut prev: Vec<f32> = Vec::new();
    while o + WIN <= target + WIN && (pos as usize) + SEARCH + WIN < y.len() {
        // Pick the lag near `pos` whose window best continues the previous one.
        let mut best = 0i32;
        if !prev.is_empty() {
            let mut best_score = f32::NEG_INFINITY;
            for lag in -(SEARCH as i32)..=(SEARCH as i32) {
                let at = pos as i32 + lag;
                if at < 0 || (at as usize) + HOP >= y.len() {
                    continue;
                }
                let mut score = 0.0f32;
                for i in 0..HOP {
                    score += prev[i] * y[at as usize + i];
                }
                if score > best_score {
                    best_score = score;
                    best = lag;
                }
            }
        }
        let p = (pos as i32 + best).max(0) as usize;
        if p + WIN >= y.len() {
            break;
        }
        for i in 0..WIN {
            out[o + i] += y[p + i] * win[i];
            wsum[o + i] += win[i];
        }
        prev = y[p + HOP..p + HOP + HOP].to_vec();
        pos += analyse_hop;
        o += HOP;
    }
    let mut res: Vec<f32> = out
        .iter()
        .zip(&wsum)
        .map(|(s, w)| if *w > 0.25 { s / *w } else { 0.0 })
        .collect();
    res.truncate(target);
    res
}

fn wav(samples: &[f32], frames: usize) -> Vec<u8> {
    let data: Vec<i16> = samples
        .iter()
        .take(frames * 2)
        .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();
    let bytes: Vec<u8> = data.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut w = Vec::with_capacity(44 + bytes.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + bytes.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&(RATE as u32).to_le_bytes());
    w.extend_from_slice(&((RATE as u32) * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    w.extend_from_slice(&bytes);
    w
}
