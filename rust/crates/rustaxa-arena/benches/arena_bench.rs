use bytes::Bytes;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ethereum_types::H512;
use rustaxa_arena::arena::{Arena, PacketId};
use rustaxa_types::ethereum::NodeId;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};

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
                b.iter_batched_ref(
                    || Arena::new(1024).expect("arena should be created"),
                    |arena| {
                        let payload = payload_of_size(size, fill);
                        let reservation = arena.try_reserve().expect("slot should be reserved");
                        let packet_id = arena
                            .insert(reservation, from_node, payload)
                            .expect("insert should succeed");
                        black_box(packet_id);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_insert_get_remove_roundtrip(c: &mut Criterion) {
    let from_node = benchmark_node_id();
    let payload = payload_of_size(512, 0x55);

    c.bench_function("arena_insert_get_remove_roundtrip", |b| {
        b.iter_batched_ref(
            || Arena::new(1024).expect("arena should be created"),
            |arena| {
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
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_fifo_like_workload(c: &mut Criterion) {
    let from_node = benchmark_node_id();

    c.bench_function("arena_fifo_like_workload", |b| {
        b.iter_batched_ref(
            || Arena::new(4096).expect("arena should be created"),
            |arena| {
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
            },
            BatchSize::SmallInput,
        );
    });
}

struct PipelineBench {
    arena: Arc<Arena>,
    tx: SyncSender<PacketId>,
    consumer: JoinHandle<usize>,
}

fn make_pipeline(capacity: usize, expected_packets: usize) -> PipelineBench {
    let arena = Arc::new(Arena::new(capacity).expect("arena should be created"));
    let (tx, rx) = sync_channel(capacity);
    let consumer = spawn_pipeline_consumer(Arc::clone(&arena), rx, expected_packets);

    PipelineBench {
        arena,
        tx,
        consumer,
    }
}

fn spawn_pipeline_consumer(
    arena: Arc<Arena>,
    rx: Receiver<PacketId>,
    expected_packets: usize,
) -> JoinHandle<usize> {
    thread::spawn(move || {
        let mut processed_bytes = 0usize;

        for _ in 0..expected_packets {
            let packet_id = rx.recv().expect("producer should send packet id");
            let packet = arena.borrow(packet_id).expect("packet should be readable");
            processed_bytes += packet.payload().len();
            black_box(packet.payload());
            drop(packet);
            arena.remove(packet_id).expect("packet should be removable");
        }

        processed_bytes
    })
}

fn reserve_insert_send(
    arena: &Arena,
    tx: &SyncSender<PacketId>,
    from_node: NodeId,
    payload: Bytes,
) {
    let reservation = loop {
        if let Some(reservation) = arena.try_reserve() {
            break reservation;
        }
        thread::yield_now();
    };
    let packet_id = arena
        .insert(reservation, from_node, payload)
        .expect("insert should succeed");
    tx.send(packet_id)
        .expect("consumer should receive packet id");
}

fn bench_pipeline_spsc_channel(c: &mut Criterion) {
    const CAPACITY: usize = 1024;
    const PACKETS: usize = 4096;

    let from_node = benchmark_node_id();
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
                                from_node,
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
