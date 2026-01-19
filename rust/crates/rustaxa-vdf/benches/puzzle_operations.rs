use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use rustaxa_vdf::puzzle::RswPuzzle;
use std::time::Duration;

fn bench_puzzle_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_creation");

    // Test puzzle creation with different time_bits
    let time_bits_values = [8, 16, 24, 32, 40, 48, 56, 63];
    let input = vec![1, 2, 3, 4, 5];
    let modulus = vec![7, 11, 13, 17, 19, 23, 29, 31];

    for time_bits in time_bits_values.iter() {
        group.bench_with_input(
            BenchmarkId::new("time_bits", time_bits),
            time_bits,
            |b, &time_bits| {
                b.iter(|| {
                    black_box(RswPuzzle::new(
                        time_bits,
                        black_box(&input),
                        black_box(&modulus),
                    ))
                })
            },
        );
    }
    group.finish();
}

fn bench_puzzle_different_input_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_input_sizes");

    let time_bits = 20u32;
    let modulus = vec![7, 11, 13, 17];

    let input_sizes = vec![
        ("tiny", vec![1u8]),
        ("small", vec![1u8, 2u8, 3u8, 4u8]),
        ("medium", vec![1u8; 16]),
        ("large", vec![1u8; 64]),
        ("very_large", vec![1u8; 256]),
        ("huge", vec![1u8; 1024]),
    ];

    for (name, input) in input_sizes.iter() {
        group.bench_with_input(BenchmarkId::new("input_size", name), input, |b, input| {
            b.iter(|| {
                black_box(RswPuzzle::new(
                    time_bits,
                    black_box(input),
                    black_box(&modulus),
                ))
            })
        });
    }
    group.finish();
}

fn bench_puzzle_different_modulus_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_modulus_sizes");

    let time_bits = 20u32;
    let input = vec![5, 7, 11];

    let modulus_sizes = vec![
        ("tiny", vec![13u8]),
        ("small", vec![13u8, 17u8]),
        ("medium", vec![13u8, 17u8, 19u8, 23u8]),
        ("large", vec![13u8; 16]),
        ("very_large", vec![13u8; 64]),
        ("huge", vec![13u8; 256]),
    ];

    for (name, modulus) in modulus_sizes.iter() {
        group.bench_with_input(
            BenchmarkId::new("modulus_size", name),
            modulus,
            |b, modulus| {
                b.iter(|| {
                    black_box(RswPuzzle::new(
                        time_bits,
                        black_box(&input),
                        black_box(modulus),
                    ))
                })
            },
        );
    }
    group.finish();
}

fn bench_puzzle_accessor_methods(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_accessors");

    let puzzle = RswPuzzle::new(25, &[1, 2, 3, 4], &[7, 11, 13, 17, 19]);

    group.bench_function("time_bits", |b| b.iter(|| black_box(puzzle.time_bits())));

    group.bench_function("base", |b| b.iter(|| black_box(puzzle.base())));

    group.bench_function("modulus", |b| b.iter(|| black_box(puzzle.modulus())));

    group.bench_function("iterations", |b| b.iter(|| black_box(puzzle.iterations())));

    group.finish();
}

fn bench_puzzle_edge_cases(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_edge_cases");

    let edge_cases = vec![
        ("zero_time_bits", 0u32, vec![1u8], vec![2u8]),
        ("max_time_bits", 63u32, vec![1u8], vec![2u8]),
        ("empty_input", 10u32, vec![], vec![3u8]),
        ("single_byte_input", 10u32, vec![42u8], vec![7u8]),
        ("single_byte_modulus", 10u32, vec![1u8, 2u8], vec![11u8]),
        (
            "large_input_small_modulus",
            15u32,
            vec![255u8; 32],
            vec![13u8],
        ),
        (
            "equal_input_modulus",
            15u32,
            vec![17u8, 19u8],
            vec![17u8, 19u8],
        ),
    ];

    for (name, time_bits, input, modulus) in edge_cases.iter() {
        group.bench_with_input(
            BenchmarkId::new("edge_case", name),
            &(time_bits, input, modulus),
            |b, (time_bits, input, modulus)| {
                b.iter(|| {
                    black_box(RswPuzzle::new(
                        **time_bits,
                        black_box(input),
                        black_box(modulus),
                    ))
                })
            },
        );
    }
    group.finish();
}

fn bench_puzzle_batch_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_batch");
    group.throughput(Throughput::Elements(100));

    group.bench_function("100_puzzles", |b| {
        b.iter(|| {
            let puzzles: Vec<_> = (0..100)
                .map(|i| {
                    let time_bits = 10 + (i % 20);
                    let input = vec![(i % 256) as u8, ((i * 2) % 256) as u8];
                    let modulus = vec![7u8, 11u8, 13u8, ((i % 10) + 17) as u8];

                    RswPuzzle::new(time_bits, &input, &modulus)
                })
                .collect();

            black_box(puzzles)
        })
    });

    group.finish();
}

fn bench_puzzle_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_memory");

    group.bench_function("memory_usage_many_puzzles", |b| {
        b.iter(|| {
            let puzzles: Vec<_> = (0..1000)
                .map(|i| {
                    let time_bits = 8 + (i % 32);
                    let input = vec![(i % 256) as u8];
                    let modulus = vec![((i % 100) + 101) as u8]; // Ensure non-zero modulus

                    RswPuzzle::new(time_bits, &input, &modulus)
                })
                .collect();

            // Use puzzles to ensure they're not optimized away
            let total_time_bits: u32 = puzzles
                .iter()
                .take(100) // Sample to avoid excessive computation
                .map(|p| p.time_bits())
                .sum();

            black_box(total_time_bits)
        })
    });

    group.finish();
}

fn bench_puzzle_modular_arithmetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_modular_arithmetic");

    // Test cases where modular arithmetic matters
    let arithmetic_cases = vec![
        ("input_less_than_modulus", vec![5u8], vec![17u8]),
        ("input_equal_to_modulus", vec![23u8], vec![23u8]),
        ("input_greater_than_modulus", vec![100u8], vec![7u8]),
        ("large_input_small_modulus", vec![255u8, 255u8], vec![3u8]),
        ("multi_byte_equal", vec![1u8, 2u8, 3u8], vec![1u8, 2u8, 3u8]),
        (
            "multi_byte_mod",
            vec![255u8, 254u8, 253u8],
            vec![1u8, 1u8, 1u8],
        ),
    ];

    let time_bits = 15u32;

    for (name, input, modulus) in arithmetic_cases.iter() {
        group.bench_with_input(
            BenchmarkId::new("arithmetic_case", name),
            &(input, modulus),
            |b, (input, modulus)| {
                b.iter(|| {
                    let puzzle = RswPuzzle::new(time_bits, black_box(input), black_box(modulus));

                    // Access the computed values to ensure computation happens
                    black_box((puzzle.base().clone(), puzzle.modulus().clone()))
                })
            },
        );
    }
    group.finish();
}

fn bench_puzzle_iterations_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_iterations");

    // Test iteration computation for different time_bits values
    let time_bits_cases = [0, 1, 8, 16, 24, 32, 40, 48, 56, 63];
    let input = vec![1u8];
    let modulus = vec![7u8];

    for time_bits in time_bits_cases.iter() {
        group.bench_with_input(
            BenchmarkId::new("time_bits", time_bits),
            time_bits,
            |b, &time_bits| {
                b.iter(|| {
                    let puzzle = RswPuzzle::new(time_bits, black_box(&input), black_box(&modulus));

                    // Access iterations to ensure computation happens
                    black_box(puzzle.iterations().clone())
                })
            },
        );
    }
    group.finish();
}

fn bench_puzzle_concurrent_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_concurrent");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("parallel_puzzle_creation", |b| {
        use rayon::prelude::*;

        b.iter(|| {
            (0..100u32)
                .into_par_iter()
                .map(|i| {
                    let time_bits = 10 + (i % 15);
                    let input = vec![(i % 256) as u8, ((i * 3) % 256) as u8];
                    let modulus = vec![7u8, 11u8, ((i % 20) + 13) as u8];

                    RswPuzzle::new(time_bits, &input, &modulus)
                })
                .collect::<Vec<_>>()
        })
    });

    group.finish();
}

fn bench_puzzle_big_integer_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("puzzle_big_integers");

    // Test with very large numbers to stress big integer operations
    let large_cases = vec![
        ("large_input", vec![255u8; 64], vec![127u8; 8]),
        ("large_modulus", vec![1u8; 8], vec![255u8; 64]),
        ("both_large", vec![128u8; 32], vec![64u8; 32]),
        ("huge_input", vec![200u8; 128], vec![50u8; 16]),
    ];

    let time_bits = 20u32;

    for (name, input, modulus) in large_cases.iter() {
        group.bench_with_input(
            BenchmarkId::new("big_int_case", name),
            &(input, modulus),
            |b, (input, modulus)| {
                b.iter(|| {
                    let puzzle = RswPuzzle::new(time_bits, black_box(input), black_box(modulus));

                    // Ensure all big integer operations are performed
                    black_box((
                        puzzle.base().clone(),
                        puzzle.modulus().clone(),
                        puzzle.iterations().clone(),
                    ))
                })
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_puzzle_creation,
    bench_puzzle_different_input_sizes,
    bench_puzzle_different_modulus_sizes,
    bench_puzzle_accessor_methods,
    bench_puzzle_edge_cases,
    bench_puzzle_batch_creation,
    bench_puzzle_memory_efficiency,
    bench_puzzle_modular_arithmetic,
    bench_puzzle_iterations_computation,
    bench_puzzle_concurrent_creation,
    bench_puzzle_big_integer_operations,
);

criterion_main!(benches);
