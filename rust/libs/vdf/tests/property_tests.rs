use proptest::prelude::*;
use rug::Integer;
use rustaxa_vdf::hash::HashToPrime;
use rustaxa_vdf::puzzle::RswPuzzle;
use rustaxa_vdf::vdf::WesolowskiVdf;

proptest! {
    /// Property test: hash_to_prime should always return a prime number
    #[test]
    fn prop_hash_to_prime_returns_prime(
        lambda in 1u32..=128,
        input_value in 0u64..=1000000u64
    ) {
        let h2p = HashToPrime::new(lambda);
        let input = Integer::from(input_value);

        let result = h2p.hash_to_prime(&input);

        if let Ok(prime) = result {
            use rug::integer::IsPrime;
            prop_assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
            prop_assert!(prime > 1);
        }
    }

    /// Property test: hash_to_prime should be deterministic
    #[test]
    fn prop_hash_to_prime_deterministic(
        lambda in 1u32..=64,
        input_value in 0u64..=10000u64
    ) {
        let h2p1 = HashToPrime::new(lambda);
        let h2p2 = HashToPrime::new(lambda);
        let input = Integer::from(input_value);

        let result1 = h2p1.hash_to_prime(&input);
        let result2 = h2p2.hash_to_prime(&input);

        prop_assert_eq!(result1.is_ok(), result2.is_ok());

        if result1.is_ok() && result2.is_ok() {
            prop_assert_eq!(result1.unwrap(), result2.unwrap());
        }
    }

    /// Property test: hash_to_prime results should follow 6k±1 form
    #[test]
    fn prop_hash_to_prime_6k_plus_minus_1(
        lambda in 32u32..=128,
        input_value in 1u64..=100000u64
    ) {
        let h2p = HashToPrime::new(lambda);
        let input = Integer::from(input_value);

        let result = h2p.hash_to_prime(&input);

        if let Ok(prime) = result {
            let remainder = Integer::from(&prime % 6);
            let rem_val = remainder.to_u32().unwrap_or(0);
            prop_assert!(rem_val == 1 || rem_val == 5,
                "Prime {} should be of form 6k±1, got remainder {}", prime, rem_val);
        }
    }

    /// Property test: RswPuzzle should satisfy basic invariants
    #[test]
    fn prop_puzzle_invariants(
        time_bits in 1u32..=20,
        input_bytes in prop::collection::vec(any::<u8>(), 1..=16),
        modulus_bytes in prop::collection::vec(any::<u8>(), 2..=16)
    ) {
        // Ensure modulus is not zero by making the first byte non-zero
        let mut mod_bytes = modulus_bytes;
        if mod_bytes[0] == 0 {
            mod_bytes[0] = 1;
        }

        let puzzle = RswPuzzle::new(time_bits, &input_bytes, &mod_bytes);

        // Basic invariants
        prop_assert_eq!(puzzle.time_bits(), time_bits);
        prop_assert_eq!(puzzle.iterations(), &(Integer::from(1u32) << time_bits));
        prop_assert!(puzzle.base() < puzzle.modulus());
        prop_assert!(*puzzle.modulus() > Integer::from(0));

        // Base should be input % modulus
        let expected_base = Integer::from_digits(&input_bytes, rug::integer::Order::MsfBe)
            % Integer::from_digits(&mod_bytes, rug::integer::Order::MsfBe);
        prop_assert_eq!(puzzle.base(), &expected_base);

        // Modulus should match the input
        let expected_modulus = Integer::from_digits(&mod_bytes, rug::integer::Order::MsfBe);
        prop_assert_eq!(puzzle.modulus(), &expected_modulus);
    }

    /// Property test: WesolowskiVdf should maintain consistency
    #[test]
    fn prop_vdf_consistency(
        lambda in 1u32..=64,
        time_bits in 1u32..=15,
        input_bytes in prop::collection::vec(any::<u8>(), 1..=8),
        modulus_bytes in prop::collection::vec(any::<u8>(), 2..=8)
    ) {
        // Ensure modulus is not zero
        let mut mod_bytes = modulus_bytes;
        if mod_bytes[0] == 0 {
            mod_bytes[0] = 1;
        }

        let vdf = WesolowskiVdf::new(lambda, time_bits, input_bytes.clone(), mod_bytes.clone());

        // Check that VDF components are consistent with puzzle
        let puzzle = RswPuzzle::new(time_bits, &input_bytes, &mod_bytes);
        prop_assert_eq!(vdf.base(), puzzle.base());
        prop_assert_eq!(vdf.modulus(), puzzle.modulus());
        prop_assert_eq!(vdf.iterations(), puzzle.iterations());

        // Test hash_to_prime functionality
        let test_input = Integer::from(42u32);
        let hash_result = vdf.hash_to_prime(&test_input);

        if let Ok(prime) = hash_result {
            use rug::integer::IsPrime;
            prop_assert_eq!(prime.is_probably_prime(10), IsPrime::Yes);
            prop_assert!(prime > 1);
        }
    }

    /// Property test: Puzzle iterations should be exact powers of 2
    #[test]
    fn prop_puzzle_iterations_power_of_2(
        time_bits in 0u32..=20,
        input_bytes in prop::collection::vec(any::<u8>(), 1..=4),
        modulus_bytes in prop::collection::vec(any::<u8>(), 2..=4)
    ) {
        let mut mod_bytes = modulus_bytes;
        if mod_bytes[0] == 0 {
            mod_bytes[0] = 1;
        }

        let puzzle = RswPuzzle::new(time_bits, &input_bytes, &mod_bytes);

        // Iterations should be exactly 2^time_bits
        let expected = Integer::from(1u32) << time_bits;
        prop_assert_eq!(puzzle.iterations(), &expected);

        // Should be a power of 2 (exactly one bit set)
        if time_bits > 0 {
            prop_assert_eq!(puzzle.iterations().count_ones(), Some(1));
        } else {
            // 2^0 = 1, which has one bit set
            prop_assert_eq!(puzzle.iterations(), &Integer::from(1));
            prop_assert_eq!(puzzle.iterations().count_ones(), Some(1));
        }
    }
}
