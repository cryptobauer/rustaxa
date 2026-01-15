// Rivest-Shamir-Wagner puzzle
pub struct RswPuzzle {
    time_bits: u32,           // log₂(T) - determines T = 2^time_bits
    base: rug::Integer,       // x - the base value to be time-locked
    modulus: rug::Integer,    // N - the RSA modulus (p × q)
    iterations: rug::Integer, // T - number of sequential operations (2^time_bits)
}

impl RswPuzzle {
    pub fn new(time_bits: u32, input: &[u8], modulus: &[u8]) -> Self {
        let base = rug::Integer::from_digits(input, rug::integer::Order::MsfBe);
        let modulus = rug::Integer::from_digits(modulus, rug::integer::Order::MsfBe);
        let iterations = rug::Integer::from(1) << time_bits;
        let base = base % &modulus;

        RswPuzzle {
            time_bits,
            base,
            modulus,
            iterations,
        }
    }

    pub fn time_bits(&self) -> u32 {
        self.time_bits
    }

    pub fn base(&self) -> &rug::Integer {
        &self.base
    }

    pub fn modulus(&self) -> &rug::Integer {
        &self.modulus
    }

    pub fn iterations(&self) -> &rug::Integer {
        &self.iterations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_puzzle_initialization() {
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        let time_bits = 101;

        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let input: Vec<u8> = (0..7).map(|_| rng.r#gen()).collect();
        let modulus: Vec<u8> = (0..70).map(|_| rng.r#gen()).collect();

        println!("Generated input: {:?}", input);
        println!("Generated modulus: {:?}", modulus);

        let puzzle = RswPuzzle::new(time_bits, &input, &modulus);

        let expected_base = rug::Integer::from_digits(&input, rug::integer::Order::MsfBe)
            % &rug::Integer::from_digits(&modulus, rug::integer::Order::MsfBe);
        let expected_modulus = rug::Integer::from_digits(&modulus, rug::integer::Order::MsfBe);
        let expected_iterations = rug::Integer::from(1) << time_bits;

        assert_eq!(puzzle.time_bits(), time_bits);
        assert_eq!(puzzle.base(), &expected_base);
        assert_eq!(puzzle.modulus(), &expected_modulus);
        assert_eq!(puzzle.iterations(), &expected_iterations);
    }
}
