use tex_mask::Random;

/// The system's random source, without modulo bias: draws past the largest multiple of `n` are
/// drawn again.
pub struct OsRandom;

impl Random for OsRandom {
    fn below(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return None;
        }
        let n = u64::from(n);
        let zone = (1u64 << 32) / n * n;
        loop {
            let draw = u64::from(getrandom::u32().ok()?);
            if draw < zone {
                return u32::try_from(draw % n).ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_stay_in_range_and_degenerate_ranges_behave() {
        let mut rng = OsRandom;
        for _ in 0..1000 {
            assert!(rng.below(7).unwrap() < 7);
        }
        assert_eq!(OsRandom.below(1), Some(0));
        assert_eq!(OsRandom.below(0), None);
    }
}
