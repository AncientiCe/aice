//! Acoustic echo cancellation for full-duplex pods.
//!
//! A full-duplex pod sends, with every microphone frame, the signal its
//! speaker was playing (the codec records the DAC output on a spare ADC
//! channel). The canceller first finds how far the echo lags that reference
//! (measured on a pod: one 40 ms frame plus 0.8 ms, stable within a session),
//! then learns the remaining speaker-to-microphone echo path with a normalized
//! LMS filter and subtracts the predicted echo, leaving the guest's voice.
//! Adaptation pauses while the guest talks over the answer (double talk), so
//! their voice does not bend the filter.

/// Echo path length the filter models, in samples (256 = 16 ms at 16 kHz).
pub const DEFAULT_ECHO_TAPS: usize = 256;

/// Step size of the normalized LMS update (0 < MU < 2; smaller is steadier).
const MU: f32 = 0.4;
/// Keeps the normalization finite when the speaker is almost silent.
const EPSILON: f32 = 1e-6;
/// Speaker energy (sum of squares over the filter, full scale = 1.0) below
/// which there is nothing to learn from.
const ACTIVE_REFERENCE_ENERGY: f32 = 1e-4;
/// Samples per double-talk decision (10 ms).
const BLOCK: usize = 160;
/// Adaptation with an active speaker before double talk is judged at all;
/// until then the echo estimate is too poor to compare against.
const WARM_UP_SAMPLES: usize = 8_000;
/// Double talk: the microphone carries this much more energy than the
/// predicted echo. Echo alone stays near 1 once the filter has converged; a
/// voice on top pushes it up. Judged afresh every block, so a loud syllable in
/// the answer never keeps the filter frozen.
const DOUBLE_TALK_RATIO: f32 = 2.0;
/// Samples into a block before the running ratio is trusted.
const DOUBLE_TALK_MIN_SAMPLES: usize = 32;
/// Updates use the leftover clipped to this many times its usual size, so the
/// first milliseconds of a guest's voice (far larger than the leftover) barely
/// move the filter before double talk is noticed.
const ROBUST_CLIP: f32 = 2.0;
/// How fast the usual leftover size follows: down quickly as the filter
/// converges, up slowly (about 0.3 s) so a sudden voice is not taken as normal.
const SCALE_FALL: f32 = 0.005;
const SCALE_RISE: f32 = 0.0002;
/// Smallest usual leftover size (about -80 dBFS), so updates never stop entirely.
const MIN_ERROR_SCALE: f32 = 1e-4;
/// Block energy that always counts as quiet (about -60 dBFS).
const QUIET_BLOCK_ENERGY: f32 = BLOCK as f32 * 1e-6;
/// Longest echo lag searched for, in samples (2048 = 128 ms).
const MAX_DELAY: usize = 2_048;
/// Samples of speech compared per delay search (0.25 s), and how often one runs.
const DELAY_WINDOW: usize = 4_000;
/// Mean speaker energy per sample below which a search window is skipped.
const DELAY_ACTIVE_ENERGY: f32 = 1e-5;
/// Normalized correlation the best lag needs before it is trusted.
const DELAY_CONFIDENCE: f32 = 0.3;
/// A new lag this close to the current one keeps what the filter learned.
const DELAY_TOLERANCE: usize = 16;
/// Filter taps kept before the measured lag, for a slightly early direct path.
const DELAY_MARGIN: usize = 32;

/// Removes a pod's own speaker signal from its microphone, one pod at a time.
pub struct EchoCanceller {
    taps: usize,
    weights: Vec<f32>,
    /// Speaker samples, newest first, stored twice so `history[pos..pos + taps]`
    /// is always one contiguous slice.
    history: Vec<f32>,
    pos: usize,
    adapt: bool,
    adapted_samples: usize,
    block_samples: usize,
    block_mic: f32,
    block_echo: f32,
    /// Usual size of the leftover (mean absolute value, full scale = 1.0).
    error_scale: f32,
    /// How many samples the reference is held back before the filter sees it.
    delay: usize,
    /// Raw reference, newest last: the delay line and the search's reference.
    reference_line: std::collections::VecDeque<f32>,
    /// Microphone samples for the next delay search, newest last.
    search_mic: std::collections::VecDeque<f32>,
    samples_since_search: usize,
}

impl EchoCanceller {
    pub fn new(taps: usize) -> Self {
        let taps = taps.max(1);
        Self {
            taps,
            weights: vec![0.0; taps],
            history: vec![0.0; 2 * taps],
            pos: 0,
            adapt: true,
            adapted_samples: 0,
            block_samples: 0,
            block_mic: 0.0,
            block_echo: 0.0,
            error_scale: 1.0,
            delay: 0,
            reference_line: std::collections::VecDeque::from(vec![0.0; MAX_DELAY + DELAY_WINDOW]),
            search_mic: std::collections::VecDeque::from(vec![0.0; DELAY_WINDOW]),
            samples_since_search: 0,
        }
    }

    /// Echo-cancelled microphone audio for one frame. `reference` is what the
    /// speaker played during the same samples; microphone samples without a
    /// matching reference sample pass through unchanged.
    pub fn process(&mut self, mic: &[i16], reference: &[i16]) -> Vec<i16> {
        let mut out = Vec::with_capacity(mic.len());
        for (index, &sample) in mic.iter().enumerate() {
            let Some(&speaker) = reference.get(index) else {
                out.push(sample);
                continue;
            };
            out.push(self.process_sample(sample, speaker));
        }
        out
    }

    fn process_sample(&mut self, mic: i16, speaker: i16) -> i16 {
        let heard = f32::from(mic) / 32_768.0;
        self.reference_line.pop_front();
        self.reference_line.push_back(f32::from(speaker) / 32_768.0);
        self.search_mic.pop_front();
        self.search_mic.push_back(heard);
        self.samples_since_search += 1;
        if self.samples_since_search >= DELAY_WINDOW {
            self.samples_since_search = 0;
            self.search_delay();
        }

        let taps = self.taps;
        self.pos = (self.pos + taps - 1) % taps;
        let newest = self.reference_line.len() - 1;
        let x = self.reference_line[newest - self.delay];
        self.history[self.pos] = x;
        self.history[self.pos + taps] = x;
        let window = &self.history[self.pos..self.pos + taps];

        let predicted: f32 = self.weights.iter().zip(window).map(|(w, x)| w * x).sum();
        let error = heard - predicted;

        let energy: f32 = window.iter().map(|x| x * x).sum();
        let clip = ROBUST_CLIP * self.error_scale;
        if energy > ACTIVE_REFERENCE_ENERGY {
            let magnitude = error.abs().min(clip);
            let rate = if magnitude < self.error_scale {
                SCALE_FALL
            } else {
                SCALE_RISE
            };
            self.error_scale =
                (self.error_scale + rate * (magnitude - self.error_scale)).max(MIN_ERROR_SCALE);
        }
        if self.adapt && energy > ACTIVE_REFERENCE_ENERGY {
            let step = MU * error.clamp(-clip, clip) / (energy + EPSILON);
            for (w, x) in self.weights.iter_mut().zip(window) {
                *w += step * x;
            }
            self.adapted_samples = self.adapted_samples.saturating_add(1);
        }

        self.block_mic += heard * heard;
        self.block_echo += predicted * predicted;
        self.block_samples += 1;
        // Stop learning as soon as the block looks like double talk: even a few
        // milliseconds of adapting on the guest's voice would bend the filter.
        if self.adapt && self.block_samples >= DOUBLE_TALK_MIN_SAMPLES && self.double_talk() {
            self.adapt = false;
        }
        if self.block_samples == BLOCK {
            self.adapt = !self.double_talk();
            self.block_samples = 0;
            self.block_mic = 0.0;
            self.block_echo = 0.0;
        }

        // Saturating conversion back to PCM16.
        (error * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16
    }

    /// Finds the lag at which the last window of microphone audio best matches
    /// the reference. A clearly different lag moves the delay line and starts
    /// learning again; the filter cannot model an echo outside its taps.
    fn search_delay(&mut self) {
        let reference = self.reference_line.make_contiguous();
        let mic = self.search_mic.make_contiguous();
        let recent = &reference[MAX_DELAY..];
        let reference_energy: f32 = recent.iter().map(|x| x * x).sum();
        if reference_energy < DELAY_ACTIVE_ENERGY * DELAY_WINDOW as f32 {
            return;
        }
        let mic_energy: f32 = mic.iter().map(|x| x * x).sum();
        let mut best = (0.0f32, 0usize);
        for lag in 0..MAX_DELAY {
            // mic[k] against the reference `lag` samples earlier.
            let lagged = &reference[MAX_DELAY - lag..MAX_DELAY - lag + DELAY_WINDOW];
            let dot: f32 = mic.iter().zip(lagged).map(|(m, r)| m * r).sum();
            if dot.abs() > best.0 {
                best = (dot.abs(), lag);
            }
        }
        let confidence = best.0 / (mic_energy * reference_energy).sqrt().max(EPSILON);
        if confidence < DELAY_CONFIDENCE {
            return;
        }
        let delay = best.1.saturating_sub(DELAY_MARGIN);
        if delay.abs_diff(self.delay) <= DELAY_TOLERANCE {
            return;
        }
        self.delay = delay;
        self.weights.iter_mut().for_each(|w| *w = 0.0);
        self.history.iter_mut().for_each(|x| *x = 0.0);
        self.adapt = true;
        self.adapted_samples = 0;
        self.error_scale = 1.0;
    }

    /// True when the block so far carries clearly more than the predicted echo.
    /// Never before the filter has warmed up: its estimate is still too poor.
    fn double_talk(&self) -> bool {
        self.adapted_samples >= WARM_UP_SAMPLES
            && self.block_mic > DOUBLE_TALK_RATIO * self.block_echo + QUIET_BLOCK_ENERGY
    }
}

#[cfg(test)]
mod tests {
    use super::EchoCanceller;

    const SECOND: usize = 16_000;

    /// Deterministic noise in [-1, 1).
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((self.0 >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
        }
    }

    /// Speaker signal: noise shaped a little like speech energy.
    fn reference(samples: usize) -> Vec<i16> {
        let mut noise = Noise(7);
        let mut low = 0.0f32;
        (0..samples)
            .map(|_| {
                low += 0.4 * (noise.next() - low);
                (low * 12_000.0) as i16
            })
            .collect()
    }

    /// The room: direct path after 30 samples, one reflection after 120.
    fn echo_of(reference: &[i16]) -> Vec<i16> {
        (0..reference.len())
            .map(|i| {
                let direct = if i >= 30 {
                    f32::from(reference[i - 30]) * 0.6
                } else {
                    0.0
                };
                let bounce = if i >= 120 {
                    f32::from(reference[i - 120]) * -0.2
                } else {
                    0.0
                };
                (direct + bounce) as i16
            })
            .collect()
    }

    /// The guest: a 300 Hz tone, uncorrelated with the speaker.
    fn guest(samples: usize) -> Vec<i16> {
        (0..samples)
            .map(|i| {
                ((i as f32 * 2.0 * std::f32::consts::PI * 300.0 / 16_000.0).sin() * 4_000.0) as i16
            })
            .collect()
    }

    fn rms(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
        (sum / samples.len() as f64).sqrt() as f32
    }

    fn difference(a: &[i16], b: &[i16]) -> Vec<i16> {
        a.iter()
            .zip(b)
            .map(|(&x, &y)| x.saturating_sub(y))
            .collect()
    }

    /// Runs the canceller over 40 ms frames, as pods send them.
    fn run(canceller: &mut EchoCanceller, mic: &[i16], reference: &[i16]) -> Vec<i16> {
        let mut out = Vec::with_capacity(mic.len());
        for (mic, reference) in mic.chunks(640).zip(reference.chunks(640)) {
            out.extend(canceller.process(mic, reference));
        }
        out
    }

    #[test]
    fn the_pods_own_voice_is_removed_from_the_microphone() {
        let reference = reference(2 * SECOND);
        let mic = echo_of(&reference);
        let mut canceller = EchoCanceller::new(256);
        let out = run(&mut canceller, &mic, &reference);
        let tail = SECOND + SECOND / 2..2 * SECOND;
        let suppression = 20.0 * (rms(&mic[tail.clone()]) / rms(&out[tail]).max(1.0)).log10();
        assert!(suppression >= 20.0, "echo only {suppression:.1} dB lower");
    }

    /// The room as measured on a pod: the echo lands 653 samples (a 40 ms
    /// frame plus 0.8 ms) after the reference.
    fn late_echo_of(reference: &[i16], delay: usize) -> Vec<i16> {
        (0..reference.len())
            .map(|i| {
                let direct = if i >= delay {
                    f32::from(reference[i - delay]) * 0.5
                } else {
                    0.0
                };
                let bounce = if i >= delay + 90 {
                    f32::from(reference[i - delay - 90]) * -0.15
                } else {
                    0.0
                };
                (direct + bounce) as i16
            })
            .collect()
    }

    #[test]
    fn an_echo_that_arrives_a_frame_late_is_still_removed() {
        let reference = reference(4 * SECOND);
        let mic = late_echo_of(&reference, 653);
        let mut canceller = EchoCanceller::new(256);
        let out = run(&mut canceller, &mic, &reference);
        let tail = 3 * SECOND..4 * SECOND;
        let suppression = 20.0 * (rms(&mic[tail.clone()]) / rms(&out[tail]).max(1.0)).log10();
        assert!(
            suppression >= 20.0,
            "late echo only {suppression:.1} dB lower"
        );
    }

    #[test]
    fn a_guest_talking_over_the_answer_comes_through() {
        // Double talk lasts only until the bridge notices the guest and stops
        // the answer (about 120 ms), so the voice must stand out from the start.
        let reference = reference(3 * SECOND);
        let echo = echo_of(&reference);
        let voice = guest(3 * SECOND);
        let talk_from = SECOND + SECOND / 2;
        let mic: Vec<i16> = (0..3 * SECOND)
            .map(|i| {
                let guest = if i >= talk_from { voice[i] } else { 0 };
                echo[i].saturating_add(guest)
            })
            .collect();
        let mut canceller = EchoCanceller::new(256);
        let out = run(&mut canceller, &mic, &reference);
        let onset = talk_from..talk_from + SECOND / 5;
        let residue = rms(&difference(&out[onset.clone()], &voice[onset.clone()]));
        let kept = rms(&out[onset.clone()]) / rms(&voice[onset.clone()]);
        assert!(
            residue <= rms(&voice[onset]) / 3.0,
            "left {residue:.0} of echo and distortion over the guest"
        );
        assert!(
            (0.7..=1.4).contains(&kept),
            "guest level scaled by {kept:.2}"
        );
    }

    #[test]
    fn talking_over_the_answer_does_not_undo_what_was_learned() {
        let reference = reference(4 * SECOND);
        let echo = echo_of(&reference);
        let voice = guest(4 * SECOND);
        // Echo, then a second of double talk, then echo alone again.
        let mic: Vec<i16> = (0..4 * SECOND)
            .map(|i| {
                let guest = if (SECOND + SECOND / 2..2 * SECOND + SECOND / 2).contains(&i) {
                    voice[i]
                } else {
                    0
                };
                echo[i].saturating_add(guest)
            })
            .collect();
        let mut canceller = EchoCanceller::new(256);
        let out = run(&mut canceller, &mic, &reference);
        let after = 3 * SECOND..4 * SECOND;
        let suppression = 20.0 * (rms(&mic[after.clone()]) / rms(&out[after]).max(1.0)).log10();
        assert!(
            suppression >= 15.0,
            "echo only {suppression:.1} dB lower after double talk"
        );
    }

    #[test]
    fn a_silent_speaker_leaves_the_microphone_untouched() {
        let voice = guest(SECOND);
        let silence = vec![0i16; SECOND];
        let mut canceller = EchoCanceller::new(256);
        assert_eq!(run(&mut canceller, &voice, &silence), voice);
    }
}
