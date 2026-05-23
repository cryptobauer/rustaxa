use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ethereum_types::H512;
use rustaxa_arena::arena::Arena;
use rustaxa_types::ethereum::NodeId;
use std::hint::black_box;

fn benchmark_node_id() -> NodeId {
    let mut arr = [0u8; 64];
    arr[63] = 7;
    NodeId(H512::from(arr))
}

fn payload_of_size(size: usize, fill: u8) -> Bytes {
    Bytes::from(vec![fill; size])
}

fn bench_insert(c: &mut Criterion) {
    let from_node = benchmark_node_id();
    let mut group = c.benchmark_group("arena_insert");

    for (name, payload_size, fill) in [
        ("small", 128usize, 0x11u8),
        ("inline_limit", 1936usize, 0x22u8),
        ("heap", 4096usize, 0x33u8),
    ] {
        group.throughput(Throughput::Bytes(payload_size as u64));
        group.bench_with_input(
            BenchmarkId::new("insert", name),
            &payload_size,
            |b, &size| {
                b.iter(|| {
                    let arena = Arena::new(1024).expect("arena should be created");
                    let payload = payload_of_size(size, fill);
                    let reservation = arena.try_reserve().expect("slot should be reserved");
                    let packet_id = arena
                        .insert(reservation, from_node, payload)
                        .expect("insert should succeed");
                    black_box(packet_id);
                });
            },
        );
    }

    group.finish();
}

fn bench_insert_get_remove_roundtrip(c: &mut Criterion) {
    let from_node = benchmark_node_id();
    let payload = payload_of_size(512, 0x55);

    c.bench_function("arena_insert_get_remove_roundtrip", |b| {
        b.iter(|| {
            let arena = Arena::new(1024).expect("arena should be created");
            let reservation = arena.try_reserve().expect("slot should be reserved");
            let packet_id = arena
                .insert(reservation, from_node, payload.clone())
                .expect("insert should succeed");

            let packet = arena
                .borrow(packet_id)
                .expect("inserted packet should exist");
            black_box(packet.payload());
            drop(packet);

            let removed = arena.remove(packet_id).expect("packet should be removable");
            black_box(removed);
        });
    });
}

fn bench_fifo_like_workload(c: &mut Criterion) {
    let from_node = benchmark_node_id();

    c.bench_function("arena_fifo_like_workload", |b| {
        b.iter(|| {
            let arena = Arena::new(4096).expect("arena should be created");
            let mut keys = Vec::with_capacity(2048);

            for i in 0..2048usize {
                let payload = payload_of_size(256, (i % 251) as u8);
                let reservation = arena.try_reserve().expect("slot should be reserved");
                let packet_id = arena
                    .insert(reservation, from_node, payload)
                    .expect("insert should succeed");
                keys.push(packet_id);
            }

            for key in keys {
                let packet = arena.borrow(key).expect("packet should exist");
                black_box(packet.payload().len());
                drop(packet);
                let removed = arena.remove(key).expect("packet should be removable");
                black_box(removed);
            }
        });
    });
}

criterion_group!(
    benches,
    bench_insert,
    bench_insert_get_remove_roundtrip,
    bench_fifo_like_workload
);
criterion_main!(benches);
