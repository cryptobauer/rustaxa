//! Network facade for bounded packet ingress.
//!
//! This module owns the producer side of the network ingress queue. Incoming
//! packets are first stored in the shared packet arena and then published to the
//! ingress worker as lightweight slot events. If publishing fails, the packet is
//! removed from the arena before the error is returned so rejected packets do not
//! consume arena capacity.

use std::sync::Arc;

use anyhow::Error;
use rtrb::{Producer, PushError, RingBuffer};
use rustaxa_arena::arena::{Arena, BorrowError, InsertError, TryReserveError};
use thiserror::Error;

use crate::{events::NetworkEvent, ingress::Ingress, packet::Packet};

/// Bounded ingress pipeline entry point for network packets.
///
/// `Network` coordinates two resources shared by the ingress path:
///
/// - an [`Arena`] that owns packet bytes and metadata in fixed slots;
/// - an `rtrb` queue that publishes [`NetworkEvent`] slot handles to the
///   ingress worker.
///
/// Producers call [`Network::ingest`] from the C++ bridge or future Rust network
/// I/O. Consumers are owned by [`Ingress`] and are started with [`Network::listen`].
pub struct Network {
    arena: Arc<Arena<Packet>>,
    ingress: Ingress,
    producer: Producer<NetworkEvent>,
}

impl Network {
    /// Creates a network ingress pipeline over an existing packet arena.
    ///
    /// The arena is supplied by the embedding layer so packet storage can be
    /// shared across bridge boundaries. `config.queue_size` controls the number
    /// of slot events that can wait for the ingress worker.
    pub fn new(arena: Arc<Arena<Packet>>, config: NetworkConfig) -> Result<Self, Error> {
        let queue = RingBuffer::<NetworkEvent>::new(config.queue_size);
        let ingress = Ingress::new(arena.clone(), queue.1);

        Ok(Network {
            arena,
            ingress,
            producer: queue.0,
        })
    }

    /// Stores `packet` and publishes its slot to the ingress worker.
    ///
    /// Returns `Ok(true)` once the packet slot has been enqueued. If the event
    /// queue is full, the freshly inserted packet is removed from the arena and
    /// [`IngestPacketError::QueueFullError`] is returned.
    pub fn ingest(&mut self, packet: Packet) -> Result<bool, IngestPacketError> {
        // TODO long-term the packet should be written as low as possible (linux network)
        let reservation = self.arena.try_reserve()?;
        let slot = self.arena.insert(reservation, packet)?;
        match self.producer.push(NetworkEvent { slot }) {
            Err(PushError::Full(_)) => {
                self.arena.remove(slot)?;
                Err(IngestPacketError::QueueFullError)
            }
            Ok(_) => Ok(true),
        }
    }

    /// Starts the ingress worker thread.
    pub fn listen(&mut self) {
        self.ingress.listen();
    }

    /// Requests worker shutdown and waits for started workers to exit.
    pub fn shutdown(self) {
        self.ingress.shutdown();
    }
}

/// Runtime settings for the network ingress facade.
pub struct NetworkConfig {
    /// Number of network events that may wait in the ingress queue.
    pub queue_size: usize,
}

/// Error returned when ingesting a packet fails.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum IngestPacketError {
    /// The network module was not able to store the packet.
    #[error("arena storage reserve error encountered")]
    ArenaReserveError(#[from] TryReserveError),

    /// The network module was not able to store the packet.
    #[error("arena storage insert error encountered")]
    ArenaInsertError(#[from] InsertError),

    /// The network module was not able to store the packet.
    #[error("arena storage borrow error encountered")]
    ArenaBorrowError(#[from] BorrowError),

    /// The network module was not able to ingest another packet.
    #[error("queue push error encountered (queue full)")]
    QueueFullError,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethereum_types::H512;
    use rustaxa_types::ethereum::NodeId;

    fn test_packet(byte: u8) -> Packet {
        let mut node_id = [0u8; 64];
        node_id[63] = byte;

        Packet::new(
            crate::packet::PacketType::DagBlockPacket,
            NodeId(H512::from(node_id)),
            Bytes::from(vec![byte; 4]),
        )
    }

    #[test]
    fn test_ingest_stores_packet_in_arena() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let mut network = Network::new(arena.clone(), NetworkConfig { queue_size: 2 })
            .expect("network should be created");

        assert_eq!(network.ingest(test_packet(1)), Ok(true));
        assert!(
            matches!(
                arena.try_reserve(),
                Err(TryReserveError::AttemptsExceeded { .. })
            ),
            "accepted packet should occupy the only arena slot"
        );
    }

    #[test]
    fn test_ingest_reports_arena_reserve_error() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let mut network = Network::new(arena, NetworkConfig { queue_size: 2 })
            .expect("network should be created");

        assert_eq!(network.ingest(test_packet(1)), Ok(true));
        assert!(matches!(
            network.ingest(test_packet(2)),
            Err(IngestPacketError::ArenaReserveError(
                TryReserveError::AttemptsExceeded { .. }
            ))
        ));
    }

    #[test]
    fn test_ingest_removes_packet_when_queue_is_full() {
        let arena = Arc::new(Arena::new(1024).unwrap());
        let mut network = Network::new(arena, NetworkConfig { queue_size: 1 })
            .expect("network should be created");

        assert_eq!(network.ingest(test_packet(1)), Ok(true));
        assert_eq!(
            network.ingest(test_packet(2)),
            Err(IngestPacketError::QueueFullError)
        );

        let reservation = network
            .arena
            .try_reserve()
            .expect("queue-full rollback should free the rejected packet slot");
        drop(reservation);
    }
}
