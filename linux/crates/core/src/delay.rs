//! Inter-key delay model (original uniform-jitter version).

use rand::RngExt;

/// Draws a delay uniformly from `base ± base * jitter%`, rounded to whole ms.
///
/// `base_delay_ms` is raised to at least 1 and `jitter_percent` is clamped to
/// `0..=100`, matching the original `DelayModel.NextDelayMs`.
pub fn next_delay_ms<R: rand::Rng + ?Sized>(rng: &mut R, base_delay_ms: u32, jitter_percent: u32) -> u32 {
    let base = f64::from(base_delay_ms.max(1));
    let span = base * f64::from(jitter_percent.min(100)) / 100.0;
    let (min, max) = (base - span, base + span);
    let value = min + rng.random::<f64>() * (max - min);
    // With a huge base and 100% jitter the upper bound exceeds u32::MAX; saturate.
    value.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn stays_in_configured_range() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..500 {
            let value = next_delay_ms(&mut rng, 100, 20);
            assert!((80..=120).contains(&value), "{value} out of range");
        }
    }

    #[test]
    fn zero_jitter_is_exact_and_zero_base_is_raised() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        assert_eq!(next_delay_ms(&mut rng, 70, 0), 70);
        assert_eq!(next_delay_ms(&mut rng, 0, 0), 1);
    }

    #[test]
    fn jitter_above_100_is_clamped() {
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        for _ in 0..200 {
            assert!(next_delay_ms(&mut rng, 50, 500) <= 100);
        }
    }
}
