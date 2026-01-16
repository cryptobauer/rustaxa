use crate::{hash::HashToPrime, puzzle::RswPuzzle};

pub struct Solution {
    pub first: Vec<u8>,
    pub second: Vec<u8>,
}

pub struct WesolowskiVdf {
    puzzle: RswPuzzle,
    hash: HashToPrime,
}

impl WesolowskiVdf {
    pub fn new(lambda: u32, time_bits: u32, input: Vec<u8>, modulus: Vec<u8>) -> Self {
        let puzzle = RswPuzzle::new(time_bits, &input, &modulus);
        let hash = HashToPrime::new(lambda);

        WesolowskiVdf { puzzle, hash }
    }

    pub fn base(&self) -> &rug::Integer {
        self.puzzle.base()
    }

    pub fn iterations(&self) -> &rug::Integer {
        self.puzzle.iterations()
    }

    pub fn modulus(&self) -> &rug::Integer {
        self.puzzle.modulus()
    }

    pub fn hash_to_prime(&self, input: &rug::Integer) -> Result<rug::Integer, String> {
        self.hash.hash_to_prime(input)
    }
}
