//! Typing parameters. Field set and defaults mirror the original Windows build.

/// Parameters that shape one typing session.
///
/// Percent fields are clamped to `0..=100` where they are used; the planner
/// never panics on out-of-range values.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypingSettings {
    /// Mean pause between two typed characters, in milliseconds.
    pub base_delay_ms: u32,
    /// Spread around `base_delay_ms`, as a percentage of it.
    pub jitter_percent: u32,
    /// Chance per character of hitting a neighbouring key instead.
    pub typo_rate_percent: u32,
    /// Chance per character of skipping it.
    pub omission_rate_percent: u32,
    /// Chance per character of swapping it with the next one.
    pub transpose_rate_percent: u32,
    /// Chance that a mistake gets noticed and fixed.
    pub repair_rate_percent: u32,
    /// How long it takes to notice a mistake before backspacing.
    pub error_detect_delay_ms: u32,
    /// Pause after each backspace while fixing a mistake.
    pub backspace_delay_ms: u32,
    /// Countdown between pressing start and the first key, so the user can
    /// focus the target window.
    pub lead_in_delay_ms: u32,
}

impl Default for TypingSettings {
    fn default() -> Self {
        Self {
            base_delay_ms: 90,
            jitter_percent: 20,
            typo_rate_percent: 8,
            omission_rate_percent: 5,
            transpose_rate_percent: 4,
            repair_rate_percent: 85,
            error_detect_delay_ms: 900,
            backspace_delay_ms: 70,
            lead_in_delay_ms: 2500,
        }
    }
}
