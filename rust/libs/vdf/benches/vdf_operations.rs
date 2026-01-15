use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use rug::Integer;
use rustaxa_vdf::vdf::WesolowskiVdf;

fn bench_vdf_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("vdf_creation");

    let lambdas = [64, 128, 256];
    let time_bits_values = [8, 12, 16];

    for &lambda in &lambdas {
        for &time_bits in &time_bits_values {
            group.bench_with_input(
                BenchmarkId::new(
                    "creation",
                    format!("lambda_{}_time_bits_{}", lambda, time_bits),
                ),
                &(lambda, time_bits),
                |b, &(lambda, time_bits)| {
                    let input = vec![1u8, 2u8, 3u8, 4u8];
                    let modulus = vec![5u8, 6u8, 7u8, 8u8, 9u8];
                    b.iter(|| {
                        black_box(WesolowskiVdf::new(
                            black_box(lambda),
                            black_box(time_bits),
                            black_box(input.clone()),
                            black_box(modulus.clone()),
                        ))
                    })
                },
            );
        }
    }

    group.finish();
}

fn bench_vdf_hash_to_prime(c: &mut Criterion) {
    let mut group = c.benchmark_group("vdf_hash_to_prime");

    let vdf = WesolowskiVdf::new(128, 10, vec![1u8, 2u8, 3u8], vec![4u8, 5u8, 6u8]);
    let test_input = Integer::from(12345u32);

    group.bench_function("hash_to_prime", |b| {
        b.iter(|| black_box(vdf.hash_to_prime(black_box(&test_input))))
    });

    group.finish();
}

criterion_group!(benches, bench_vdf_creation, bench_vdf_hash_to_prime);
criterion_main!(benches);
