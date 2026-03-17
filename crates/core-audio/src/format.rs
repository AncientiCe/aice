//! Audio format constants.

use super::{CHANNELS, SAMPLE_RATE};

/// Format description for pipeline use.
pub fn description() -> (u32, u16) {
    (SAMPLE_RATE, CHANNELS)
}
