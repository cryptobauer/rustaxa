use bytes::Bytes;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rustaxa_arena::arena::{Arena, SlotId};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};

const BENCH_PACKET_INLINE_LIMIT: usize = 1960;

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum BenchPacketPayload {
    Inline {
        len: usize,
        buf: [u8; BENCH_PACKET_INLINE_LIMIT],
    },
    Heap(Bytes),
}

impl Default for BenchPacketPayload {
    fn default() -> Self {
        Self::Inline {
            len: 0,
            buf: [0; BENCH_PACKET_INLINE_LIMIT],
        }
    }
}

#[derive(Clone)]
struct BenchPacket {
    received_micros: u64,
    peer_id: [u8; 64],
    payload: BenchPacketPayload,
}

impl Default for BenchPacket {
    fn default() -> Self {
        Self {
            received_micros: 0,
            peer_id: [0; 64],
            payload: BenchPacketPayload::default(),
        }
    }
}

impl BenchPacket {
    fn new(sequence: usize, payload_size: usize, fill: u8) -> Self {
        let mut peer_id = [0u8; 64];
        peer_id[0] = fill;
        peer_id[63] = (sequence % 251) as u8;

        Self {
            received_micros: sequence as u64,
            peer_id,
            payload: if payload_size > BENCH_PACKET_INLINE_LIMIT {
                BenchPacketPayload::Heap(payload_of_size(payload_size, fill))
            } else {
                let mut buf = [0u8; BENCH_PACKET_INLINE_LIMIT];
                buf[..payload_size].fill(fill);
                BenchPacketPayload::Inline {
                    len: payload_size,
                    buf,
                }
            },
        }
    }

    fn payload_len(&self) -> usize {
        match &self.payload {
            BenchPacketPayload::Inline { len, .. } => *len,
            BenchPacketPayload::Heap(bytes) => bytes.len(),
        }
    }

    fn stage_checksum(&self) -> usize {
        let payload_byte = match &self.payload {
            BenchPacketPayload::Inline { len, buf } if *len > 0 => buf[0] as usize,
            BenchPacketPayload::Heap(bytes) => bytes.first().copied().unwrap_or_default() as usize,
            _ => 0,
        };
        payload_byte ^ self.peer_id[63] as usize ^ self.received_micros as usize
    }
}

fn payload_of_size(size: usize, fill: u8) -> Bytes {
    Bytes::from(vec![fill; size])
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_micro_insert_only_synthetic");

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

    c.bench_function("arena_micro_single_thread_insert_borrow_remove", |b| {
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
    c.bench_function("arena_micro_single_thread_fifo_batch", |b| {
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
        if let Ok(reservation) = arena.try_reserve() {
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

    let mut group = c.benchmark_group("arena_pipeline_spsc_single_stage_remove");

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

struct MultiStagePipeline {
    arena: Arc<Arena<BenchPacket>>,
    first_tx: SyncSender<SlotId>,
    stage_handles: Vec<JoinHandle<usize>>,
}

fn make_multi_stage_pipeline(capacity: usize, packets: usize, stages: usize) -> MultiStagePipeline {
    assert!(
        stages > 0,
        "pipeline must have at least one processing stage"
    );

    let arena = Arc::new(Arena::<BenchPacket>::new(capacity).expect("arena should be created"));
    let (first_tx, first_rx) = sync_channel(capacity);
    let mut current_rx = Some(first_rx);
    let mut stage_handles = Vec::with_capacity(stages);

    for stage_index in 0..stages {
        let arena = Arc::clone(&arena);
        let rx = current_rx
            .take()
            .expect("pipeline receiver should be available");
        let is_final_stage = stage_index + 1 == stages;

        if is_final_stage {
            stage_handles.push(spawn_final_packet_stage(arena, rx, packets));
        } else {
            let (next_tx, next_rx) = sync_channel(capacity);
            stage_handles.push(spawn_forwarding_packet_stage(
                arena,
                rx,
                next_tx,
                packets,
                stage_index,
            ));
            current_rx = Some(next_rx);
        }
    }

    MultiStagePipeline {
        arena,
        first_tx,
        stage_handles,
    }
}

fn spawn_forwarding_packet_stage(
    arena: Arc<Arena<BenchPacket>>,
    rx: Receiver<SlotId>,
    tx: SyncSender<SlotId>,
    packets: usize,
    stage_index: usize,
) -> JoinHandle<usize> {
    thread::spawn(move || {
        let mut checksum = 0usize;

        for _ in 0..packets {
            let slot_id = rx.recv().expect("previous stage should send slot id");
            let packet = arena.borrow(slot_id).expect("packet should be readable");
            checksum ^= packet.stage_checksum().wrapping_add(stage_index);
            black_box(packet.payload_len());
            drop(packet);
            tx.send(slot_id).expect("next stage should receive slot id");
        }

        checksum
    })
}

fn spawn_final_packet_stage(
    arena: Arc<Arena<BenchPacket>>,
    rx: Receiver<SlotId>,
    packets: usize,
) -> JoinHandle<usize> {
    thread::spawn(move || {
        let mut checksum = 0usize;

        for _ in 0..packets {
            let slot_id = rx.recv().expect("previous stage should send slot id");
            let packet = arena.borrow(slot_id).expect("packet should be readable");
            checksum ^= packet.stage_checksum();
            black_box(packet.payload_len());
            drop(packet);
            arena.remove(slot_id).expect("packet should be removable");
        }

        checksum
    })
}

fn spawn_packet_producers(
    arena: Arc<Arena<BenchPacket>>,
    tx: SyncSender<SlotId>,
    producers: usize,
    packets: usize,
    payload_size: usize,
    fill: u8,
) -> Vec<JoinHandle<usize>> {
    let mut handles = Vec::with_capacity(producers);

    for producer_id in 0..producers {
        let arena = Arc::clone(&arena);
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            let mut produced = 0usize;

            for packet_index in (producer_id..packets).step_by(producers) {
                let reservation = loop {
                    if let Ok(reservation) = arena.try_reserve() {
                        break reservation;
                    }
                    thread::yield_now();
                };
                let packet = BenchPacket::new(
                    packet_index,
                    payload_size,
                    fill.wrapping_add((packet_index % 17) as u8),
                );
                let slot_id = arena
                    .insert(reservation, packet)
                    .expect("insert should succeed");
                tx.send(slot_id)
                    .expect("first stage should receive slot id");
                produced += 1;
            }

            produced
        }));
    }

    handles
}

fn bench_realistic_packet_pipeline(c: &mut Criterion) {
    const CAPACITY: usize = 1024;
    const PACKETS: usize = 8192;

    let mut group = c.benchmark_group("arena_realistic_packet_pipeline_mpsc_multistage");

    for (producers, stages, payload_name, payload_size, fill) in [
        (2usize, 3usize, "small_inline_256b", 256usize, 0x31u8),
        (4usize, 3usize, "small_inline_256b", 256usize, 0x41u8),
        (4usize, 6usize, "small_inline_256b", 256usize, 0x51u8),
        (4usize, 6usize, "near_inline_limit_1936b", 1936usize, 0x61u8),
        (4usize, 6usize, "heap_payload_4096b", 4096usize, 0x71u8),
    ] {
        group.throughput(Throughput::Bytes((PACKETS * payload_size) as u64));
        let scenario = format!(
            "producers={producers}/stages={stages}/packets={PACKETS}/capacity={CAPACITY}/payload={payload_name}"
        );

        group.bench_with_input(
            BenchmarkId::new(
                "reserve_insert_then_stage_borrow_forward_final_remove",
                scenario,
            ),
            &(producers, stages, payload_size, fill),
            |b, &(producers, stages, payload_size, fill)| {
                b.iter_batched(
                    || make_multi_stage_pipeline(CAPACITY, PACKETS, stages),
                    |pipeline| {
                        let producer_handles = spawn_packet_producers(
                            Arc::clone(&pipeline.arena),
                            pipeline.first_tx.clone(),
                            producers,
                            PACKETS,
                            payload_size,
                            fill,
                        );
                        drop(pipeline.first_tx);

                        let produced = producer_handles
                            .into_iter()
                            .map(|handle| handle.join().expect("producer should not panic"))
                            .sum::<usize>();
                        let stage_checksum = pipeline
                            .stage_handles
                            .into_iter()
                            .map(|handle| handle.join().expect("stage should not panic"))
                            .fold(0usize, |acc, checksum| acc ^ checksum);

                        black_box((produced, stage_checksum));
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
    bench_pipeline_spsc_channel,
    bench_realistic_packet_pipeline
);
criterion_main!(benches);
