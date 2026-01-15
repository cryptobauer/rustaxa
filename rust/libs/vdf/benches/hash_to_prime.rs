use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use rug::Integer;
use rug::ops::Pow;
use rustaxa_vdf::hash::HashToPrime;
use std::time::Duration;

fn bench_hash_to_prime_different_lambdas(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_lambdas");

    let lambdas = [64, 128, 256, 512, 1024];
    let test_input = Integer::from(12345u32);

    for lambda in lambdas.iter() {
        group.bench_with_input(BenchmarkId::new("lambda", lambda), lambda, |b, &lambda| {
            let h2p = HashToPrime::new(lambda);
            b.iter(|| black_box(h2p.hash_to_prime(black_box(&test_input)).unwrap()))
        });
    }
    group.finish();
}

fn bench_hash_to_prime_different_inputs(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_inputs");

    let h2p = HashToPrime::new(128);

    // Test different input sizes
    let inputs = vec![
        ("small", Integer::from(42u32)),
        ("medium", Integer::from(u64::MAX)),
        ("large", Integer::from(2u32).pow(128)),
        ("very_large", Integer::from(2u32).pow(256)),
    ];

    for (name, input) in inputs.iter() {
        group.bench_with_input(BenchmarkId::new("input_size", name), input, |b, input| {
            b.iter(|| black_box(h2p.hash_to_prime(black_box(input)).unwrap()))
        });
    }
    group.finish();
}

fn bench_hash_to_prime_creation_with_caching(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_creation");

    let lambda = 256u32;

    // Benchmark cold creation (first time)
    group.bench_function("cold_creation", |b| {
        b.iter(|| {
            // Clear cache by using unique lambda values
            static mut COUNTER: u32 = 1000;
            unsafe {
                COUNTER += 1;
                black_box(HashToPrime::new(COUNTER))
            }
        })
    });

    // Benchmark warm creation (should use cache)
    group.bench_function("warm_creation", |b| {
        // Pre-warm the cache
        let _ = HashToPrime::new(lambda);

        b.iter(|| black_box(HashToPrime::new(lambda)))
    });

    group.finish();
}

fn bench_hash_to_prime_sequential_calls(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_sequential");
    group.throughput(Throughput::Elements(100));

    let h2p = HashToPrime::new(128);

    group.bench_function("100_sequential_calls", |b| {
        b.iter(|| {
            for i in 0..100 {
                let input = Integer::from(i);
                black_box(h2p.hash_to_prime(black_box(&input)).unwrap());
            }
        })
    });

    group.finish();
}

fn bench_hash_to_prime_with_negative_inputs(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_negative");

    let h2p = HashToPrime::new(128);

    let inputs = vec![
        ("positive", Integer::from(12345)),
        ("negative", Integer::from(-12345)),
        ("zero", Integer::from(0)),
        ("large_negative", -Integer::from(2u32).pow(100)),
    ];

    for (name, input) in inputs.iter() {
        group.bench_with_input(BenchmarkId::new("input_type", name), input, |b, input| {
            b.iter(|| black_box(h2p.hash_to_prime(black_box(input)).unwrap()))
        });
    }
    group.finish();
}

fn bench_hash_to_prime_parallel(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_parallel");
    group.measurement_time(Duration::from_secs(10));

    let h2p = std::sync::Arc::new(HashToPrime::new(128));

    group.bench_function("parallel_4_threads", |b| {
        use rayon::prelude::*;

        b.iter(|| {
            (0..100u32)
                .into_par_iter()
                .map(|i| {
                    let h2p = h2p.clone();
                    let input = Integer::from(i * 1000);
                    black_box(h2p.hash_to_prime(&input).unwrap())
                })
                .collect::<Vec<_>>()
        })
    });

    group.finish();
}

fn bench_precision_bound_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("precision_bound");

    // Test precision bound computation for different lambda values
    let lambdas = [32, 64, 128, 256, 512, 1024, 2048];

    for lambda in lambdas.iter() {
        group.bench_with_input(BenchmarkId::new("lambda", lambda), lambda, |b, &lambda| {
            b.iter(|| {
                // Use a unique lambda each time to avoid caching effects
                static mut BASE_LAMBDA: u32 = 10000;
                unsafe {
                    BASE_LAMBDA += 1;
                    let unique_lambda = BASE_LAMBDA + lambda;
                    black_box(HashToPrime::new(unique_lambda))
                }
            })
        });
    }
    group.finish();
}

fn bench_hash_to_prime_edge_cases(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_edge_cases");

    let test_cases = vec![
        ("lambda_1", 1u32, Integer::from(42)),
        ("lambda_max", 4096u32, Integer::from(42)),
        ("input_boundary_u64_max", 128u32, Integer::from(u64::MAX)),
        ("input_power_of_2", 128u32, Integer::from(2u32).pow(64)),
    ];

    for (name, lambda, input) in test_cases.iter() {
        group.bench_with_input(
            BenchmarkId::new("case", name),
            &(lambda, input),
            |b, (lambda, input)| {
                let h2p = HashToPrime::new(**lambda);
                b.iter(|| black_box(h2p.hash_to_prime(black_box(input)).unwrap()))
            },
        );
    }
    group.finish();
}

fn bench_hash_to_prime_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_to_prime_memory");
    group.measurement_time(Duration::from_secs(15));

    // Test memory efficiency with many hash-to-prime instances
    group.bench_function("many_instances", |b| {
        b.iter(|| {
            let instances: Vec<_> = (0..100)
                .map(|i| {
                    let lambda = 64 + (i % 64); // Vary lambda to test cache efficiency
                    HashToPrime::new(lambda)
                })
                .collect();

            // Use the instances to ensure they're not optimized away
            let total_max_ints: rug::Integer = instances
                .iter()
                .take(10) // Just sample a few to avoid excessive computation
                .map(|h2p| h2p.max_int())
                .fold(Integer::from(0), |acc, max_int| acc + max_int);

            black_box(total_max_ints)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_hash_to_prime_different_lambdas,
    bench_hash_to_prime_different_inputs,
    bench_hash_to_prime_creation_with_caching,
    bench_hash_to_prime_sequential_calls,
    bench_hash_to_prime_with_negative_inputs,
    bench_hash_to_prime_parallel,
    bench_precision_bound_computation,
    bench_hash_to_prime_edge_cases,
    bench_hash_to_prime_memory_efficiency,
);

criterion_main!(benches);
