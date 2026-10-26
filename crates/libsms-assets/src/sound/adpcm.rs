//! AFC ADPCM for bank waves: frame decoding lives in `libcommon::audio::afc` (shared with the
//! other GameCube JAudio libraries), re-exported here so the sound loaders keep their paths.
//! Bank waves are decoded as the DSP plays them (saturating).

pub use libcommon::audio::afc::{History, Overflow, decode_frame2, decode_frame4, decode_wave};

#[cfg(test)]
pub(crate) mod baseline {
    use super::*;

    pub fn fnv(samples: &[i16]) -> u64 {
        samples.iter().flat_map(|s| s.to_le_bytes()).fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        })
    }

    pub fn random(n: usize) -> Vec<u8> {
        let mut s = 0x9E37_79B9u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 24) as u8
            })
            .collect()
    }

    /// Output over random frames (large scales and every predictor, so samples saturate)
    /// pinned before the decoder moved to libcommon.
    #[test]
    fn decoded_waves_match_baseline() {
        let data = random(4096);
        let got: Vec<u64> = (0..=3).map(|f| fnv(&decode_wave(f, &data, 3000))).collect();
        println!("{got:#x?}");
        assert_eq!(got, BASELINE);
    }

    // Recorded from the decoder before it moved to libcommon::audio::afc.
    const BASELINE: [u64; 4] = [
        0x223853e150b32ece,
        0x89821f36cb54cef6,
        0xef99874cc370411c,
        0x991c82309938633e,
    ];
}
