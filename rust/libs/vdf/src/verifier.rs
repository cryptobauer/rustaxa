use crate::vdf::{Solution, WesolowskiVdf};

pub struct WesolowskiVerifier<'a> {
    vdf: &'a WesolowskiVdf,
}

impl<'a> WesolowskiVerifier<'a> {
    pub fn new(vdf: &'a WesolowskiVdf) -> Self {
        WesolowskiVerifier { vdf }
    }

    /// Verifies a Wesolowski VDF solution
    ///
    /// This implements the Wesolowski verification algorithm:
    /// 1. Parse solution components (pi, y) from byte arrays
    /// 2. Validate that pi and y are in valid range [1, N-1]
    /// 3. Compute prime p = Hash(x || y)
    /// 4. Compute r = 2^T mod p
    /// 5. Verify that y = x^r * pi^p mod N
    ///
    /// # Arguments
    /// * `solution` - The VDF solution containing proof (pi) and output (y)
    ///
    /// # Returns
    /// * `true` if the solution is valid, `false` otherwise
    pub fn verify(&self, solution: &Solution) -> bool {
        let modulus = self.vdf.modulus(); // N
        let base = self.vdf.base(); // x
        let iterations = self.vdf.iterations(); // T

        let pi = rug::Integer::from_digits(&solution.first, rug::integer::Order::MsfBe); // pi = proof element
        let sigma = rug::Integer::from_digits(&solution.second, rug::integer::Order::MsfBe); // sigma = y = solution

        // Check that sigma (y) and pi are not zero and within valid range
        if sigma.is_zero() || pi.is_zero() || sigma >= *modulus || pi >= *modulus {
            return false;
        }

        // Prepare xy for hashing: xy = x || y (concatenate x and y)
        // More efficient: compute xy = x * 2^(modulus_bits) + y
        let modulus_bits = modulus.significant_bits();
        let xy = {
            let mut temp = rug::Integer::from(base);
            temp <<= modulus_bits; // Left shift x by number of bits in N
            temp + &sigma // Add y to get x||y
        };

        // Hash xy to get prime p
        let p = match self.vdf.hash_to_prime(&xy) {
            Ok(prime) => prime,
            Err(_) => return false, // Hash-to-prime failed
        };

        // Compute r = 2^T mod p
        let r = match rug::Integer::from(2).pow_mod(iterations, &p) {
            Ok(result) => result,
            Err(_) => return false, // Modular exponentiation failed
        };

        // Compute x^r mod N (use reference to avoid cloning)
        let x_r = match rug::Integer::from(base).pow_mod(&r, modulus) {
            Ok(result) => result,
            Err(_) => return false, // Modular exponentiation failed
        };

        // Compute pi^p mod N
        let pi_p = match pi.pow_mod(&p, modulus) {
            Ok(result) => result,
            Err(_) => return false, // Modular exponentiation failed
        };

        // Compute h = (x^r * pi^p) mod N - use more efficient approach
        let mut h = x_r;
        h *= pi_p;
        h %= modulus;

        // Verification: check if y == h
        sigma == h
    }
}

#[cfg(test)]
mod tests {
    use crate::prover::{CancellationToken, WesolowskiProver};

    use super::*;

    #[test]
    fn test_verifier_basic() {
        // Test basic verifier creation and functionality
        let lambda = 128u32;
        let time_bits = 20u32;
        let input = vec![1, 2, 3, 4];
        let modulus = vec![5, 6, 7, 8];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let _verifier = WesolowskiVerifier::new(&vdf);

        // Verify that the verifier can be created and basic accessors work
        assert!(!vdf.modulus().is_zero());
        assert!(!vdf.base().is_zero());
        assert!(!vdf.iterations().is_zero());
    }

    #[test]
    fn test_verifier_with_empty_solution() {
        // Test verification with empty solution (should fail)
        let lambda = 128u32;
        let time_bits = 20u32;
        let input = vec![1, 2, 3, 4];
        let modulus = vec![5, 6, 7, 8];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let verifier = WesolowskiVerifier::new(&vdf);

        // Create an empty solution
        let solution = Solution {
            first: vec![],
            second: vec![],
        };

        // Verification should fail for empty solution
        let result = verifier.verify(&solution);
        assert!(!result);
    }

    #[test]
    fn test_prove_and_verify() {
        // Test the prove function with a simple case
        let lambda = 128u32;
        let time_bits = 4u32; // Small number for faster testing (T = 2^4 = 16)

        // Create a simple modulus (should be a safe RSA modulus in practice)
        // For testing, we'll use a small number
        let modulus = vec![0x01, 0x01]; // 257 in decimal
        let input = vec![0x02]; // 2 in decimal

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let verifier = WesolowskiVerifier::new(&vdf);
        let stop_flag = CancellationToken::new();

        // Generate proof
        let prover = WesolowskiProver::new(&vdf);
        let solution = prover.prove(&stop_flag);

        // Verify that the solution is not empty
        assert!(!solution.first.is_empty());
        assert!(!solution.second.is_empty());

        // Verify the solution
        let is_valid = verifier.verify(&solution);
        assert!(is_valid, "Generated proof should be valid");
    }

    #[test]
    fn test_prove_verify_different_time_bits() {
        // Test with different time_bits values
        let lambda = 128u32;
        let modulus = vec![0x01, 0x01]; // 257 in decimal
        let input = vec![0x03]; // 3 in decimal

        for time_bits in [2u32, 3u32, 5u32] {
            let vdf = WesolowskiVdf::new(lambda, time_bits, input.clone(), modulus.clone());
            let verifier = WesolowskiVerifier::new(&vdf);
            let stop_flag = CancellationToken::new();

            let prover = WesolowskiProver::new(&vdf);
            let solution = prover.prove(&stop_flag);

            // Verify that the solution is not empty
            assert!(
                !solution.first.is_empty(),
                "Solution should not be empty for time_bits={}",
                time_bits
            );
            assert!(
                !solution.second.is_empty(),
                "Solution should not be empty for time_bits={}",
                time_bits
            );

            // Verify the solution
            let is_valid = verifier.verify(&solution);
            assert!(
                is_valid,
                "Generated proof should be valid for time_bits={}",
                time_bits
            );
        }
    }

    #[test]
    fn test_verify_with_invalid_proof() {
        // Test verification with deliberately invalid proof
        let lambda = 128u32;
        let time_bits = 4u32;
        let modulus = vec![0x01, 0x01]; // 257 in decimal
        let input = vec![0x02]; // 2 in decimal

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let verifier = WesolowskiVerifier::new(&vdf);
        let stop_flag = CancellationToken::new();

        // Generate a valid proof first
        let prover = WesolowskiProver::new(&vdf);
        let valid_solution = prover.prove(&stop_flag);

        // Create invalid solutions by modifying the valid one
        let mut invalid_solution_1 = Solution {
            first: valid_solution.first.clone(),
            second: valid_solution.second.clone(),
        };

        // Corrupt the proof element (pi)
        if !invalid_solution_1.first.is_empty() {
            invalid_solution_1.first[0] = invalid_solution_1.first[0].wrapping_add(1);
        }

        let mut invalid_solution_2 = Solution {
            first: valid_solution.first.clone(),
            second: valid_solution.second.clone(),
        };

        // Corrupt the solution element (y)
        if !invalid_solution_2.second.is_empty() {
            invalid_solution_2.second[0] = invalid_solution_2.second[0].wrapping_add(1);
        }

        // Both corrupted solutions should fail verification
        assert!(
            !verifier.verify(&invalid_solution_1),
            "Corrupted proof should be invalid"
        );
        assert!(
            !verifier.verify(&invalid_solution_2),
            "Corrupted solution should be invalid"
        );
    }

    #[test]
    fn test_verify_with_zero_values() {
        // Test verification with zero values in solution
        let lambda = 128u32;
        let time_bits = 4u32;
        let modulus = vec![0x01, 0x01];
        let input = vec![0x02];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let verifier = WesolowskiVerifier::new(&vdf);

        // Test with zero pi (proof element)
        let zero_pi_solution = Solution {
            first: vec![0x00],  // Zero proof
            second: vec![0x01], // Non-zero solution
        };
        assert!(
            !verifier.verify(&zero_pi_solution),
            "Zero proof should be invalid"
        );

        // Test with zero y (solution element)
        let zero_y_solution = Solution {
            first: vec![0x01],  // Non-zero proof
            second: vec![0x00], // Zero solution
        };
        assert!(
            !verifier.verify(&zero_y_solution),
            "Zero solution should be invalid"
        );
    }

    #[test]
    fn test_different_inputs_produce_different_solutions() {
        // Test that different inputs produce different solutions
        let lambda = 128u32;
        let time_bits = 6u32; // Slightly larger for better distinction
        // Use a larger modulus to reduce collision probability
        let modulus = vec![0x01, 0x00, 0x01]; // Larger modulus

        let vdf1 = WesolowskiVdf::new(lambda, time_bits, vec![0x02], modulus.clone());
        let verifier1 = WesolowskiVerifier::new(&vdf1);
        let vdf2 = WesolowskiVdf::new(lambda, time_bits, vec![0x05], modulus.clone()); // More different input
        let verifier2 = WesolowskiVerifier::new(&vdf2);

        let stop_flag = CancellationToken::new();

        let prover1 = WesolowskiProver::new(&vdf1);
        let prover2 = WesolowskiProver::new(&vdf2);
        let solution1 = prover1.prove(&stop_flag);
        let solution2 = prover2.prove(&stop_flag);

        // Solutions should be different for different inputs
        // Note: In very rare cases with small parameters, solutions might be the same
        // We test that at least one component is different
        let proofs_different = solution1.first != solution2.first;
        let solutions_different = solution1.second != solution2.second;
        assert!(
            proofs_different || solutions_different,
            "Different inputs should produce different proofs or solutions"
        );

        // Each solution should verify with its own verifier
        assert!(
            verifier1.verify(&solution1),
            "Solution1 should verify with verifier1"
        );
        assert!(
            verifier2.verify(&solution2),
            "Solution2 should verify with verifier2"
        );

        // Cross-verification should fail
        assert!(
            !verifier1.verify(&solution2),
            "Solution2 should not verify with verifier1"
        );
        assert!(
            !verifier2.verify(&solution1),
            "Solution1 should not verify with verifier2"
        );
    }

    #[test]
    fn test_cancellation_token_functionality() {
        // Test CancellationToken basic functionality
        let cancellation_token = CancellationToken::new();

        // Initially should not be cancelled
        assert!(!cancellation_token.is_cancelled());

        // After setting, should be cancelled
        cancellation_token.cancel();
        assert!(cancellation_token.is_cancelled());

        // Should remain cancelled
        assert!(cancellation_token.is_cancelled());
    }

    #[test]
    fn test_large_modulus_handling() {
        // Test with a larger, more realistic modulus
        let lambda = 128u32;
        let time_bits = 4u32;
        // A larger modulus (still small for testing, but more realistic structure)
        let modulus = vec![0xFF, 0xFF, 0xFF, 0xFF]; // Large 32-bit number
        let input = vec![0x12, 0x34]; // 2-byte input

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let verifier = WesolowskiVerifier::new(&vdf);
        let stop_flag = CancellationToken::new();

        let prover = WesolowskiProver::new(&vdf);
        let solution = prover.prove(&stop_flag);

        // Should produce valid solution
        assert!(!solution.first.is_empty());
        assert!(!solution.second.is_empty());

        // Verification should pass
        assert!(verifier.verify(&solution));
    }

    #[test]
    fn test_edge_case_time_bits() {
        // Test with minimal time_bits (T = 2^1 = 2 iterations)
        let lambda = 128u32;
        let time_bits = 1u32;
        let modulus = vec![0x01, 0x01];
        let input = vec![0x02];

        let vdf = WesolowskiVdf::new(lambda, time_bits, input, modulus);
        let verifier = WesolowskiVerifier::new(&vdf);
        let stop_flag = CancellationToken::new();

        let prover = WesolowskiProver::new(&vdf);
        let solution = prover.prove(&stop_flag);

        // Even with minimal iterations, should work
        assert!(!solution.first.is_empty());
        assert!(!solution.second.is_empty());
        assert!(verifier.verify(&solution));
    }
}
