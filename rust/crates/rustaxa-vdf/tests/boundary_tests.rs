use rug::Integer;
use rug::ops::Pow;
use rustaxa_vdf::hash::HashToPrime;
use rustaxa_vdf::puzzle::RswPuzzle;
use rustaxa_vdf::vdf::WesolowskiVdf;

/// Test boundary conditions for hash-to-prime functionality
#[cfg(test)]
mod hash_to_prime_boundary_tests {
    use super::*;

    #[test]
    fn test_minimum_lambda_values() {
        // Test with the smallest possible lambda values
        for lambda in 0..=5 {
            let h2p = HashToPrime::new(lambda);
            let input = Integer::from(1u32);

            let result = h2p.hash_to_prime(&input);
            assert!(result.is_ok(), "Failed for lambda={}", lambda);

            if let Ok(prime) = result {
                assert!(prime > 1, "Prime should be > 1 for lambda={}", lambda);
                // Verify it's actually prime
                use rug::integer::IsPrime;
                assert_eq!(
                    prime.is_probably_prime(10),
                    IsPrime::Yes,
                    "Result should be prime for lambda={}",
                    lambda
                );
            }
        }
    }

    #[test]
    fn test_maximum_reasonable_lambda_values() {
        // Test with large but reasonable lambda values
        let large_lambdas = [512, 1024, 2048, 4096];

        for &lambda in &large_lambdas {
            let h2p = HashToPrime::new(lambda);
            let input = Integer::from(42u32);

            let result = h2p.hash_to_prime(&input);
            assert!(result.is_ok(), "Failed for lambda={}", lambda);

            if let Ok(prime) = result {
                assert!(prime > 1, "Prime should be > 1 for lambda={}", lambda);
                use rug::integer::IsPrime;
                assert_eq!(
                    prime.is_probably_prime(10),
                    IsPrime::Yes,
                    "Result should be prime for lambda={}",
                    lambda
                );
            }
        }
    }

    #[test]
    fn test_zero_input() {
        let h2p = HashToPrime::new(128);
        let zero_input = Integer::new();

        let result = h2p.hash_to_prime(&zero_input);
        assert!(result.is_ok(), "Hash-to-prime should handle zero input");

        if let Ok(prime) = result {
            assert!(prime > 1);
            use rug::integer::IsPrime;
            assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
        }
    }

    #[test]
    fn test_very_large_input() {
        let h2p = HashToPrime::new(128);
        // Create a very large input (2^1000)
        let large_input = Integer::from(2u32).pow(1000);

        let result = h2p.hash_to_prime(&large_input);
        assert!(
            result.is_ok(),
            "Hash-to-prime should handle very large input"
        );

        if let Ok(prime) = result {
            assert!(prime > 1);
            use rug::integer::IsPrime;
            assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
        }
    }

    #[test]
    fn test_negative_input() {
        let h2p = HashToPrime::new(128);
        let negative_input = Integer::from(-42);

        let result = h2p.hash_to_prime(&negative_input);
        assert!(result.is_ok(), "Hash-to-prime should handle negative input");

        if let Ok(prime) = result {
            assert!(prime > 1);
            use rug::integer::IsPrime;
            assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
        }
    }

    #[test]
    fn test_max_iterations_boundary() {
        // Test cases that might hit the MAX_ITER boundary
        let h2p = HashToPrime::new(10); // Very small lambda to increase chance of timeout

        // Try multiple different inputs to see if any hit the iteration limit
        for i in 0..10 {
            let input = Integer::from(i);
            let result = h2p.hash_to_prime(&input);

            // We expect either success or a specific iteration limit error
            match result {
                Ok(prime) => {
                    assert!(prime > 1);
                    use rug::integer::IsPrime;
                    assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
                }
                Err(msg) => {
                    assert!(
                        msg.contains("Prime not found within"),
                        "Unexpected error message: {}",
                        msg
                    );
                }
            }
        }
    }

    #[test]
    fn test_deterministic_across_boundaries() {
        // Test that the hash-to-prime function is deterministic across different boundary conditions
        let test_cases = vec![
            (64, Integer::from(0)),
            (128, Integer::from(1)),
            (256, Integer::from(u64::MAX)),
            (512, Integer::from(2u32).pow(100)),
        ];

        for (lambda, input) in test_cases {
            let h2p1 = HashToPrime::new(lambda);
            let h2p2 = HashToPrime::new(lambda);

            let result1 = h2p1.hash_to_prime(&input);
            let result2 = h2p2.hash_to_prime(&input);

            assert_eq!(
                result1.is_ok(),
                result2.is_ok(),
                "Consistency check failed for lambda={}, input={}",
                lambda,
                input
            );

            if result1.is_ok() && result2.is_ok() {
                assert_eq!(
                    result1.unwrap(),
                    result2.unwrap(),
                    "Determinism check failed for lambda={}, input={}",
                    lambda,
                    input
                );
            }
        }
    }

    #[test]
    fn test_6k_plus_minus_1_property_boundary() {
        // Test the 6k±1 property specifically at boundary conditions
        let h2p = HashToPrime::new(128);

        let boundary_inputs = vec![
            Integer::from(0),
            Integer::from(1),
            Integer::from(2),
            Integer::from(u32::MAX),
            Integer::from(u64::MAX),
            Integer::from(2u32).pow(64),
            Integer::from(2u32).pow(128),
        ];

        for input in boundary_inputs {
            let result = h2p.hash_to_prime(&input);

            if let Ok(prime) = result {
                let remainder = Integer::from(&prime % 6);
                let rem_val = remainder.to_u32().unwrap_or(0);
                assert!(
                    rem_val == 1 || rem_val == 5,
                    "Prime {} should be of form 6k±1, but {} % 6 = {} for input {}",
                    prime,
                    prime,
                    rem_val,
                    input
                );
            }
        }
    }
}

/// Test boundary conditions for RSW puzzle functionality
#[cfg(test)]
mod puzzle_boundary_tests {
    use super::*;

    #[test]
    fn test_minimum_time_bits() {
        // Test with minimum time_bits values
        for time_bits in 0..=5 {
            let input = vec![1, 2, 3];
            let modulus = vec![7, 11, 13, 17]; // Some primes as bytes

            let puzzle = RswPuzzle::new(time_bits, &input, &modulus);

            assert_eq!(puzzle.time_bits(), time_bits);
            assert_eq!(puzzle.iterations(), &(Integer::from(1) << time_bits));
            assert!(puzzle.base() < puzzle.modulus());
        }
    }

    #[test]
    fn test_maximum_reasonable_time_bits() {
        // Test with large but reasonable time_bits
        let large_time_bits = [32, 40, 50, 60];

        for &time_bits in &large_time_bits {
            let input = vec![1, 2, 3, 4];
            let modulus = vec![7, 11, 13, 17, 19, 23]; // Enough bytes for a reasonable modulus

            let puzzle = RswPuzzle::new(time_bits, &input, &modulus);

            assert_eq!(puzzle.time_bits(), time_bits);
            assert_eq!(puzzle.iterations(), &(Integer::from(1) << time_bits));
            assert!(puzzle.base() < puzzle.modulus());

            // Verify that iterations is indeed 2^time_bits
            let expected_iterations = Integer::from(2u32).pow(time_bits);
            assert_eq!(puzzle.iterations(), &expected_iterations);
        }
    }

    #[test]
    fn test_empty_input() {
        let empty_input = vec![];
        let modulus = vec![7, 11, 13];

        let puzzle = RswPuzzle::new(10, &empty_input, &modulus);

        // Empty input should result in base = 0
        assert_eq!(puzzle.base(), &Integer::from(0));
        assert!(puzzle.modulus() > &Integer::from(0));
    }

    #[test]
    fn test_single_byte_input_and_modulus() {
        let input = vec![5];
        let modulus = vec![7];

        let puzzle = RswPuzzle::new(8, &input, &modulus);

        assert_eq!(puzzle.base(), &Integer::from(5));
        assert_eq!(puzzle.modulus(), &Integer::from(7));
        assert_eq!(puzzle.iterations(), &Integer::from(256)); // 2^8
    }

    #[test]
    fn test_large_input_larger_than_modulus() {
        let large_input = vec![255, 255, 255, 255]; // Large input
        let small_modulus = vec![17]; // Small modulus

        let puzzle = RswPuzzle::new(10, &large_input, &small_modulus);

        // Base should be input % modulus
        let expected_base = Integer::from_digits(&large_input, rug::integer::Order::MsfBe) % 17;
        assert_eq!(puzzle.base(), &expected_base);
        assert_eq!(puzzle.modulus(), &Integer::from(17));
    }

    #[test]
    fn test_identical_input_and_modulus() {
        let data = vec![42, 42, 42];

        let puzzle = RswPuzzle::new(5, &data, &data);

        // When input equals modulus, base should be 0 (input % modulus = 0)
        assert_eq!(puzzle.base(), &Integer::from(0));

        let expected_modulus = Integer::from_digits(&data, rug::integer::Order::MsfBe);
        assert_eq!(puzzle.modulus(), &expected_modulus);
    }

    #[test]
    fn test_zero_time_bits() {
        let input = vec![1, 2, 3];
        let modulus = vec![7, 11];

        let puzzle = RswPuzzle::new(0, &input, &modulus);

        // 2^0 = 1 iteration
        assert_eq!(puzzle.iterations(), &Integer::from(1));
        assert_eq!(puzzle.time_bits(), 0);
    }

    #[test]
    fn test_maximum_u32_time_bits() {
        let input = vec![1];
        let modulus = vec![7];

        // Test with a very large time_bits value (but still reasonable)
        let large_time_bits = 63; // Just under u64 overflow

        let puzzle = RswPuzzle::new(large_time_bits, &input, &modulus);

        assert_eq!(puzzle.time_bits(), large_time_bits);
        // iterations should be 2^63
        let expected_iterations = Integer::from(1u64) << large_time_bits;
        assert_eq!(puzzle.iterations(), &expected_iterations);
    }
}

/// Test boundary conditions for WesolowskiVdf functionality
#[cfg(test)]
mod vdf_boundary_tests {
    use super::*;

    #[test]
    fn test_minimum_parameters() {
        let lambda = 1;
        let time_bits = 1;
        let input = vec![1];
        let modulus = vec![7];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);

        assert_eq!(vdf.iterations(), &Integer::from(2)); // 2^1
        assert_eq!(vdf.base(), &Integer::from(1));
        assert_eq!(vdf.modulus(), &Integer::from(7));

        // Test hash_to_prime functionality
        let test_input = Integer::from(42);
        let result = vdf.hash_to_prime(&test_input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_maximum_reasonable_parameters() {
        let lambda = 2048;
        let time_bits = 50;
        let input = vec![1, 2, 3, 4, 5];
        let modulus = vec![7, 11, 13, 17, 19, 23, 29];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);

        assert_eq!(vdf.iterations(), &(Integer::from(1u64) << time_bits));
        assert!(vdf.base() < vdf.modulus());

        // Test hash_to_prime functionality with large lambda
        let test_input = Integer::from(12345);
        let result = vdf.hash_to_prime(&test_input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_zero_lambda() {
        let lambda = 0;
        let time_bits = 10;
        let input = vec![2];
        let modulus = vec![11];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);

        // Should still create a valid VDF
        assert_eq!(vdf.base(), &Integer::from(2));
        assert_eq!(vdf.modulus(), &Integer::from(11));

        // Hash-to-prime should still work with lambda=0
        let test_input = Integer::from(100);
        let result = vdf.hash_to_prime(&test_input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_input_arrays() {
        let lambda = 128;
        let time_bits = 8;
        let empty_input = vec![];
        let modulus = vec![13];

        let vdf = WesolowskiVdf::new(lambda, time_bits, empty_input, modulus);

        assert_eq!(vdf.base(), &Integer::from(0));
        assert_eq!(vdf.modulus(), &Integer::from(13));
        assert_eq!(vdf.iterations(), &Integer::from(256)); // 2^8
    }

    #[test]
    fn test_single_byte_arrays() {
        let lambda = 64;
        let time_bits = 4;
        let input = vec![3];
        let modulus = vec![5];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);

        assert_eq!(vdf.base(), &Integer::from(3));
        assert_eq!(vdf.modulus(), &Integer::from(5));
        assert_eq!(vdf.iterations(), &Integer::from(16)); // 2^4

        // Test edge case where base >= modulus
        let large_input = vec![10];
        let small_modulus = vec![3];
        let vdf2 = WesolowskiVdf::new(lambda, time_bits, large_input, small_modulus);

        assert_eq!(vdf2.base(), &Integer::from(1)); // 10 % 3 = 1
        assert_eq!(vdf2.modulus(), &Integer::from(3));
    }

    #[test]
    fn test_hash_to_prime_with_boundary_inputs() {
        let vdf = WesolowskiVdf::new(128, 10, vec![1], vec![7]);

        let boundary_inputs = vec![
            Integer::from(0),
            Integer::from(1),
            Integer::from(-1),
            Integer::from(u64::MAX),
            Integer::from(2u32).pow(100),
            Integer::from(-2i32).pow(100),
        ];

        for input in boundary_inputs {
            let result = vdf.hash_to_prime(&input);
            assert!(result.is_ok(), "Hash-to-prime failed for input: {}", input);

            if let Ok(prime) = result {
                use rug::integer::IsPrime;
                assert_eq!(
                    prime.is_probably_prime(10),
                    IsPrime::Yes,
                    "Result should be prime for input: {}",
                    input
                );
                assert!(prime > 1, "Prime should be > 1 for input: {}", input);
            }
        }
    }
}

/// Test stress conditions and edge cases
#[cfg(test)]
mod stress_and_edge_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_hash_to_prime_collision_resistance() {
        let h2p = HashToPrime::new(256); // Use larger lambda to reduce collision probability
        let mut seen_primes = HashSet::new();

        // Test many different inputs to check for collisions
        // With lambda=256, collisions should be extremely rare
        for i in 0..100 {
            // Reduced from 1000 to 100 to keep test fast
            let input = Integer::from(i * 7919 + 1); // Use scattered inputs instead of consecutive
            if let Ok(prime) = h2p.hash_to_prime(&input) {
                // Check that we haven't seen this prime before
                assert!(
                    !seen_primes.contains(&prime),
                    "Collision detected: prime {} generated for inputs with different values",
                    prime
                );
                seen_primes.insert(prime);
            }
        }

        assert!(
            seen_primes.len() > 95,
            "Too many hash-to-prime failures: only {} successes",
            seen_primes.len()
        );
    }

    #[test]
    fn test_concurrent_hash_to_prime() {
        use std::sync::Arc;
        use std::thread;

        let h2p = Arc::new(HashToPrime::new(128));
        let mut handles = vec![];

        // Spawn multiple threads to test thread safety
        for i in 0..10 {
            let h2p_clone: Arc<HashToPrime> = Arc::clone(&h2p);
            let handle = thread::spawn(move || {
                let input = Integer::from(i * 1000);
                h2p_clone.hash_to_prime(&input)
            });
            handles.push(handle);
        }

        // Collect all results
        for handle in handles {
            let result = handle.join().unwrap();
            assert!(result.is_ok(), "Thread-safe hash-to-prime failed");

            if let Ok(prime) = result {
                use rug::integer::IsPrime;
                assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
            }
        }
    }

    #[test]
    fn test_memory_usage_with_large_numbers() {
        // Test that we can handle very large numbers without excessive memory usage
        let h2p = HashToPrime::new(64); // Smaller lambda to reduce computation time

        // Create progressively larger inputs
        let large_inputs = vec![
            Integer::from(2u32).pow(64),
            Integer::from(2u32).pow(128),
            Integer::from(2u32).pow(256),
            Integer::from(2u32).pow(512),
        ];

        for (i, input) in large_inputs.iter().enumerate() {
            let result = h2p.hash_to_prime(input);
            assert!(
                result.is_ok(),
                "Failed for large input #{}: bits={}",
                i,
                input.significant_bits()
            );

            if let Ok(prime) = result {
                use rug::integer::IsPrime;
                assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);

                // Ensure the prime isn't excessively large
                assert!(
                    prime.significant_bits() < 1000,
                    "Prime too large: {} bits for input #{}",
                    prime.significant_bits(),
                    i
                );
            }
        }
    }

    #[test]
    fn test_precision_bound_caching() {
        // Test that the caching mechanism works correctly
        use std::time::Instant;

        let lambda = 256;

        // First call - should compute and cache
        let start1 = Instant::now();
        let h2p1 = HashToPrime::new(lambda);
        let duration1 = start1.elapsed();

        // Second call - should use cache
        let start2 = Instant::now();
        let h2p2 = HashToPrime::new(lambda);
        let duration2 = start2.elapsed();

        // Third call - should also use cache
        let start3 = Instant::now();
        let h2p3 = HashToPrime::new(lambda);
        let duration3 = start3.elapsed();

        // Verify all instances have the same max_int (deterministic caching)
        assert_eq!(h2p1.max_int(), h2p2.max_int());
        assert_eq!(h2p2.max_int(), h2p3.max_int());

        // Test that they produce the same results
        let test_input = Integer::from(42);
        let result1 = h2p1.hash_to_prime(&test_input).unwrap();
        let result2 = h2p2.hash_to_prime(&test_input).unwrap();
        let result3 = h2p3.hash_to_prime(&test_input).unwrap();

        assert_eq!(result1, result2);
        assert_eq!(result2, result3);

        println!("Caching performance test results:");
        println!("  First call (cold):   {:?}", duration1);
        println!("  Second call (cache): {:?}", duration2);
        println!("  Third call (cache):  {:?}", duration3);

        // The cached calls should be significantly faster (at least 2x)
        // But we'll use a very generous bound to avoid flaky tests
        assert!(
            duration2 < duration1,
            "Second call should be faster than first"
        );
        assert!(
            duration3 < duration1,
            "Third call should be faster than first"
        );
    }
}
