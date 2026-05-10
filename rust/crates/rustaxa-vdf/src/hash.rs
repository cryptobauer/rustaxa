use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// Simple cache for precision bounds to avoid recomputing for common lambda values
static PRECISION_CACHE: OnceLock<Mutex<HashMap<u32, rug::Integer>>> = OnceLock::new();

pub struct HashToPrime {
    max_int: rug::Integer,
}

impl HashToPrime {
    pub fn new(lambda: u32) -> Self {
        let max_int = {
            // Check cache first for common lambda values
            let cache = PRECISION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
            if let Ok(cache_guard) = cache.lock() {
                if let Some(cached_value) = cache_guard.get(&lambda) {
                    cached_value.clone()
                } else {
                    drop(cache_guard); // Release lock before computation
                    let computed = compute_precision_bound(lambda);

                    // Try to cache the result (ignore errors if cache is busy)
                    if let Ok(mut cache_guard) = cache.try_lock() {
                        cache_guard.insert(lambda, computed.clone());
                    }
                    computed
                }
            } else {
                // Fallback if cache is unavailable
                compute_precision_bound(lambda)
            }
        };

        // Divide max_int by 6 to take only values 6k±1 as candidates.
        // C++ uses ceiling division here via `mpz_cdiv_q`; keep that exact
        // range because the RNG draws are consensus-visible for VDF proofs.
        let mut optimized_max_int = max_int;
        optimized_max_int += 5;
        optimized_max_int /= 6;
        if optimized_max_int < 2 {
            optimized_max_int = rug::Integer::from(2);
        }

        HashToPrime {
            max_int: optimized_max_int,
        }
    }

    /// Get the maximum integer bound used for prime generation
    /// This is primarily for testing purposes
    pub fn max_int(&self) -> &rug::Integer {
        &self.max_int
    }

    pub fn hash_to_prime(&self, input: &rug::Integer) -> Result<rug::Integer, String> {
        use rug::integer::IsPrime;
        const MAX_ITER: u32 = 10000;
        const MAX_MILLER_RABIN: u32 = 30;

        // Create a fresh Mersenne Twister state for this call. C++ seeds GMP's
        // `gmp_randinit_mt` with the full big-endian BIGNUM value, so do not
        // truncate or otherwise hash the seed here.
        let mut prime_gen = rug::rand::RandState::new();
        prime_gen.seed(input);

        let mut is_prime = IsPrime::No;
        let mut count = 0u32;
        let mut candidate = rug::Integer::new();

        // Pre-allocate constants to avoid repeated allocations in the loop
        let six = rug::Integer::from(6);
        let one = rug::Integer::from(1);
        let two = rug::Integer::from(2);

        while is_prime == IsPrime::No && count < MAX_ITER {
            // Generate random candidate in range [0, max_int).
            candidate = self.max_int.clone().random_below(&mut prime_gen);

            // Generate random sign bit (0 or 1) and convert to -1 or +1
            let sign_bit = two.clone().random_below(&mut prime_gen);
            let sign = rug::Integer::from(&two * &sign_bit - &one); // Convert 0,1 to -1,+1

            // Apply transformation: candidate = 6 * candidate + sign
            // Use more efficient in-place operations
            candidate *= &six;
            candidate += &sign;

            // Ensure candidate is at least 2
            if candidate < 2 {
                count += 1;
                continue;
            }

            // Test for primality using Miller-Rabin
            is_prime = candidate.is_probably_prime(MAX_MILLER_RABIN);
            count += 1;
        }

        if count == MAX_ITER {
            return Err(format!(
                "Prime not found within {} iterations for practical_bits={}",
                MAX_ITER,
                self.max_int.significant_bits()
            ));
        }

        Ok(candidate)
    }
}

/// Compute precision bound using Lambert W function approximation.
/// This is an optimized version that reduces memory allocations and improves numerical efficiency.
fn compute_precision_bound(lambda: u32) -> rug::Integer {
    use rug::Assign;
    use rug::Float;
    use rug::float::Round;
    use rug::ops::Pow;

    let precision = 4096;
    const MAX_ITER: i32 = 32;

    let mut x = Float::with_val(precision, 2).pow(-(lambda as i32));
    let tmp_x = x.clone();
    x = -x; // x = -2^(-lambda)

    // Calculate initial approximation more efficiently
    let l1 = tmp_x.ln(); // log(2^(-lambda)) = -lambda * ln(2)
    let neg_l1 = Float::with_val(precision, -&l1);
    let l2 = neg_l1.ln();

    // Initial guess: w = L1 - L2 + L2/L1 (reuse variables)
    let mut w = Float::with_val(precision, &l2);
    w /= &l1; // w = L2/L1
    w += &l1; // w = L1 + L2/L1
    w -= &l2; // w = L1 - L2 + L2/L1

    let eps_exp = std::cmp::min(200i32, precision as i32 / 3);
    let eps = Float::with_val(precision, 2).pow(-eps_exp);

    // Pre-allocate working variables for the iteration to reduce allocations
    let mut e = Float::with_val(precision, 0);
    let mut t = Float::with_val(precision, 0);
    let mut p = Float::with_val(precision, 0);
    let mut ep = Float::with_val(precision, 0);

    // Newton-Raphson / Halley iteration with optimized operations
    for _iter in 0..MAX_ITER {
        // e = exp(w)
        e.assign(&w);
        e = e.exp();

        // t = e*w - x
        t.assign(&e);
        t *= &w;
        t -= &x;

        // p = w + 1
        p.assign(&w);
        p += 1;

        if w.is_sign_positive() {
            // Newton iteration: t = t/(e*p)
            ep.assign(&e);
            ep *= &p;
            t /= &ep;
        } else {
            // Halley iteration for better convergence when w < 0
            // denominator = e*p - 0.5*(p + 1)*t/p
            ep.assign(&e);
            ep *= &p; // e*p

            let mut numerator = Float::with_val(precision, &p + 1); // p + 1
            numerator *= &t; // (p + 1)*t
            numerator /= &p; // (p + 1)*t/p
            numerator *= 0.5; // 0.5*(p + 1)*t/p

            ep -= &numerator; // e*p - 0.5*(p + 1)*t/p
            t /= &ep;
        }

        w -= &t;

        let mut tolerance_denominator = Float::with_val(precision, &p);
        tolerance_denominator.abs_mut();
        tolerance_denominator *= &e;
        let mut inverse_tolerance = Float::with_val(precision, 1);
        inverse_tolerance /= &tolerance_denominator;
        let w_abs = w.clone().abs();
        let tolerance_max = if inverse_tolerance > w_abs {
            inverse_tolerance
        } else {
            w_abs
        };
        let tolerance = Float::with_val(precision, &eps * tolerance_max * 10);
        let abs_t = t.clone().abs();

        if tolerance > abs_t {
            break;
        }
    }

    w = -w;
    w = w.exp();
    w.ceil_mut();
    w.to_integer_round(Round::Nearest)
        .map(|(value, _)| value)
        .unwrap_or_else(|| rug::Integer::from(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rug::ops::Pow;
    #[test]
    fn test_compute_precision_bound_basic() {
        // Test with a reasonable lambda value
        let result = compute_precision_bound(128);

        // The result should be a positive integer (at least our fallback values)
        assert!(result > 0);

        assert!(result.significant_bits() >= 32);
    }

    #[test]
    fn test_compute_precision_bound_small_lambda() {
        // Test with a small lambda value
        let result = compute_precision_bound(32);

        assert!(result > 0);
        assert!(result.significant_bits() >= 32);
    }

    #[test]
    fn test_compute_precision_bound_large_lambda() {
        // Test with a larger lambda value
        let result = compute_precision_bound(256);

        assert!(result > 0);
        assert!(result.significant_bits() >= 256);
    }

    #[test]
    fn test_compute_precision_bound_deterministic() {
        // Test that the function is deterministic - same input gives same output
        let result1 = compute_precision_bound(128);
        let result2 = compute_precision_bound(128);

        assert_eq!(result1, result2);
    }
    #[test]
    fn test_compute_precision_bound_monotonic() {
        // Test that the function produces consistent results
        let result_small = compute_precision_bound(64);
        let result_large = compute_precision_bound(256);

        // Both should be positive
        assert!(result_small > 0);
        assert!(result_large > 0);

        assert!(result_large > result_small);
    }

    #[test]
    fn test_compute_precision_bound_zero_lambda() {
        // Test edge case with lambda=0
        let result = compute_precision_bound(0);

        assert!(result > 0);
    }
    #[test]
    fn test_compute_precision_bound_edge_cases() {
        // Test with very small lambda
        let result_small = compute_precision_bound(1);
        assert!(result_small > 0);

        // Test with large lambda value (1500 is a realistic value)
        let result_large = compute_precision_bound(1500);
        assert!(result_large > 0);
        assert!(result_large > result_small);
    }

    #[test]
    fn test_compute_precision_bound_lambda_1500() {
        // Test specifically with lambda=1500 (expected real-world value)
        let result = compute_precision_bound(1500);

        assert!(result > 0);

        // Should be deterministic
        let result2 = compute_precision_bound(1500);
        assert_eq!(result, result2);

        assert!(result.significant_bits() >= 1500);
    }
    #[test]
    fn test_precision_bound_is_legacy_lambert_bound() {
        let result = compute_precision_bound(128);

        assert!(result > 0);
        assert!(result.significant_bits() >= 128);
        assert_ne!(result.count_ones(), Some(1));
    }
    #[test]
    fn test_precision_bound_reasonable_range() {
        // Test that the precision bounds are in a reasonable range for cryptographic use
        let lambdas = [64, 128, 192, 256];

        for lambda in lambdas.iter() {
            let result = compute_precision_bound(*lambda);

            assert!(
                result.significant_bits() < 10000,
                "Precision bound too large for lambda={}",
                lambda
            );
        }
    }

    #[test]
    fn test_precision_bound_performance() {
        use std::time::Instant;

        // Test multiple lambda values to measure average performance
        let test_lambdas = [32, 64, 128, 192, 256, 320, 384, 512];
        let iterations_per_lambda = 10;

        let start_time = Instant::now();
        let mut total_computations = 0;

        for &lambda in &test_lambdas {
            for _ in 0..iterations_per_lambda {
                let result = compute_precision_bound(lambda);

                assert!(result > 0);
                total_computations += 1;
            }
        }

        let elapsed = start_time.elapsed();
        let avg_per_computation = elapsed / total_computations;

        println!("Performance Results:");
        println!("- Total computations: {}", total_computations);
        println!("- Total time: {:?}", elapsed);
        println!("- Average per computation: {:?}", avg_per_computation);

        // Assert that each computation takes less than 10ms on average (very generous)
        // This is a reasonable performance expectation for cryptographic applications
        assert!(
            avg_per_computation.as_millis() < 10,
            "Performance regression: average time per computation is {:?}",
            avg_per_computation
        );
    }
    #[test]
    fn test_hash_to_prime_basic() {
        let h2p = HashToPrime::new(128);
        let input = rug::Integer::from(42u32);

        let result = h2p.hash_to_prime(&input);

        if let Err(ref e) = result {
            println!("Error: {}", e);
        }

        assert!(result.is_ok());

        let prime = result.unwrap();

        // Verify it's actually a probable prime
        use rug::integer::IsPrime;
        assert_ne!(prime.is_probably_prime(30), IsPrime::No);

        // Verify it's greater than 1
        assert!(prime > 1);
    }

    #[test]
    fn test_hash_to_prime_deterministic() {
        let h2p1 = HashToPrime::new(128);
        let h2p2 = HashToPrime::new(128);
        let input = rug::Integer::from(1337u32);

        let result1 = h2p1.hash_to_prime(&input).unwrap();
        let result2 = h2p2.hash_to_prime(&input).unwrap();

        // Same input should produce same prime (deterministic)
        assert_eq!(result1, result2);
    }

    #[test]
    fn test_hash_to_prime_6k_plus_minus_1_form() {
        let h2p = HashToPrime::new(128);
        let input = rug::Integer::from(12345u32);

        let prime = h2p.hash_to_prime(&input).unwrap();

        // The algorithm generates primes of the form 6k±1
        // So prime % 6 should be either 1 or 5 (equivalent to -1 mod 6)
        let remainder = rug::Integer::from(&prime % 6);
        let rem_val = remainder.to_u32().unwrap();
        assert!(
            rem_val == 1 || rem_val == 5,
            "Prime {} should be of form 6k±1, but {} % 6 = {}",
            prime,
            prime,
            rem_val
        );
    }

    #[test]
    fn test_integer_bytes_conversion() {
        // Test integer to bytes conversion (used internally for seeding)
        let small_int = rug::Integer::from(255u32);
        let bit_size = small_int.significant_bits();
        let byte_size = bit_size.div_ceil(8);
        let mut cache = vec![0u8; byte_size as usize];
        small_int.write_digits(&mut cache, rug::integer::Order::MsfBe);

        // Verify we can reconstruct the integer
        let reconstructed = rug::Integer::from_digits(&cache, rug::integer::Order::MsfBe);
        assert_eq!(small_int, reconstructed);

        // Test with larger integer
        let large_int = rug::Integer::from(2u32).pow(100);
        let bit_size_large = large_int.significant_bits();
        let byte_size_large = bit_size_large.div_ceil(8);
        let mut cache_large = vec![0u8; byte_size_large as usize];
        large_int.write_digits(&mut cache_large, rug::integer::Order::MsfBe);
        let reconstructed_large =
            rug::Integer::from_digits(&cache_large, rug::integer::Order::MsfBe);
        assert_eq!(large_int, reconstructed_large);
    }

    #[test]
    fn test_hashtoprime_with_lambda_1500() {
        // Test HashToPrime creation and usage with lambda=1500
        let h2p = HashToPrime::new(1500);
        let input = rug::Integer::from(12345u32);

        let result = h2p.hash_to_prime(&input);
        assert!(result.is_ok());

        let prime = result.unwrap();

        // Verify it's actually a probable prime
        use rug::integer::IsPrime;
        assert_ne!(prime.is_probably_prime(30), IsPrime::No);

        // Verify it's greater than 1
        assert!(prime > 1);

        // Verify it's of the form 6k±1
        let remainder = rug::Integer::from(&prime % 6);
        let rem_val = remainder.to_u32().unwrap();
        assert!(rem_val == 1 || rem_val == 5);
    }

    #[test]
    fn test_caching_performance() {
        use std::time::Instant;

        // Test that caching improves performance for repeated lambda values
        let lambda = 256u32;

        // First call (should cache the result)
        let start1 = Instant::now();
        let h2p1 = HashToPrime::new(lambda);
        let duration1 = start1.elapsed();

        // Second call (should use cached result)
        let start2 = Instant::now();
        let h2p2 = HashToPrime::new(lambda);
        let duration2 = start2.elapsed();

        // Third call (should also use cached result)
        let start3 = Instant::now();
        let h2p3 = HashToPrime::new(lambda);
        let duration3 = start3.elapsed();

        println!("Cache performance test:");
        println!("- First call (cold): {:?}", duration1);
        println!("- Second call (cached): {:?}", duration2);
        println!("- Third call (cached): {:?}", duration3);

        // Verify all instances produce the same max_int (deterministic)
        assert_eq!(h2p1.max_int, h2p2.max_int);
        assert_eq!(h2p2.max_int, h2p3.max_int);

        // The cached calls should generally be faster, but we won't enforce it strictly
        // as system load can affect timing. Just verify they complete successfully.
        assert!(duration2 < duration1 * 2); // Very generous bound
        assert!(duration3 < duration1 * 2); // Very generous bound
    }
}
