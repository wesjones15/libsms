/// Stereo Butterworth low-pass for a host's playback tone; zero cutoff bypasses it.
/// State stays on the audio thread, independently for left and right.
pub(crate) struct LowPass {
    rate: f32,
    cutoff: f32,
    b: [f32; 3],
    a: [f32; 2],
    state: [[f32; 2]; 2],
}

impl LowPass {
    pub fn new(rate: f32) -> Self {
        Self {
            rate,
            cutoff: 0.0,
            b: [1.0, 0.0, 0.0],
            a: [0.0; 2],
            state: [[0.0; 2]; 2],
        }
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        let cutoff = if cutoff.is_finite() {
            cutoff.clamp(0.0, self.rate * 0.45)
        } else {
            0.0
        };
        if cutoff == self.cutoff {
            return;
        }
        self.cutoff = cutoff;
        self.state = [[0.0; 2]; 2];
        if cutoff == 0.0 {
            return;
        }
        let w = std::f32::consts::TAU * cutoff / self.rate;
        let alpha = w.sin() / std::f32::consts::SQRT_2;
        let norm = 1.0 / (1.0 + alpha);
        self.b = [
            (1.0 - w.cos()) * 0.5 * norm,
            (1.0 - w.cos()) * norm,
            (1.0 - w.cos()) * 0.5 * norm,
        ];
        self.a = [-2.0 * w.cos() * norm, (1.0 - alpha) * norm];
    }

    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.cutoff == 0.0 {
            return input;
        }
        std::array::from_fn(|channel| {
            let s = &mut self.state[channel];
            let y = self.b[0] * input[channel] + s[0];
            s[0] = self.b[1] * input[channel] - self.a[0] * y + s[1];
            s[1] = self.b[2] * input[channel] - self.a[1] * y;
            y
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softens_treble_preserves_voice_band_and_stereo_at_device_rates() {
        for rate in [32000.0, 48000.0] {
            for (frequency, min, max) in [
                (1000.0, 0.95, 1.01),
                (8000.0, 0.0, 0.3),
                (12000.0, 0.0, 0.1),
            ] {
                let mut filter = LowPass::new(rate);
                filter.set_cutoff(4500.0);
                let mut before = 0.0f64;
                let mut after = 0.0f64;
                for i in 0..rate as usize {
                    let x = (std::f32::consts::TAU * frequency * i as f32 / rate).sin();
                    let y = filter.process([x, 0.0]);
                    assert!(y[0].is_finite() && y[1] == 0.0);
                    if i > 100 {
                        before += (x as f64).powi(2);
                        after += (y[0] as f64).powi(2);
                    }
                }
                let gain = (after / before).sqrt();
                assert!(
                    gain >= min && gain <= max,
                    "{rate} Hz, {frequency} Hz: gain {gain}"
                );
            }
            let mut filter = LowPass::new(rate);
            assert_eq!(filter.process([0.3, -0.7]), [0.3, -0.7]);
            filter.set_cutoff(4500.0);
            filter.process([1.0, 1.0]);
            filter.set_cutoff(0.0);
            assert_eq!(filter.process([0.3, -0.7]), [0.3, -0.7]);
        }
    }
}
