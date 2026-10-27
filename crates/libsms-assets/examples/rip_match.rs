//! Finds which waves of a disc's own JAudio banks a folder of ripped WAVs came from, so a
//! labelled rip can name unlabelled disc waves (the disc stays the source of the audio):
//!
//!     SMS_ASSETS=<folder with AudioRes/ and params_mario/> \
//!       cargo run --release -p libsms-assets --example rip_match -- <rip folder> [out table]
//!
//! Both sides go to 8 kHz mono. A clip is first shortlisted by loudness envelope (a 100 Hz
//! envelope correlation, length within 8%), then scored by normalised correlation over a few
//! hundred samples of lag, which absorbs a decoder's start offset and a resampler's delay.

use std::path::Path;

use libsms_assets::sound::SoundData;

const RATE: f32 = 8000.0;

fn read_wav(path: &Path) -> Option<(f32, Vec<f32>)> {
    let d = std::fs::read(path).ok()?;
    if d.get(0..4)? != b"RIFF" || d.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut at, mut rate, mut chans, mut bits, mut fmt) = (12, 0u32, 1usize, 16u16, 1u16);
    while at + 8 <= d.len() {
        let id = &d[at..at + 4];
        let len = u32::from_le_bytes(d[at + 4..at + 8].try_into().ok()?) as usize;
        let body = d.get(at + 8..(at + 8 + len).min(d.len()))?;
        if id == b"fmt " {
            fmt = u16::from_le_bytes(body[0..2].try_into().ok()?);
            chans = u16::from_le_bytes(body[2..4].try_into().ok()?) as usize;
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
        } else if id == b"data" {
            if fmt != 1 || bits != 16 {
                return None;
            }
            let s: Vec<f32> = body
                .chunks_exact(2 * chans)
                .map(|f| {
                    (0..chans).map(|c| i16::from_le_bytes([f[2 * c], f[2 * c + 1]]) as f32).sum::<f32>()
                        / chans as f32
                        / 32768.0
                })
                .collect();
            return Some((rate as f32, s));
        }
        at += 8 + len + (len & 1);
    }
    None
}

fn resample(x: &[f32], from: f32, to: f32) -> Vec<f32> {
    // Box-average when shrinking (a cheap low-pass), linear interpolation otherwise.
    let n = (x.len() as f32 * to / from) as usize;
    let ratio = from / to;
    (0..n)
        .map(|i| {
            let a = i as f32 * ratio;
            let b = ((i + 1) as f32 * ratio).min(x.len() as f32);
            let (lo, hi) = (a as usize, (b.ceil() as usize).min(x.len()));
            if hi > lo + 1 {
                x[lo..hi].iter().sum::<f32>() / (hi - lo) as f32
            } else {
                x[lo.min(x.len() - 1)]
            }
        })
        .collect()
}

fn trim(x: &[f32]) -> &[f32] {
    let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
    let thr = peak * 0.02;
    let a = x.iter().position(|v| v.abs() > thr).unwrap_or(0);
    let b = x.iter().rposition(|v| v.abs() > thr).unwrap_or(0);
    &x[a..=b.max(a)]
}

fn envelope(x: &[f32]) -> Vec<f32> {
    let win = (RATE / 100.0) as usize;
    let mut e: Vec<f32> = x.chunks(win).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let m = e.iter().cloned().fold(1e-9, f32::max);
    e.iter_mut().for_each(|v| *v /= m);
    e
}

fn env_dist(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    (a[..n].iter().zip(&b[..n]).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / n.max(1) as f32).sqrt()
}

/// Best normalised correlation over lags in -`max_lag`..=`max_lag`.
fn corr(a: &[f32], b: &[f32], max_lag: i32) -> f32 {
    let mut best = -1f32;
    for lag in -max_lag..=max_lag {
        let (sa, sb) = if lag >= 0 { (lag as usize, 0) } else { (0, (-lag) as usize) };
        let n = a.len().saturating_sub(sa).min(b.len().saturating_sub(sb));
        if n < 80 {
            continue;
        }
        let (x, y) = (&a[sa..sa + n], &b[sb..sb + n]);
        let (mut xy, mut xx, mut yy) = (0f32, 0f32, 0f32);
        for i in 0..n {
            xy += x[i] * y[i];
            xx += x[i] * x[i];
            yy += y[i] * y[i];
        }
        best = best.max(xy / (xx * yy).sqrt().max(1e-9));
    }
    best
}

struct Wave {
    id: (usize, usize, usize),
    signal: Vec<f32>,
    env: Vec<f32>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rip_dir = args.first().expect("<rip folder>");
    let assets = libsms::assets::find(None).expect("no assets folder (SMS_ASSETS)");
    let sd = SoundData::load(&assets).expect("sound data");

    let mut disc: Vec<Wave> = Vec::new();
    for (b, wb) in sd.init.wave_banks.iter().enumerate() {
        for (g, grp) in wb.bank.groups.iter().enumerate() {
            for k in 0..grp.waves.len() {
                let (Some(w), Ok(pcm)) = (sd.wave((b, g, k)), sd.pcm((b, g, k))) else { continue };
                if pcm.len() < 200 {
                    continue;
                }
                let f: Vec<f32> = pcm.iter().map(|s| *s as f32 / 32768.0).collect();
                let s = trim(&resample(&f, w.sample_rate.max(1.0), RATE)).to_vec();
                if s.len() < 80 {
                    continue;
                }
                disc.push(Wave { id: (b, g, k), env: envelope(&s), signal: s });
            }
        }
    }
    println!("{} disc waves", disc.len());

    let mut rips: Vec<_> = std::fs::read_dir(rip_dir)
        .expect("rip folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")))
        .collect();
    // Luigi2 before Luigi10: sort by the number in the name.
    rips.sort_by_key(|p| {
        let s = p.file_stem().unwrap().to_string_lossy().to_string();
        let n: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        (s.trim_end_matches(|c: char| c.is_ascii_digit()).to_string(), n.parse::<u32>().unwrap_or(0))
    });
    let mut table = String::from("rip bank group index score second runner_up_gap secs\n");
    let (mut exact, mut good, mut weak, mut none) = (0, 0, 0, 0);
    for p in &rips {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let Some((rate, pcm)) = read_wav(p) else {
            println!("{name}: unreadable (not 16-bit PCM)");
            continue;
        };
        let s = trim(&resample(&pcm, rate, RATE)).to_vec();
        if s.len() < 80 {
            println!("{name}: silent");
            none += 1;
            continue;
        }
        let env = envelope(&s);
        let mut short: Vec<(f32, &Wave)> = disc
            .iter()
            .filter(|w| (w.signal.len() as f32 / s.len() as f32 - 1.0).abs() < 0.08)
            .map(|w| (env_dist(&env, &w.env), w))
            .collect();
        short.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut scored: Vec<(f32, (usize, usize, usize))> =
            short.iter().take(12).map(|(_, w)| (corr(&s, &w.signal, 200), w.id)).collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let secs = s.len() as f32 / RATE;
        match scored.as_slice() {
            [] => {
                println!("{name}: no disc wave of that length");
                none += 1;
            }
            [(best, id), rest @ ..] => {
                let second = rest.first().map_or(0.0, |r| r.0);
                let tag = if *best > 0.97 {
                    exact += 1;
                    "EXACT"
                } else if *best > 0.85 {
                    good += 1;
                    "good"
                } else {
                    weak += 1;
                    "weak"
                };
                println!("{name:<10} -> bank {} group {} wave {:>3}  r={best:.3} (next {second:.3}) {secs:.2}s {tag}", id.0, id.1, id.2);
                table.push_str(&format!("{name} {} {} {} {best:.3} {second:.3} {:.3} {secs:.2}\n", id.0, id.1, id.2, best - second));
            }
        }
    }
    println!("{} rips: {exact} exact (r>0.97), {good} good (>0.85), {weak} weak, {none} unmatched", rips.len());
    if let Some(out) = args.get(1) {
        std::fs::write(out, table).unwrap();
    }
}
