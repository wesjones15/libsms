//! The GameCube's AFC ADPCM, as the DSP decodes it for bank waves. The DSP microcode isn't in
//! the decomp; the frame layout and predictor table are the ones the CPU-side stream decoder
//! uses (`filter_table`, JAIGFrameStream.cpp), and the 2-bit form follows `JASystem::Driver`'s
//! wave formats (`Wave_::unk1`: 0 = 4-bit ADPCM, 1 = 2-bit ADPCM, 2 = PCM8, 3 = PCM16).
//!
//! A 4-bit frame is a header byte (scale in the high nibble, predictor in the low nibble) and
//! 8 bytes of 16 signed nibbles. The 2-bit form packs 16 two-bit codes into 4 bytes; the DSP
//! scales them so they cover the same range as nibbles (the code is shifted 2 bits further).

/// `filter_table` (JAIGFrameStream.cpp): predictor coefficients in 1/2048ths.
const COEFS: [[i32; 2]; 16] = [
    [0, 0],
    [2048, 0],
    [0, 2048],
    [1024, 1024],
    [4096, -2048],
    [3584, -1536],
    [3072, -1024],
    [4608, -2560],
    [4200, -2248],
    [4800, -2300],
    [5120, -3072],
    [2048, -2048],
    [1024, -1024],
    [-1024, 1024],
    [-1024, 0],
    [-2048, 0],
];

/// Decoder history: the last two samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct History {
    pub hist1: i16,
    pub hist2: i16,
}

fn predict(code: i32, shift: u32, coef: [i32; 2], h: &mut History) -> i16 {
    let s = (code << shift) + ((coef[0] * h.hist1 as i32 + coef[1] * h.hist2 as i32) >> 11);
    let s = s.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    h.hist2 = h.hist1;
    h.hist1 = s;
    s
}

/// Decodes one 9-byte 4-bit frame into 16 samples.
pub fn decode_frame4(frame: &[u8], h: &mut History, out: &mut [i16; 16]) {
    let shift = (frame[0] >> 4) as u32;
    let coef = COEFS[(frame[0] & 0xF) as usize];
    for i in 0..16 {
        let b = frame[1 + i / 2];
        let nib = if i % 2 == 0 { b >> 4 } else { b & 0xF };
        let code = ((nib as i8) << 4 >> 4) as i32;
        out[i] = predict(code, shift, coef, h);
    }
}

/// Decodes one 5-byte 2-bit frame into 16 samples.
pub fn decode_frame2(frame: &[u8], h: &mut History, out: &mut [i16; 16]) {
    let shift = (frame[0] >> 4) as u32 + 2;
    let coef = COEFS[(frame[0] & 0xF) as usize];
    for i in 0..16 {
        let b = frame[1 + i / 4];
        let bits = (b >> (6 - 2 * (i % 4))) & 3;
        let code = ((bits as i8) << 6 >> 6) as i32;
        out[i] = predict(code, shift, coef, h);
    }
}

/// Decodes a bank wave's bytes (`Wave::format` 0-3) into exactly `samples` samples. A few waves
/// state a sample count slightly past their bytes; the rest is silence.
pub fn decode_wave(format: u8, data: &[u8], samples: usize) -> Vec<i16> {
    let mut out = Vec::with_capacity(samples);
    let mut h = History::default();
    let mut buf = [0i16; 16];
    match format {
        0 | 1 => {
            let size = if format == 0 { 9 } else { 5 };
            for frame in data.chunks_exact(size) {
                if out.len() >= samples {
                    break;
                }
                if format == 0 {
                    decode_frame4(frame, &mut h, &mut buf);
                } else {
                    decode_frame2(frame, &mut h, &mut buf);
                }
                out.extend_from_slice(&buf[..16.min(samples - out.len())]);
            }
        }
        2 => out.extend(data.iter().take(samples).map(|&b| ((b as i8) as i16) << 8)),
        _ => out.extend(
            data.as_chunks::<2>()
                .0
                .iter()
                .take(samples)
                .map(|c| i16::from_be_bytes(*c)),
        ),
    }
    out.resize(samples, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_frame_is_silent() {
        let mut h = History::default();
        let mut out = [1i16; 16];
        decode_frame4(&[0; 9], &mut h, &mut out);
        assert_eq!(out, [0; 16]);
    }

    #[test]
    fn nibbles_are_signed_and_scaled() {
        // Predictor 0, scale 2: nibble 1 -> 4, nibble 0xF (-1) -> -4.
        let mut h = History::default();
        let mut out = [0i16; 16];
        decode_frame4(&[0x20, 0x1F, 0, 0, 0, 0, 0, 0, 0], &mut h, &mut out);
        assert_eq!(&out[..3], &[4, -4, 0]);
    }

    #[test]
    fn predictor_uses_history() {
        // Predictor 1 = hist1 * 2048 / 2048: a single impulse holds.
        let mut h = History::default();
        let mut out = [0i16; 16];
        decode_frame4(&[0x01, 0x70, 0, 0, 0, 0, 0, 0, 0], &mut h, &mut out);
        assert!(out.iter().all(|&s| s == 7));
    }
}
