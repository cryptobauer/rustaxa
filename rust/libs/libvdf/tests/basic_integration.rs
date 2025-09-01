use rug::Integer;
use rustaxa_vdf::hash::HashToPrime;
use rustaxa_vdf::puzzle::RswPuzzle;
use rustaxa_vdf::vdf::WesolowskiVdf;

#[test]
fn test_basic_functionality() {
    // Test HashToPrime
    let h2p = HashToPrime::new(64);
    let input = Integer::from(42);
    let result = h2p.hash_to_prime(&input);
    assert!(result.is_ok());

    // Test RswPuzzle
    let puzzle = RswPuzzle::new(8, &[1, 2, 3], &[7, 11]);
    assert_eq!(puzzle.time_bits(), 8);
    assert_eq!(*puzzle.iterations(), Integer::from(256)); // 2^8

    // Test WesolowskiVdf
    let vdf = WesolowskiVdf::new(64, 8, vec![1, 2, 3], vec![7, 11]);
    assert_eq!(*vdf.iterations(), Integer::from(256));

    let hash_result = vdf.hash_to_prime(&Integer::from(123));
    assert!(hash_result.is_ok());
}
