//! Network facade for bounded packet ingress.
//!
//! This module owns the producer side of the network ingress queue. Incoming
//! packets are first stored in the shared packet arena and then published to the
//! ingress worker as lightweight slot events. If publishing fails, the packet is
//! removed from the arena before the error is returned so rejected packets do not
//! consume arena capacity.

use anyhow::Error;
use rtrb::{Producer, PushError, RingBuffer};
use rustaxa_arena::arena::{Arena, BorrowError, InsertError, TryReserveError};
use rustaxa_types::ethereum::NodeId;
use std::sync::Arc;
use thiserror::Error;

use crate::{
    events::NetworkEvent,
    ingress::Ingress,
    packet::Packet,
    peers::{PeerRef, PeerRegistry, PeerRegistryError, PeerSession},
};

/// Bounded ingress pipeline entry point for network packets.
///
/// `Network` coordinates two resources shared by the ingress path:
///
/// - an [`Arena`] that owns packet bytes and metadata in fixed slots;
/// - an `rtrb` queue that publishes [`NetworkEvent`] slot handles to the
///   ingress worker.
///
/// Producers call [`Network::ingest`] from the C++ bridge or future Rust network
/// I/O. Consumers are owned by [`Ingress`] and are started with [`Network::start`].
pub struct Network {
    arena: Arc<Arena<Packet>>,
    ingress: Ingress,
    producer: Producer<NetworkEvent>,
    registry: Arc<PeerRegistry>,
    started: bool,
}

impl Network {
    /// Creates a network ingress pipeline over an existing packet arena.
    ///
    /// The arena is supplied by the embedding layer so packet storage can be
    /// shared across bridge boundaries. `config.queue_size` controls the number
    /// of slot events that can wait for the ingress worker.
    pub fn new(arena: Arc<Arena<Packet>>, config: NetworkConfig) -> Result<Self, Error> {
        let queue = RingBuffer::<NetworkEvent>::new(config.queue_size);
        let registry = Arc::new(PeerRegistry::new());
        let ingress = Ingress::new(arena.clone(), queue.1, registry.clone());

        Ok(Network {
            arena,
            ingress,
            producer: queue.0,
            registry,
            started: false,
        })
    }

    /// Registers an active peer session when the ingress queue can accept work.
    ///
    /// New peer connections are rejected while the ingress queue is full so the
    /// node does not accept more producers when packet processing is already
    /// backed up.
    pub fn connect(&self, node: NodeId) -> Result<PeerRef, PeerHandlingError> {
        if self.producer.is_full() {
            return Err(PeerHandlingError::QueueFullRejectPeer);
        }

        Ok(self.registry.connect(node)?)
    }

    /// Returns the active peer/session reference for `node`, if one exists.
    pub fn connected(&self, node: NodeId) -> Result<Option<PeerRef>, PeerHandlingError> {
        Ok(self.registry.connected(node)?)
    }

    /// Removes the active peer session for `node`.
    pub fn disconnect(&self, node: NodeId) -> Result<Option<Arc<PeerSession>>, PeerHandlingError> {
        Ok(self.registry.disconnect(node)?)
    }

    /// Returns whether the ingress event queue is at capacity.
    pub fn full(&self) -> bool {
        self.producer.is_full()
    }

    /// Stores `packet` and publishes its slot to the ingress worker.
    ///
    /// Returns `Ok(())` once the packet slot has been enqueued. If the event
    /// queue is full, the newly inserted packet is removed from the arena and
    /// [`IngestPacketError::QueueFullError`] is returned.
    pub fn ingest(&mut self, packet: Packet) -> Result<(), IngestPacketError> {
        let reservation = self.arena.try_reserve()?;
        let slot = self.arena.insert(reservation, packet)?;
        match self.producer.push(NetworkEvent { slot }) {
            Err(PushError::Full(_)) => {
                self.arena.remove(slot)?;
                Err(IngestPacketError::QueueFullError)
            }
            Ok(_) => Ok(()),
        }
    }

    /// Starts the ingress worker thread.
    pub fn start(&mut self) {
        self.ingress.start();
        self.started = true;
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

/// Error returned when peer handling fails.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PeerHandlingError {
    #[error("peer registry yielded an error")]
    /// Peer registry operation failed.
    PeerRegistry(#[from] PeerRegistryError),

    #[error("queue full reject peer")]
    /// New peer was rejected because the ingress queue is full.
    QueueFullRejectPeer,
}

#[cfg(test)]
mod tests {
    use crate::peers::{PeerRegistryError, SessionId};

    use super::*;
    use bytes::Bytes;
    use ethereum_types::H512;
    use rustaxa_types::ethereum::NodeId;

    fn test_node(byte: u8) -> NodeId {
        let mut node_id = [0u8; 64];
        node_id[63] = byte;
        NodeId(H512::from(node_id))
    }

    fn test_packet(byte: u8) -> Packet {
        Packet::new(
            crate::packet::PacketType::DagBlockPacket,
            PeerRef::new(test_node(byte), SessionId(1)),
            Bytes::from(vec![byte; 4]),
        )
    }

    #[test]
    fn test_network_creation_starts_with_available_queue() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let network = Network::new(arena, NetworkConfig { queue_size: 1 })
            .expect("network should be created");

        assert!(!network.full());
    }

    #[test]
    fn test_network_start_then_shutdown() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let mut network = Network::new(arena, NetworkConfig { queue_size: 1 })
            .expect("network should be created");

        network.start();
        network.shutdown();
    }

    #[test]
    fn test_ingest_stores_packet_in_arena() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let mut network = Network::new(arena.clone(), NetworkConfig { queue_size: 2 })
            .expect("network should be created");

        assert_eq!(network.ingest(test_packet(1)), Ok(()));
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

        assert_eq!(network.ingest(test_packet(1)), Ok(()));
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

        assert_eq!(network.ingest(test_packet(1)), Ok(()));
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

    #[test]
    fn test_full_reports_queue_capacity() {
        let arena = Arc::new(Arena::new(1024).unwrap());
        let mut network = Network::new(arena, NetworkConfig { queue_size: 1 })
            .expect("network should be created");

        assert!(!network.full());
        assert_eq!(network.ingest(test_packet(1)), Ok(()));
        assert!(network.full());
    }

    #[test]
    fn test_connect_registers_peer_when_queue_has_capacity() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let network = Network::new(arena, NetworkConfig { queue_size: 1 })
            .expect("network should be created");
        let peer = test_node(2);

        let peer_ref = network.connect(peer).expect("peer should connect");

        assert_eq!(network.connected(peer), Ok(Some(peer_ref)));
    }

    #[test]
    fn test_connect_rejects_peer_when_queue_is_full() {
        let arena = Arc::new(Arena::new(1024).unwrap());
        let mut network = Network::new(arena, NetworkConfig { queue_size: 1 })
            .expect("network should be created");
        let peer = test_node(3);

        assert_eq!(network.ingest(test_packet(1)), Ok(()));

        assert_eq!(
            network.connect(peer),
            Err(PeerHandlingError::QueueFullRejectPeer)
        );
        assert_eq!(network.connected(peer), Ok(None));
    }

    #[test]
    fn test_connect_reports_registry_error_for_duplicate_peer() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let network = Network::new(arena, NetworkConfig { queue_size: 2 })
            .expect("network should be created");
        let peer = test_node(4);

        assert!(network.connect(peer).is_ok());

        assert_eq!(
            network.connect(peer),
            Err(PeerHandlingError::PeerRegistry(
                PeerRegistryError::PeerAlreadyConnected { peer }
            ))
        );
    }

    #[test]
    fn test_disconnect_removes_peer_session() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let network = Network::new(arena, NetworkConfig { queue_size: 2 })
            .expect("network should be created");
        let peer = test_node(5);

        assert!(network.connect(peer).is_ok());
        let session = network
            .disconnect(peer)
            .expect("disconnect should succeed")
            .expect("session should be returned");

        assert_eq!(session.node, peer);
        assert_eq!(network.connected(peer), Ok(None));
    }

    #[test]
    fn test_disconnect_reports_registry_error_for_disconnected_peer() {
        let arena = Arc::new(Arena::new(1).unwrap());
        let network = Network::new(arena, NetworkConfig { queue_size: 2 })
            .expect("network should be created");
        let peer = test_node(6);

        assert_eq!(
            network.disconnect(peer),
            Err(PeerHandlingError::PeerRegistry(
                PeerRegistryError::DisconnectedPeer { peer }
            ))
        );
    }
}
