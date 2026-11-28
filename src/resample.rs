//! Streaming sample-rate conversion for call audio (device rate ↔ 16 kHz Opus).
//!
//! The call pipeline used to pick the nearest sample with no filter and restarted its read position
//! in every audio callback. With a 44.1 kHz microphone that aliases high frequencies into the voice
//! band and adds a small jump at every callback boundary, heard as a buzz / crackle ("rè"). This
//! resampler keeps its fractional position and filter state across chunks, interpolates linearly,
//! and band-limits the signal with two cascaded biquad low-pass filters.

/// RBJ low-pass biquad (direct form I).
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn low_pass(sample_rate: f64, cutoff: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * (cutoff / sample_rate).clamp(1e-4, 0.49);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos) / 2.0 / a0) as f32,
            b1: ((1.0 - cos) / a0) as f32,
            b2: ((1.0 - cos) / 2.0 / a0) as f32,
            a1: (-2.0 * cos / a0) as f32,
            a2: ((1.0 - alpha) / a0) as f32,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        // Flush denormals so a silent stream does not slow the audio thread down.
        self.y1 = if y.abs() < 1e-20 { 0.0 } else { y };
        self.y1
    }
}

/// Two cascaded biquads: 24 dB/octave.
#[derive(Clone, Copy, Debug)]
struct LowPass([Biquad; 2]);

impl LowPass {
    fn new(sample_rate: f64, cutoff: f64) -> Self {
        let stage = Biquad::low_pass(sample_rate, cutoff);
        Self([stage, stage])
    }

    fn run(&mut self, x: f32) -> f32 {
        // One NaN/inf from a device would otherwise stay in the filter state for the whole call.
        let x = if x.is_finite() { x } else { 0.0 };
        let y = self.0[0].run(x);
        self.0[1].run(y)
    }
}

/// Mono streaming resampler; feed chunks of any size, outputs come out continuous.
#[derive(Clone, Debug)]
pub struct StreamResampler {
    /// Input samples per output sample.
    step: f64,
    /// Read position of the next output, relative to the start of the next chunk
    /// (`-1.0` is the last sample of the previous chunk).
    pos: f64,
    last: f32,
    /// Anti-aliasing filter before decimation (input rate).
    pre: Option<LowPass>,
    /// Anti-imaging filter after interpolation (output rate).
    post: Option<LowPass>,
    scratch: Vec<f32>,
}

impl StreamResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let in_rate = f64::from(in_rate.max(1));
        let out_rate = f64::from(out_rate.max(1));
        // Keep 90 % of the lower Nyquist band (7.2 kHz for 16 kHz audio).
        let cutoff = 0.45 * in_rate.min(out_rate);
        Self {
            step: in_rate / out_rate,
            pos: 0.0,
            last: 0.0,
            pre: (in_rate > out_rate).then(|| LowPass::new(in_rate, cutoff)),
            post: (out_rate > in_rate).then(|| LowPass::new(out_rate, cutoff)),
            scratch: Vec::new(),
        }
    }

    /// Resample `input` and append the result to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        if (self.step - 1.0).abs() < f64::EPSILON {
            out.extend_from_slice(input);
            return;
        }
        self.scratch.clear();
        match self.pre.as_mut() {
            Some(filter) => self
                .scratch
                .extend(input.iter().map(|&sample| filter.run(sample))),
            None => self.scratch.extend_from_slice(input),
        }
        let len = self.scratch.len();
        let last_index = (len - 1) as f64;
        let mut pos = self.pos;
        while pos <= last_index {
            let base = pos.floor();
            let frac = (pos - base) as f32;
            let index = base as isize;
            let s0 = if index < 0 {
                self.last
            } else {
                self.scratch[index as usize]
            };
            let sample = if frac == 0.0 {
                s0
            } else {
                let s1 = self.scratch[(index + 1) as usize];
                s0 + (s1 - s0) * frac
            };
            out.push(match self.post.as_mut() {
                Some(filter) => filter.run(sample),
                None => sample,
            });
            pos += self.step;
        }
        self.pos = pos - len as f64;
        self.last = self.scratch[len - 1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin() as f32 * 0.8
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        // Skip the filter's settling time.
        let tail = &samples[samples.len() / 4..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    }

    // The last fraction of a chunk is produced with the next chunk, so a few samples may lag.
    #[test]
    fn output_length_follows_the_rate_ratio() {
        for (from, to) in [(44_100, 16_000), (48_000, 16_000), (16_000, 48_000)] {
            let mut resampler = StreamResampler::new(from, to);
            let mut out = Vec::new();
            resampler.process(&vec![0.0; from as usize], &mut out);
            assert!(
                (out.len() as i64 - i64::from(to)).abs() <= 4,
                "{from}->{to}: {}",
                out.len()
            );
        }
    }

    #[test]
    fn chunked_input_matches_one_block() {
        let input = sine(440.0, 44_100, 9_000);
        let mut whole = Vec::new();
        StreamResampler::new(44_100, 16_000).process(&input, &mut whole);
        let mut chunked = Vec::new();
        let mut resampler = StreamResampler::new(44_100, 16_000);
        for chunk in input.chunks(471) {
            resampler.process(chunk, &mut chunked);
        }
        assert_eq!(whole.len(), chunked.len());
        for (a, b) in whole.iter().zip(&chunked) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn voice_band_passes_and_high_frequencies_do_not_alias() {
        let mut out = Vec::new();
        StreamResampler::new(44_100, 16_000).process(&sine(1_000.0, 44_100, 44_100), &mut out);
        assert!(rms(&out) > 0.5, "1 kHz kept: {}", rms(&out));

        let mut aliased = Vec::new();
        StreamResampler::new(44_100, 16_000).process(&sine(14_000.0, 44_100, 44_100), &mut aliased);
        assert!(
            rms(&aliased) < 0.05,
            "14 kHz must not fold into the voice band: {}",
            rms(&aliased)
        );
    }

    #[test]
    fn upsampling_keeps_the_voice_band() {
        let mut out = Vec::new();
        StreamResampler::new(16_000, 48_000).process(&sine(1_000.0, 16_000, 16_000), &mut out);
        assert!(rms(&out) > 0.5, "{}", rms(&out));
    }
}
