use rug::Integer;
use rug::ops::Pow;
use rustaxa_vdf::hash::HashToPrime;
use rustaxa_vdf::prover::{CancellationToken, WesolowskiProver};
use rustaxa_vdf::puzzle::RswPuzzle;
use rustaxa_vdf::vdf::{Solution, WesolowskiVdf};
use rustaxa_vdf::verifier::WesolowskiVerifier;

/// Integration test: Complete VDF workflow with small parameters
#[test]
fn test_complete_vdf_workflow_small() {
    let lambda = 64u32;
    let time_bits = 8u32; // 2^8 = 256 iterations (feasible for testing)
    let input = vec![3, 5, 7];
    let modulus = vec![11, 13, 17, 19]; // Small RSA-like modulus

    // Create VDF
    let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);

    // Verify basic properties
    assert_eq!(vdf.iterations(), &Integer::from(256)); // 2^8
    assert!(*vdf.base() < *vdf.modulus());
    assert!(*vdf.modulus() > Integer::from(0));

    // Test hash-to-prime functionality
    let test_input = Integer::from(42);
    let prime_result = vdf.hash_to_prime(&test_input);
    assert!(prime_result.is_ok());

    let prime = prime_result.unwrap();
    use rug::integer::IsPrime;
    assert_eq!(prime.is_probably_prime(20), IsPrime::Yes);

    // Create prover and verifier
    let prover = WesolowskiProver::new(&vdf);
    let verifier = WesolowskiVerifier::new(&vdf);
    let token = CancellationToken::new();

    // Generate proof
    let solution = prover.prove(&token);

    // Verify the solution
    let is_valid = verifier.verify(&solution);
    assert!(is_valid, "VDF solution should be valid");
}

/// Integration test: Multiple VDF instances with different parameters
#[test]
fn test_multiple_vdf_instances() {
    let test_cases = vec![
        (32u32, 6u32, vec![1u8], vec![7u8]),
        (64u32, 8u32, vec![2u8, 3u8], vec![11u8, 13u8]),
        (128u32, 10u32, vec![5u8, 7u8, 11u8], vec![17u8, 19u8, 23u8]),
    ];

    for (lambda, time_bits, input, modulus) in test_cases {
        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);

        // Test basic functionality
        assert_eq!(vdf.iterations(), &(Integer::from(1) << time_bits));

        // Test hash-to-prime
        let test_input = Integer::from(lambda + time_bits);
        let prime_result = vdf.hash_to_prime(&test_input);
        assert!(prime_result.is_ok());

        if let Ok(prime) = prime_result {
            use rug::integer::IsPrime;
            assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
        }

        // Test prove-verify cycle with very small iterations
        if time_bits <= 10 {
            let prover = WesolowskiProver::new(&vdf);
            let verifier = WesolowskiVerifier::new(&vdf);
            let token = CancellationToken::new();

            let solution = prover.prove(&token);
            let is_valid = verifier.verify(&solution);
            assert!(
                is_valid,
                "Solution should be valid for lambda={}, time_bits={}",
                lambda, time_bits
            );
        }
    }
}

/// Integration test: Edge case inputs and parameters
#[test]
fn test_edge_case_integration() {
    // Test with minimal parameters
    let vdf_min = WesolowskiVdf::new(1, 1, vec![1], vec![2]);
    assert_eq!(vdf_min.iterations(), &Integer::from(2)); // 2^1

    let prime_result = vdf_min.hash_to_prime(&Integer::from(1));
    assert!(prime_result.is_ok());

    // Test with zero lambda (should still work)
    let vdf_zero_lambda = WesolowskiVdf::new(0, 5, vec![3], vec![7]);
    let prime_result = vdf_zero_lambda.hash_to_prime(&Integer::from(100));
    assert!(prime_result.is_ok());

    // Test with large input relative to modulus
    let vdf_large_input = WesolowskiVdf::new(64, 6, vec![255, 255], vec![11]);
    assert!(*vdf_large_input.base() < *vdf_large_input.modulus());

    // Test with identical input and modulus (base should be 0)
    let identical_data = vec![42, 43, 44];
    let vdf_identical = WesolowskiVdf::new(64, 6, identical_data.clone(), identical_data);
    assert_eq!(*vdf_identical.base(), Integer::from(0));
}

/// Integration test: Hash-to-prime determinism across multiple instances
#[test]
fn test_hash_to_prime_determinism_integration() {
    let lambda = 128u32;
    let test_inputs = vec![
        Integer::from(0),
        Integer::from(1),
        Integer::from(42),
        Integer::from(u64::MAX),
        Integer::from(2u32).pow(100),
        -Integer::from(12345),
    ];

    // Create multiple hash-to-prime instances
    let h2p1 = HashToPrime::new(lambda);
    let h2p2 = HashToPrime::new(lambda);
    let h2p3 = HashToPrime::new(lambda);

    for input in test_inputs {
        let result1 = h2p1.hash_to_prime(&input);
        let result2 = h2p2.hash_to_prime(&input);
        let result3 = h2p3.hash_to_prime(&input);

        // All should succeed or fail together
        assert_eq!(result1.is_ok(), result2.is_ok());
        assert_eq!(result2.is_ok(), result3.is_ok());

        // If successful, results should be identical
        if result1.is_ok() && result2.is_ok() && result3.is_ok() {
            let r1 = result1.unwrap();
            let r2 = result2.unwrap();
            let r3 = result3.unwrap();
            assert_eq!(r1, r2);
            assert_eq!(r2, r3);
        }
    }
}

/// Integration test: Puzzle and VDF consistency
#[test]
fn test_puzzle_vdf_consistency() {
    let time_bits = 12u32;
    let input = vec![7, 11, 13];
    let modulus = vec![17, 19, 23, 29];

    // Create puzzle and VDF with same parameters
    let puzzle = RswPuzzle::new(time_bits, &input, &modulus);
    let vdf = WesolowskiVdf::new(128, time_bits, input, modulus);

    // They should have consistent values
    assert_eq!(puzzle.time_bits(), time_bits);
    assert_eq!(puzzle.base(), vdf.base());
    assert_eq!(puzzle.modulus(), vdf.modulus());
    assert_eq!(puzzle.iterations(), vdf.iterations());

    // Iterations should be exactly 2^time_bits
    let expected_iterations = Integer::from(1) << time_bits;
    assert_eq!(*puzzle.iterations(), expected_iterations);
    assert_eq!(*vdf.iterations(), expected_iterations);
}

/// Integration test: Concurrent hash-to-prime operations
#[test]
fn test_concurrent_hash_to_prime() {
    use std::sync::Arc;
    use std::thread;

    let h2p = Arc::new(HashToPrime::new(128));
    let mut handles = vec![];

    // Spawn threads to test concurrent access
    for i in 0..10 {
        let h2p_clone: Arc<HashToPrime> = Arc::clone(&h2p);
        let handle = thread::spawn(move || {
            let mut results = vec![];
            for j in 0..10 {
                let input = Integer::from(i * 100 + j);
                let result = h2p_clone.hash_to_prime(&input);
                results.push(result);
            }
            results
        });
        handles.push(handle);
    }

    // Collect all results
    let mut all_results = vec![];
    for handle in handles {
        let thread_results = handle.join().unwrap();
        all_results.extend(thread_results);
    }

    // Verify all results are valid primes
    for result in all_results {
        assert!(result.is_ok(), "Concurrent hash-to-prime should succeed");

        if let Ok(prime) = result {
            use rug::integer::IsPrime;
            assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
            assert!(prime > 1);
        }
    }
}

/// Integration test: Large parameter stress test (with reasonable limits)
#[test]
fn test_large_parameter_stress() {
    // Test with moderately large lambda values
    let large_lambdas = [256, 512, 1024];

    for lambda in large_lambdas.iter() {
        let vdf = WesolowskiVdf::new(*lambda, 8, vec![1, 2], vec![7, 11]);

        // Should create successfully
        assert!(*vdf.iterations() > Integer::from(0));
        assert!(*vdf.modulus() > Integer::from(0));

        // Hash-to-prime should work
        let test_input = Integer::from(*lambda);
        let result = vdf.hash_to_prime(&test_input);
        assert!(result.is_ok(), "Hash-to-prime failed for lambda={}", lambda);

        if let Ok(prime) = result {
            use rug::integer::IsPrime;
            assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
        }
    }
}

/// Integration test: Verify invalid solutions are rejected
#[test]
fn test_invalid_solution_rejection() {
    let vdf = WesolowskiVdf::new(64, 8, vec![5], vec![13, 17]);
    let verifier = WesolowskiVerifier::new(&vdf);

    // Test with invalid solutions
    let invalid_solutions = vec![
        // Empty solution
        Solution {
            first: vec![],
            second: vec![],
        },
        // Zero values
        Solution {
            first: vec![0],
            second: vec![0],
        },
        // Random garbage
        Solution {
            first: vec![1, 2, 3, 4],
            second: vec![5, 6, 7, 8],
        },
        // Values too large (should exceed modulus)
        Solution {
            first: vec![255; 16],
            second: vec![255; 16],
        },
    ];

    for (i, invalid_solution) in invalid_solutions.iter().enumerate() {
        let is_valid = verifier.verify(invalid_solution);
        assert!(!is_valid, "Invalid solution #{} should be rejected", i);
    }
}

/// Integration test: Caching behavior verification
#[test]
fn test_caching_behavior() {
    use std::time::Instant;

    let lambda = 256u32;

    // First creation (cold)
    let start = Instant::now();
    let h2p1 = HashToPrime::new(lambda);
    let cold_duration = start.elapsed();

    // Second creation (should use cache)
    let start = Instant::now();
    let h2p2 = HashToPrime::new(lambda);
    let warm_duration = start.elapsed();

    // Third creation (should also use cache)
    let start = Instant::now();
    let h2p3 = HashToPrime::new(lambda);
    let warm_duration2 = start.elapsed();

    // Verify they have the same max_int (indicating cache hit)
    assert_eq!(h2p1.max_int(), h2p2.max_int());
    assert_eq!(h2p2.max_int(), h2p3.max_int());

    // Verify deterministic behavior
    let test_input = Integer::from(12345);
    let result1 = h2p1.hash_to_prime(&test_input).unwrap();
    let result2 = h2p2.hash_to_prime(&test_input).unwrap();
    let result3 = h2p3.hash_to_prime(&test_input).unwrap();

    assert_eq!(result1, result2);
    assert_eq!(result2, result3);

    println!("Cache performance test:");
    println!("  Cold creation: {:?}", cold_duration);
    println!("  Warm creation 1: {:?}", warm_duration);
    println!("  Warm creation 2: {:?}", warm_duration2);

    // Cached calls should generally be faster, but we'll be generous with timing
    // to avoid flaky tests on slow systems
    assert!(
        warm_duration < cold_duration * 2,
        "Cached creation should be faster"
    );
    assert!(
        warm_duration2 < cold_duration * 2,
        "Cached creation should be faster"
    );
}

/// Integration test: Memory usage with many instances
#[test]
fn test_memory_usage_many_instances() {
    // Create many VDF instances to test memory efficiency
    let instances: Vec<_> = (0..100)
        .map(|i| {
            let lambda = 64 + (i % 64);
            let time_bits = 6 + (i % 10);
            let input = vec![(i % 256) as u8];
            let modulus = vec![((i % 20) + 11) as u8];

            WesolowskiVdf::new(lambda, time_bits, input, modulus)
        })
        .collect();

    // Verify all instances are valid
    for (i, vdf) in instances.iter().enumerate() {
        assert!(
            *vdf.iterations() > Integer::from(0),
            "Instance {} should have positive iterations",
            i
        );
        assert!(
            *vdf.modulus() > Integer::from(0),
            "Instance {} should have positive modulus",
            i
        );

        // Test hash-to-prime for a sample of instances
        if i % 10 == 0 {
            let test_input = Integer::from(i);
            let result = vdf.hash_to_prime(&test_input);
            assert!(
                result.is_ok(),
                "Hash-to-prime should work for instance {}",
                i
            );
        }
    }

    println!("Successfully created {} VDF instances", instances.len());
}

/// Integration test: Boundary condition combinations
#[test]
fn test_boundary_combinations() {
    let boundary_cases = vec![
        // (lambda, time_bits, input, modulus, description)
        (1, 0, vec![0u8], vec![1u8], "minimal_all"),
        (1, 1, vec![1u8], vec![2u8], "minimal_valid"),
        (4096, 6, vec![1u8], vec![3u8], "max_lambda_small_others"),
        (64, 63, vec![1u8], vec![5u8], "large_time_bits"),
        (128, 10, vec![255u8; 64], vec![7u8], "large_input"),
        (128, 10, vec![1u8], vec![255u8; 32], "large_modulus"),
    ];

    for (lambda, time_bits, input, modulus, description) in boundary_cases {
        println!("Testing boundary case: {}", description);

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);

        // Basic sanity checks
        assert!(*vdf.iterations() > Integer::from(0));
        assert!(*vdf.base() >= Integer::from(0));
        assert!(*vdf.modulus() > Integer::from(0));
        assert!(*vdf.base() < *vdf.modulus());

        // Test hash-to-prime functionality
        let test_input = Integer::from(42);
        let result = vdf.hash_to_prime(&test_input);

        match result {
            Ok(prime) => {
                use rug::integer::IsPrime;
                assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
                assert!(prime > 1);
            }
            Err(msg) => {
                // For extreme boundary cases, hash-to-prime might fail
                // but should fail gracefully with a proper error message
                assert!(
                    msg.contains("Prime not found"),
                    "Unexpected error message for {}: {}",
                    description,
                    msg
                );
            }
        }
    }
}
