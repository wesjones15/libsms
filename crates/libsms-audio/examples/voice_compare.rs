//! Ranks another disc's voice clips against Mario's voice cues, to find stand-ins for a skin.
//!
//! Each Mario cue (`MSD_SE_MV*`, `MSD_SE_MA_VO*`) is played through the full sound system and
//! rendered; each candidate wave of the other disc's JAudio folder is decoded as it is. Both
//! are reduced to what a listener hears first: length, loudness shape, pitch contour (voiced
//! frames, in semitones about the clip's own median) and brightness (zero crossings). The
//! closest few per cue are written next to Mario's own as WAVs to audition: it finds acoustic
//! lookalikes, not meaning, so a person still decides.
//!
//!     cargo run --release -p libsms-audio --example voice_compare -- <other disc's assets dir>
//!         <out dir> [bank:first-last ... | pool.txt]      (default 2:0-215 3:0-215)
//!
//! `pool.txt` is `rip_match`'s table: only those disc waves are candidates, labelled with the
//! rip's own names (`Luigi204`), which is how a labelled rip narrows the search to one speaker.
//!
//! `<other assets dir>` holds `AudioRes/msound.aaf` etc. under Sunshine's names (see
//! `wave_survey`); Mario's cues come from the normal `assets/` (`SMS_ASSETS`).

use std::path::Path;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use libsms_assets::sound::SoundData;
use libsms_audio::SoundSystem;

const ANALYSIS_RATE: f32 = 16000.0;

struct Clip {
    label: String,
    rate: f32,
    pcm: Vec<f32>,
    feat: Features,
}

struct Features {
    secs: f32,
    env: [f32; 16],
    /// Voiced fraction of the trimmed clip, and its median pitch in Hz (0 when unvoiced).
    voiced: f32,
    f0: f32,
    /// Pitch about the median in semitones at eight evenly spaced voiced points.
    contour: [f32; 8],
    zcr: f32,
}

fn resample(x: &[f32], from: f32, to: f32) -> Vec<f32> {
    let n = (x.len() as f32 * to / from) as usize;
    (0..n)
        .map(|i| {
            let p = i as f32 * from / to;
            let k = p as usize;
            let f = p - k as f32;
            x[k.min(x.len() - 1)] * (1.0 - f) + x[(k + 1).min(x.len() - 1)] * f
        })
        .collect()
}

fn trim(x: &[f32], rate: f32) -> &[f32] {
    let win = (rate * 0.01) as usize;
    let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt();
    let frames: Vec<f32> = x.chunks(win.max(1)).map(rms).collect();
    let peak = frames.iter().cloned().fold(0.0, f32::max);
    let thr = peak * 0.08;
    let first = frames.iter().position(|v| *v > thr).unwrap_or(0);
    let last = frames.iter().rposition(|v| *v > thr).unwrap_or(0);
    &x[(first * win).min(x.len())..((last + 1) * win).min(x.len())]
}

fn features(x: &[f32], rate: f32) -> Features {
    let t = trim(x, rate);
    let secs = t.len() as f32 / rate;
    let mut env = [0f32; 16];
    for (i, e) in env.iter_mut().enumerate() {
        let seg = &t[t.len() * i / 16..t.len() * (i + 1) / 16];
        *e = (seg.iter().map(|v| v * v).sum::<f32>() / seg.len().max(1) as f32).sqrt();
    }
    let m = env.iter().cloned().fold(1e-9, f32::max);
    env.iter_mut().for_each(|e| *e /= m);
    let zcr = t.windows(2).filter(|w| w[0] * w[1] < 0.0).count() as f32 / secs.max(0.01) / rate;

    // Pitch: normalised autocorrelation on 40 ms frames of the 16 kHz signal.
    let a = resample(t, rate, ANALYSIS_RATE);
    let (frame, hop) = (640usize, 160usize);
    let max_rms = a
        .chunks(hop)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())
        .fold(1e-9, f32::max);
    let mut f0s: Vec<Option<f32>> = Vec::new();
    let mut at = 0;
    while at + frame <= a.len() {
        let w = &a[at..at + frame];
        let e0: f32 = w.iter().map(|v| v * v).sum();
        let rms = (e0 / frame as f32).sqrt();
        let mut best = (0usize, 0f32);
        if rms > 0.1 * max_rms {
            for lag in 32..200 {
                let r: f32 = (0..frame - lag).map(|i| w[i] * w[i + lag]).sum::<f32>() / e0.max(1e-9);
                if r > best.1 {
                    best = (lag, r);
                }
            }
        }
        f0s.push((best.1 > 0.5).then(|| ANALYSIS_RATE / best.0 as f32));
        at += hop;
    }
    let voiced: Vec<f32> = f0s.iter().flatten().cloned().collect();
    let voiced_frac = voiced.len() as f32 / f0s.len().max(1) as f32;
    let mut sorted = voiced.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let f0 = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let mut contour = [0f32; 8];
    if voiced.len() >= 3 && f0 > 0.0 {
        for (i, c) in contour.iter_mut().enumerate() {
            let k = ((voiced.len() - 1) * i / 7).min(voiced.len() - 1);
            *c = 12.0 * (voiced[k] / f0).log2();
        }
    }
    Features { secs, env, voiced: voiced_frac, f0, contour, zcr }
}

/// Lower is closer: length, loudness shape, pitch movement, how voiced, brightness.
fn distance(a: &Features, b: &Features) -> f32 {
    let len = (a.secs / b.secs.max(0.02)).ln().abs();
    let env = (a.env.iter().zip(&b.env).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / 16.0).sqrt();
    let both = a.voiced > 0.2 && b.voiced > 0.2;
    let contour = if both {
        (a.contour.iter().zip(&b.contour).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / 8.0).sqrt() / 6.0
    } else if a.voiced > 0.2 || b.voiced > 0.2 {
        1.0
    } else {
        0.0
    };
    let voiced = (a.voiced - b.voiced).abs();
    let bright = (a.zcr - b.zcr).abs() * 8.0;
    1.0 * len + 1.5 * env + 1.0 * contour + 0.6 * voiced + 0.5 * bright
}

fn write_wav(path: &Path, rate: u32, pcm: &[f32]) {
    let data = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0001_0001, rate, rate * 2, 0x0010_0002] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// One second and a half of `name` as Mario's voice, mono at 32 kHz; None if it won't start.
fn render_cue(data: &Arc<SoundData>, name: &str) -> Option<Vec<f32>> {
    let mut s = SoundSystem::new(data.clone());
    s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
    s.mario_voice(name)?;
    let mut out = Vec::new();
    let mut buf = vec![0f32; (32000 / 30) * 2];
    for _ in 0..105 {
        s.frame();
        s.render(&mut buf, 32000.0);
        out.extend(buf.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5));
    }
    (out.iter().any(|v| v.abs() > 0.01)).then_some(out)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let other_dir = args.first().expect("<other assets dir>");
    let out_dir = Path::new(args.get(1).expect("<out dir>"));
    // A `rip_match` table: (label, bank, index) of the waves to consider, first of any duplicates.
    let mut pool: Vec<(String, usize, usize)> = Vec::new();
    if let Some(t) = args.get(2).filter(|a| a.ends_with(".txt")) {
        for line in std::fs::read_to_string(t).expect("pool table").lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if let [label, bank, _group, idx, ..] = f.as_slice()
                && let (Ok(b), Ok(k)) = (bank.parse::<usize>(), idx.parse::<usize>())
                && !pool.iter().any(|p| p.1 == b && p.2 == k)
            {
                pool.push((label.to_string(), b, k));
            }
        }
    }
    let mut ranges: Vec<(usize, usize, usize)> = args[2.min(args.len())..]
        .iter()
        .filter_map(|a| {
            let (b, r) = a.split_once(':')?;
            let (lo, hi) = r.split_once('-')?;
            Some((b.parse().ok()?, lo.parse().ok()?, hi.parse().ok()?))
        })
        .collect();
    if ranges.is_empty() && pool.is_empty() {
        ranges = vec![(2, 0, 215), (3, 0, 215)];
    }

    let mario_dir = libsms::assets::find(None).expect("no Sunshine assets/");
    let mario = Arc::new(SoundData::load(&mario_dir).expect("Sunshine sound data"));
    let other = SoundData::load(Path::new(other_dir)).expect("other sound data");

    // Candidates: one-shot waves in the requested ranges, voice-sized.
    let mut candidates: Vec<Clip> = Vec::new();
    let mut wanted: Vec<(String, usize, usize)> = pool.clone();
    for &(bank, lo, hi) in &ranges {
        wanted.extend((lo..=hi).map(|k| (format!("b{bank}_{k:03}"), bank, k)));
    }
    for (label, bank, k) in wanted {
        {
            let Some(w) = other.wave((bank, 0, k)) else { continue };
            if w.looping {
                continue;
            }
            let Ok(pcm) = other.pcm((bank, 0, k)) else { continue };
            let secs = pcm.len() as f32 / w.sample_rate.max(1.0);
            if !(0.1..4.0).contains(&secs) {
                continue;
            }
            let f: Vec<f32> = pcm.iter().map(|s| *s as f32 / 32768.0).collect();
            let feat = features(&f, w.sample_rate);
            if feat.secs < 0.08 {
                continue;
            }
            let label = if label.starts_with('b') { label } else { format!("{label}_b{bank}_{k}") };
            candidates.push(Clip { label, rate: w.sample_rate, pcm: f, feat });
        }
    }
    println!("{} candidate clips", candidates.len());

    let mut names: Vec<String> = mario
        .names
        .iter()
        .map(|(n, _)| n.to_string())
        .filter(|n| n.starts_with("MSD_SE_MV") || n.starts_with("MSD_SE_MA_VO"))
        .collect();
    names.sort();
    println!("{:<34} {:>5} {:>6} {:>5}   best matches (distance, bank_index, secs, median Hz)", "Mario cue", "secs", "Hz", "voice");
    for name in &names {
        let Some(r) = render_cue(&mario, name) else { continue };
        let mf = features(&r, 32000.0);
        if mf.secs < 0.05 {
            continue;
        }
        let mut ranked: Vec<(f32, &Clip)> = candidates.iter().map(|c| (distance(&mf, &c.feat), c)).collect();
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        let line: Vec<String> = ranked
            .iter()
            .take(5)
            .map(|(d, c)| format!("{d:.2} {} {:.2}s {:.0}Hz", c.label, c.feat.secs, c.feat.f0))
            .collect();
        println!("{:<34} {:>5.2} {:>6.0} {:>5.2}   {}", name.trim_start_matches("MSD_SE_"), mf.secs, mf.f0, mf.voiced, line.join(" | "));
        let dir = out_dir.join(name.trim_start_matches("MSD_SE_"));
        std::fs::create_dir_all(&dir).unwrap();
        let t = trim(&r, 32000.0);
        write_wav(&dir.join("00_mario.wav"), 32000, t);
        for (i, (d, c)) in ranked.iter().take(5).enumerate() {
            write_wav(&dir.join(format!("{:02}_{}_d{:.2}.wav", i + 1, c.label, d)), c.rate as u32, trim(&c.pcm, c.rate));
        }
    }
}
