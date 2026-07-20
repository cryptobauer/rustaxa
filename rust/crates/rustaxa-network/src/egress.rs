//! Egress worker for queued network packet events.
//!
//! The egress stage owns the consumer side of the bounded network event queue.
//! Producers publish [`NetworkEvent`] values that point at packets stored in the
//! shared arena; the egress processor borrows each packet by slot id and is the
//! place where early filtering and dispatch into consensus-facing events will
//! be wired. The current implementation keeps that processing minimal while the
//! pipeline boundaries are being established.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

use rtrb::{Consumer, PopError};
use rustaxa_arena::arena::Arena;

use crate::{events::NetworkEvent, packet::Packet, peers::PeerRegistry};

/// Lifecycle wrapper for egress worker threads.
///
/// `Egress` owns a single [`Processor`] until [`Egress::listen`] is called.
/// Starting the worker moves the processor into a background thread and stores
/// the thread handle so [`Egress::shutdown`] can request termination and join
/// it.
pub struct Egress {
    processor: Option<Processor>,
    workers: Vec<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl Egress {
    /// Creates an egress worker over the supplied packet arena and event queue.
    ///
    /// The worker starts in a shut down state. Call [`Egress::listen`] to spawn
    /// the background processing thread.
    pub fn new(
        arena: Arc<Arena<Packet>>,
        events: Consumer<NetworkEvent>,
        registry: Arc<PeerRegistry>,
    ) -> Self {
        let shutdown = Arc::new(AtomicBool::new(true));
        Egress {
            processor: Some(Processor::new(arena, events, registry, shutdown.clone())),
            workers: vec![],
            shutdown,
        }
    }

    /// Starts the background egress loop.
    ///
    /// This consumes the internally stored processor and spawns one worker
    /// thread. The current type is single-start: calling `listen` more than once
    /// is a programming error.
    pub fn start(&mut self) {
        self.shutdown.store(false, Ordering::Release);
        let processor = self.processor.take();
        let handle = thread::spawn(move || processor.unwrap().listen());
        self.workers.push(handle);
    }

    /// Requests shutdown and waits for all started egress workers to exit.
    ///
    /// Calling this before [`Egress::listen`] is valid and simply consumes the
    /// idle worker wrapper.
    pub fn shutdown(self) {
        self.shutdown.store(true, Ordering::Release);
        for handle in self.workers {
            if let Err(e) = handle.join() {
                println!("error while stopping worker {e:?}")
            }
        }
    }
}

/// Single-threaded event consumer for network egress.
///
/// The processor polls a bounded event queue, borrows packets from the shared
/// arena, and runs packet processing for each event. It does not own a thread by
/// itself; [`Egress`] is responsible for spawning and joining workers.
pub struct Processor {
    arena: Arc<Arena<Packet>>,
    events: Consumer<NetworkEvent>,
    registry: Arc<PeerRegistry>,
    shutdown: Arc<AtomicBool>,
}

impl Processor {
    /// Creates a processor from packet storage, event input, and a shutdown flag.
    pub fn new(
        arena: Arc<Arena<Packet>>,
        events: Consumer<NetworkEvent>,
        registry: Arc<PeerRegistry>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Processor {
            arena,
            events,
            registry,
            shutdown,
        }
    }

    /// Runs the event loop until `shutdown` is set.
    ///
    /// Empty queues use [`crossbeam_utils::Backoff`] so idle workers back off
    /// without sleeping on the hot burst path.
    pub fn listen(&mut self) {
        let backoff = crossbeam_utils::Backoff::new();

        while !self.shutdown.load(Ordering::Acquire) {
            match self.events.pop() {
                Ok(event) => {
                    self.process(event);
                    backoff.reset();
                }
                Err(PopError::Empty) => {
                    // We will start with a more friendly (less energy, less aggressive) approach.
                    // Backoff will slowly drive the thread down when there aren't any incoming
                    // packages, but at times of bursts stay in a hot spin loop.
                    backoff.snooze();
                }
            }
        }
    }

    /// Processes one queued packet event.
    ///
    /// This currently validates that the arena slot can be borrowed and exposes
    /// the sender for the future filter/dispatch stage.
    fn process(&self, event: NetworkEvent) -> Result<(), anyhow::Error> {
        // filter: packet_handler.cpp - if peer is now disconnected drop the packet (this should happen earlier)
        // filter: packet_handler.cpp - malicious, broken packet, broken RLP, other things (also as early as possible)
        // process is implemented for each type of package

        let _ = self.arena.borrow(event.slot)?;

        Ok(())

        // match self.registry.connected(packet.peer.) {
        //     Ok(peer) => {}
        //     Err(err) => {}
        // }

        // println!("got packet from {from:?}")
    }
}

#[cfg(test)]
mod tests {
    use crate::peers::{PeerRef, SessionId};

    use super::*;
    use bytes::Bytes;
    use ethereum_types::H512;
    use rtrb::RingBuffer;
    use rustaxa_types::ethereum::NodeId;
    use std::sync::Arc;

    fn test_packet() -> Packet {
        Packet::new(
            crate::packet::PacketType::StatusPacket,
            PeerRef::new(NodeId(H512::from([1u8; 64])), SessionId(1)),
            Bytes::from_static(b"status"),
        )
    }

    #[test]
    fn test_egress_creation() {
        let arena = Arc::new(Arena::<Packet>::new(1024).unwrap());
        let ringbuffer = RingBuffer::<NetworkEvent>::new(100);
        let registry = Arc::new(PeerRegistry::new());
        let _ = Egress::new(arena, ringbuffer.1, registry);
    }

    #[test]
    fn test_egress_shutdown_before_listen_is_valid() {
        let arena = Arc::new(Arena::<Packet>::new(1024).unwrap());
        let ringbuffer = RingBuffer::<NetworkEvent>::new(100);
        let registry = Arc::new(PeerRegistry::new());

        Egress::new(arena, ringbuffer.1, registry).shutdown();
    }

    #[test]
    fn test_egress_listen_then_shutdown() {
        let arena = Arc::new(Arena::<Packet>::new(1024).unwrap());
        let ringbuffer = RingBuffer::<NetworkEvent>::new(100);
        let registry = Arc::new(PeerRegistry::new());
        let mut egress = Egress::new(arena, ringbuffer.1, registry);

        egress.start();
        egress.shutdown();
    }

    #[test]
    fn test_processor_listen_returns_when_already_shutdown() {
        let arena = Arc::new(Arena::<Packet>::new(1024).unwrap());
        let ringbuffer = RingBuffer::<NetworkEvent>::new(100);
        let registry = Arc::new(PeerRegistry::new());
        let shutdown = Arc::new(AtomicBool::new(true));
        let mut processor = Processor::new(arena, ringbuffer.1, registry, shutdown);

        processor.listen();
    }

    #[test]
    fn test_processor_processes_existing_packet_event() {
        let arena = Arc::new(Arena::<Packet>::new(1024).unwrap());
        let ringbuffer = RingBuffer::<NetworkEvent>::new(100);
        let registry = Arc::new(PeerRegistry::new());
        let shutdown = Arc::new(AtomicBool::new(true));
        let processor = Processor::new(arena.clone(), ringbuffer.1, registry, shutdown);
        let reservation = arena.try_reserve().unwrap();
        let slot = arena.insert(reservation, test_packet()).unwrap();

        processor.process(NetworkEvent { slot });

        assert!(arena.borrow(slot).is_ok());
    }
}
