use crate::vdf::{Solution, WesolowskiVdf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct WesolowskiProver<'a> {
    vdf: &'a WesolowskiVdf,
}

impl<'a> WesolowskiProver<'a> {
    pub fn new(vdf: &'a WesolowskiVdf) -> Self {
        WesolowskiProver { vdf }
    }

    /// Helper method for efficient modular squaring
    #[inline]
    fn square_mod(value: &mut rug::Integer, modulus: &rug::Integer) {
        value.square_mut();
        *value %= modulus;
    }

    // Wesolowski prover
    pub fn prove(&self, cancelled: &CancellationToken) -> Solution {
        // Get puzzle parameters
        let modulus = self.vdf.modulus(); // N
        let base = self.vdf.base(); // x
        let iterations = self.vdf.iterations(); // T

        // Convert iterations to u64 for efficient counting
        // For VDF, T = 2^time_bits, so this should fit in u64 for reasonable time_bits (up to 63)
        let iterations_u64 = match iterations.to_u64() {
            Some(val) => val,
            None => {
                // If iterations is too large for u64, fall back to BigInt iteration
                return self.prove_large_iterations(cancelled);
            }
        };

        // Compute y = x^(2^T) mod N by repeatedly squaring
        let mut y = base.clone();

        // Optimize cancellation check frequency based on iteration count
        let check_interval = (iterations_u64 / 100).clamp(1, 10000);

        for i in 1..=iterations_u64 {
            // Check cancellation at optimal intervals
            if i % check_interval == 0 && cancelled.is_cancelled() {
                return Solution {
                    first: vec![],
                    second: vec![],
                };
            }

            // y = y^2 mod N - use helper method for consistency
            Self::square_mod(&mut y, modulus);
        }

        // Prepare xy for hashing: xy = x || y (concatenate x and y)
        // Use the same method as verifier: x * 2^(modulus_bits) + y
        let modulus_bits = modulus.significant_bits();
        let mut xy = base.clone();
        xy <<= modulus_bits; // Left shift x by number of bits in N
        xy += &y; // Add y to get x||y

        // Hash xy to get prime p
        let p = match self.vdf.hash_to_prime(&xy) {
            Ok(prime) => prime,
            Err(_) => {
                // Return empty solution if hashing failed
                return Solution {
                    first: vec![],
                    second: vec![],
                };
            }
        };

        // Compute pi using the binary representation approach
        let mut r = rug::Integer::from(1);
        let mut pi = rug::Integer::from(1);

        for i in 1..=iterations_u64 {
            // Check cancellation at optimal intervals
            if i % check_interval == 0 && cancelled.is_cancelled() {
                return Solution {
                    first: vec![],
                    second: vec![],
                };
            }

            // pi = pi^2 mod N - use helper method for consistency
            Self::square_mod(&mut pi, modulus);

            // r = r * 2 = left shift by 1
            r <<= 1;

            // If r >= p, then r = r - p and pi = pi * x mod N
            if r >= p {
                r -= &p;
                pi *= base;
                pi %= modulus;
            }
        }

        // Convert pi and y to byte vectors
        let pi_bytes = pi.to_digits::<u8>(rug::integer::Order::MsfBe);
        let y_bytes = y.to_digits::<u8>(rug::integer::Order::MsfBe);

        // Return solution as (pi, y)
        Solution {
            first: pi_bytes,
            second: y_bytes,
        }
    }

    // Fallback method for very large iteration counts that don't fit in u64
    fn prove_large_iterations(&self, cancelled: &CancellationToken) -> Solution {
        // Get puzzle parameters
        let modulus = self.vdf.modulus(); // N
        let base = self.vdf.base(); // x
        let iterations = self.vdf.iterations(); // T

        // Compute y = x^(2^T) mod N by repeatedly squaring
        let mut y = base.clone();
        let mut current_iteration = rug::Integer::from(1);

        // For very large iterations, check cancellation less frequently
        let mut check_counter = 0u64;
        const LARGE_CHECK_INTERVAL: u64 = 100000;

        while &current_iteration <= iterations {
            // Check cancellation periodically
            check_counter += 1;
            if check_counter >= LARGE_CHECK_INTERVAL {
                check_counter = 0;
                if cancelled.is_cancelled() {
                    return Solution {
                        first: vec![],
                        second: vec![],
                    };
                }
            }

            // y = y^2 mod N
            Self::square_mod(&mut y, modulus);
            current_iteration += 1;
        }

        // Prepare xy for hashing: xy = x || y (concatenate x and y)
        let modulus_bits = modulus.significant_bits();
        let mut xy = base.clone();
        xy <<= modulus_bits;
        xy += &y;

        // Hash xy to get prime p
        let p = match self.vdf.hash_to_prime(&xy) {
            Ok(prime) => prime,
            Err(_) => {
                return Solution {
                    first: vec![],
                    second: vec![],
                };
            }
        };

        // Compute pi using the binary representation approach
        let mut r = rug::Integer::from(1);
        let mut pi = rug::Integer::from(1);
        let mut current_iteration = rug::Integer::from(1);
        check_counter = 0;

        while &current_iteration <= iterations {
            // Check cancellation periodically
            check_counter += 1;
            if check_counter >= LARGE_CHECK_INTERVAL {
                check_counter = 0;
                if cancelled.is_cancelled() {
                    return Solution {
                        first: vec![],
                        second: vec![],
                    };
                }
            }

            // pi = pi^2 mod N
            Self::square_mod(&mut pi, modulus);

            // r = r * 2 = left shift by 1
            r <<= 1;

            // If r >= p, then r = r - p and pi = pi * x mod N
            if r >= p {
                r -= &p;
                pi *= base;
                pi %= modulus;
            }

            current_iteration += 1;
        }

        // Convert pi and y to byte vectors
        let pi_bytes = pi.to_digits::<u8>(rug::integer::Order::MsfBe);
        let y_bytes = y.to_digits::<u8>(rug::integer::Order::MsfBe);

        Solution {
            first: pi_bytes,
            second: y_bytes,
        }
    }
}

pub struct CancellationToken {
    flag: Arc<AtomicBool>,
    external_ptr: Option<*const bool>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        CancellationToken {
            flag: Arc::new(AtomicBool::new(false)),
            external_ptr: None,
        }
    }

    pub fn from_atomic_ptr(atomic_ptr: *const bool) -> Self {
        CancellationToken {
            flag: Arc::new(AtomicBool::new(false)), // Unused in this case
            external_ptr: Some(atomic_ptr),
        }
    }

    /// Signals cancellation to any listening operations
    pub fn cancel(&self) {
        if let Some(ptr) = self.external_ptr {
            unsafe {
                *(ptr as *mut AtomicBool) = AtomicBool::new(true);
            }
        } else {
            self.flag.store(true, Ordering::Release);
        }
    }

    /// Checks if cancellation has been requested
    pub fn is_cancelled(&self) -> bool {
        if let Some(ptr) = self.external_ptr {
            unsafe { (*(ptr as *const AtomicBool)).load(Ordering::Acquire) }
        } else {
            self.flag.load(Ordering::Acquire)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::prover::WesolowskiProver;

    use super::*;
    #[test]
    fn test_prove_with_cancellation() {
        // Test that prove function respects cancellation
        let lambda = 128u32;
        let time_bits = 10u32; // Larger number to give time for cancellation
        let modulus = vec![0x01, 0x01]; // 257 in decimal
        let input = vec![0x02]; // 2 in decimal

        let stop_flag = CancellationToken::new();

        // Set the stop flag before starting
        stop_flag.cancel();

        // Generate proof with cancellation
        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let prover = WesolowskiProver::new(&vdf);
        let solution = prover.prove(&stop_flag);

        // Solution should be empty due to cancellation
        assert!(solution.first.is_empty());
        assert!(solution.second.is_empty());
    }

    #[test]
    fn test_prove_deterministic() {
        // Test that prove function is deterministic for same inputs
        let lambda = 128u32;
        let time_bits = 3u32; // Small for faster testing
        let modulus = vec![0x01, 0x01];
        let input = vec![0x02];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let stop_flag1 = CancellationToken::new();
        let stop_flag2 = CancellationToken::new();

        let prover1 = WesolowskiProver::new(&vdf);
        let prover2 = WesolowskiProver::new(&vdf);
        let solution1 = prover1.prove(&stop_flag1);
        let solution2 = prover2.prove(&stop_flag2);

        // Solutions should be identical for same inputs
        assert_eq!(
            solution1.first, solution2.first,
            "Prove should be deterministic for proof element"
        );
        assert_eq!(
            solution1.second, solution2.second,
            "Prove should be deterministic for solution element"
        );
    }

    #[test]
    fn test_solution_components_non_trivial() {
        // Test that solutions are non-trivial (not just 1 or 0)
        let lambda = 128u32;
        let time_bits = 5u32;
        let modulus = vec![0x01, 0x00, 0x01]; // 65537
        let input = vec![0x03];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let stop_flag = CancellationToken::new();

        let prover = WesolowskiProver::new(&vdf);
        let solution = prover.prove(&stop_flag);

        // Convert back to integers to check values
        let pi = rug::Integer::from_digits(&solution.first, rug::integer::Order::MsfBe);
        let y = rug::Integer::from_digits(&solution.second, rug::integer::Order::MsfBe);

        // Should not be trivial values
        assert!(!pi.is_zero(), "Proof element should not be zero");
        assert!(!y.is_zero(), "Solution element should not be zero");
        assert!(pi != 1, "Proof element should not be 1");
        // Note: y could theoretically be 1 in some cases, so we don't test for that
    }

    #[test]
    fn test_optimized_cancellation_intervals() {
        // Test that the optimized cancellation check intervals work correctly
        let lambda = 128u32;
        let time_bits = 8u32; // 256 iterations
        let modulus = vec![0x01, 0x01]; // 257
        let input = vec![0x02];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let stop_flag = CancellationToken::new();

        let prover = WesolowskiProver::new(&vdf);

        // Measure time to ensure optimization doesn't break functionality
        let start = std::time::Instant::now();
        let solution = prover.prove(&stop_flag);
        let duration = start.elapsed();

        // Should complete reasonably fast for small iterations
        assert!(
            duration.as_millis() < 1000,
            "Should complete in under 1 second for small test"
        );
        assert!(!solution.first.is_empty(), "Should produce valid solution");
        assert!(!solution.second.is_empty(), "Should produce valid solution");
    }
}
