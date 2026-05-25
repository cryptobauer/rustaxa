use bytes::Bytes;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rustaxa_arena::arena::{Arena, SlotId};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};

fn payload_of_size(size: usize, fill: u8) -> Bytes {
    Bytes::from(vec![fill; size])
}

fn bench_insert(c: &mut Criterion) {
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
                b.iter_batched_ref(
                    || Arena::<Bytes>::new(1024).expect("arena should be created"),
                    |arena| {
                        let payload = payload_of_size(size, fill);
                        let reservation = arena.try_reserve().expect("slot should be reserved");
                        let slot_id = arena
                            .insert(reservation, payload)
                            .expect("insert should succeed");
                        black_box(slot_id);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_insert_get_remove_roundtrip(c: &mut Criterion) {
    let payload = payload_of_size(512, 0x55);

    c.bench_function("arena_insert_get_remove_roundtrip", |b| {
        b.iter_batched_ref(
            || Arena::<Bytes>::new(1024).expect("arena should be created"),
            |arena| {
                let reservation = arena.try_reserve().expect("slot should be reserved");
                let slot_id = arena
                    .insert(reservation, payload.clone())
                    .expect("insert should succeed");

                let data = arena.borrow(slot_id).expect("inserted value should exist");
                black_box(data.as_ref());
                drop(data);

                let removed = arena.remove(slot_id).expect("value should be removable");
                black_box(removed);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_fifo_like_workload(c: &mut Criterion) {
    c.bench_function("arena_fifo_like_workload", |b| {
        b.iter_batched_ref(
            || Arena::<Bytes>::new(4096).expect("arena should be created"),
            |arena| {
                let mut keys = Vec::with_capacity(2048);

                for i in 0..2048usize {
                    let payload = payload_of_size(256, (i % 251) as u8);
                    let reservation = arena.try_reserve().expect("slot should be reserved");
                    let slot_id = arena
                        .insert(reservation, payload)
                        .expect("insert should succeed");
                    keys.push(slot_id);
                }

                for key in keys {
                    let data = arena.borrow(key).expect("value should exist");
                    black_box(data.len());
                    drop(data);
                    let removed = arena.remove(key).expect("value should be removable");
                    black_box(removed);
                }
            },
            BatchSize::SmallInput,
        );
    });
}

struct PipelineBench {
    arena: Arc<Arena<Bytes>>,
    tx: SyncSender<SlotId>,
    consumer: JoinHandle<usize>,
}

fn make_pipeline(capacity: usize, expected_packets: usize) -> PipelineBench {
    let arena = Arc::new(Arena::<Bytes>::new(capacity).expect("arena should be created"));
    let (tx, rx) = sync_channel(capacity);
    let consumer = spawn_pipeline_consumer(Arc::clone(&arena), rx, expected_packets);

    PipelineBench {
        arena,
        tx,
        consumer,
    }
}

fn spawn_pipeline_consumer(
    arena: Arc<Arena<Bytes>>,
    rx: Receiver<SlotId>,
    expected_packets: usize,
) -> JoinHandle<usize> {
    thread::spawn(move || {
        let mut processed_bytes = 0usize;

        for _ in 0..expected_packets {
            let slot_id = rx.recv().expect("producer should send slot id");
            let data = arena.borrow(slot_id).expect("value should be readable");
            processed_bytes += data.len();
            black_box(data.as_ref());
            drop(data);
            arena.remove(slot_id).expect("value should be removable");
        }

        processed_bytes
    })
}

fn reserve_insert_send(arena: &Arena<Bytes>, tx: &SyncSender<SlotId>, payload: Bytes) {
    let reservation = loop {
        if let Some(reservation) = arena.try_reserve() {
            break reservation;
        }
        thread::yield_now();
    };
    let slot_id = arena
        .insert(reservation, payload)
        .expect("insert should succeed");
    tx.send(slot_id).expect("consumer should receive slot id");
}

fn bench_pipeline_spsc_channel(c: &mut Criterion) {
    const CAPACITY: usize = 1024;
    const PACKETS: usize = 4096;

    let mut group = c.benchmark_group("arena_pipeline_spsc_channel");

    for (name, payload_size, fill) in [
        ("small", 256usize, 0x44u8),
        ("inline_limit", 1936usize, 0x55u8),
        ("heap", 4096usize, 0x66u8),
    ] {
        group.throughput(Throughput::Bytes((PACKETS * payload_size) as u64));
        group.bench_with_input(
            BenchmarkId::new("insert_send_borrow_remove", name),
            &payload_size,
            |b, &size| {
                b.iter_batched(
                    || make_pipeline(CAPACITY, PACKETS),
                    |pipeline| {
                        for i in 0..PACKETS {
                            reserve_insert_send(
                                &pipeline.arena,
                                &pipeline.tx,
                                payload_of_size(size, fill.wrapping_add((i % 17) as u8)),
                            );
                        }

                        drop(pipeline.tx);
                        let processed_bytes =
                            pipeline.consumer.join().expect("consumer should not panic");
                        black_box(processed_bytes);
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_insert,
    bench_insert_get_remove_roundtrip,
    bench_fifo_like_workload,
    bench_pipeline_spsc_channel
);
criterion_main!(benches);
