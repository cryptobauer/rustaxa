//! Peer registry and session state for network ingress.
//!
//! The registry keeps long-lived peer records separate from active sessions.
//! A peer may be known to the node without being connected, while an active
//! session represents one currently accepted connection for that peer. The
//! network facade uses [`PeerRef`] values to attach both peer identity and
//! session identity to packets as they enter the ingress queue.

use chrono::Utc;
use rustaxa_types::ethereum::NodeId;
use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{Arc, RwLock},
};
use thiserror::Error;

/// Registry of known peers and currently connected peer sessions.
pub struct PeerRegistry {
    registry: RwLock<HashMap<NodeId, Arc<PeerRecord>>>,
    connected: RwLock<HashMap<NodeId, Arc<PeerSession>>>,
}

impl Default for PeerRegistry {
    /// Creates an empty peer registry.
    fn default() -> Self {
        Self::new()
    }
}

impl PeerRegistry {
    /// Creates an empty registry with no known or connected peers.
    pub fn new() -> Self {
        Self {
            registry: RwLock::new(HashMap::new()),
            connected: RwLock::new(HashMap::new()),
        }
    }

    /// Creates or activates a session for `node`.
    ///
    /// The peer record is created on first contact and reused by later
    /// sessions. Only one active session is allowed for a node at a time.
    pub fn connect(&self, node: NodeId) -> Result<PeerRef, PeerRegistryError> {
        let record = self.get_or_register(node)?;

        match self
            .connected
            .write()
            .map_err(|_| PeerRegistryError::PoisonedLock)?
            .entry(node)
        {
            Entry::Occupied(_) => Err(PeerRegistryError::PeerAlreadyConnected { peer: node }),
            Entry::Vacant(entry) => {
                if record.malicious {
                    return Err(PeerRegistryError::MaliciousPeer { peer: node });
                }

                let session = PeerSession {
                    id: SessionId(Utc::now().timestamp_millis() as u64),
                    node,
                    pending: true,
                    malicious: false,
                    record,
                };

                let v = entry.insert(Arc::new(session));

                Ok(PeerRef {
                    node,
                    session: v.id.clone(),
                })
            }
        }
    }

    /// Returns the existing peer record or creates a clean record for `node`.
    fn get_or_register(&self, node: NodeId) -> Result<Arc<PeerRecord>, PeerRegistryError> {
        match self
            .registry
            .write()
            .map_err(|_| PeerRegistryError::PoisonedLock)?
            .entry(node)
        {
            Entry::Occupied(v) => Ok(v.get().clone()),
            Entry::Vacant(entry) => {
                let record = PeerRecord {
                    node,
                    malicious: false,
                    strikes: 0,
                };
                let v = entry.insert(Arc::new(record));
                Ok(v.clone())
            }
        }
    }

    /// Removes the active session for `node`.
    pub fn disconnect(&self, node: NodeId) -> Result<Option<Arc<PeerSession>>, PeerRegistryError> {
        match self
            .connected
            .write()
            .map_err(|_| PeerRegistryError::PoisonedLock)?
            .remove(&node)
        {
            Some(v) => Ok(Some(v)),
            None => Err(PeerRegistryError::DisconnectedPeer { peer: node }),
        }
    }

    /// Returns the active session reference for `node`, if the peer is connected.
    pub fn connected(&self, node: NodeId) -> Result<Option<PeerRef>, PeerRegistryError> {
        Ok(self
            .connected
            .read()
            .map_err(|_| PeerRegistryError::PoisonedLock)?
            .get(&node)
            .map(|v| PeerRef {
                node,
                session: v.id.clone(),
            }))
    }

    /// Returns the persistent record for a known peer.
    pub fn record(&self, node: NodeId) -> Result<Arc<PeerRecord>, PeerRegistryError> {
        match self
            .registry
            .read()
            .map_err(|_| PeerRegistryError::PoisonedLock)?
            .get(&node)
        {
            Some(v) => Ok(v.clone()),
            None => Err(PeerRegistryError::UnknownPeer { peer: node }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Reference to one peer session.
pub struct PeerRef {
    /// Remote node identity.
    pub node: NodeId,
    /// Connection generation for this peer reference.
    pub session: SessionId,
}

impl PeerRef {
    /// Creates a peer/session reference for packet attribution.
    pub fn new(node: NodeId, session: SessionId) -> Self {
        Self { node, session }
    }
}

/// State associated with one active connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSession {
    /// Unique id for this active peer session.
    pub id: SessionId,

    /// Node id for the connected peer.
    /// Remote node identity.
    pub node: NodeId,

    /// Whether the connection is still waiting for full handshake acceptance.
    pub pending: bool,

    /// Whether this active session has been classified as malicious.
    pub malicious: bool,

    /// Persistent peer record shared with other sessions over time.
    pub record: Arc<PeerRecord>,
}

/// Long-lived peer state retained across connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRecord {
    /// Node id for the known peer.
    /// Remote node identity.
    pub node: NodeId,

    /// Whether this peer is barred from connecting.
    pub malicious: bool,

    /// Number of recorded protocol or behavior violations.
    pub strikes: u32,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
/// Errors returned by peer registry operations.
pub enum PeerRegistryError {
    #[error("attempt to connect already connected peer {peer:?}")]
    /// The peer already has an active session.
    PeerAlreadyConnected {
        /// Node id for the peer that is already connected.
        peer: NodeId,
    },

    #[error("attempt operation on disconnected peer {peer:?}")]
    /// The operation requires an active session, but the peer is disconnected.
    DisconnectedPeer {
        /// Node id for the disconnected peer.
        peer: NodeId,
    },

    #[error("unknown peer {peer:?}")]
    /// The peer has no persistent record.
    UnknownPeer {
        /// Node id for the unknown peer.
        peer: NodeId,
    },

    #[error("unknown peer {peer:?}")]
    /// The peer is known but marked malicious.
    MaliciousPeer {
        /// Node id for the malicious peer.
        peer: NodeId,
    },

    #[error("session mismatch want {want:?}, but current session {current_session:?} ")]
    /// A packet or operation referenced a stale peer session.
    SessionMismatch {
        /// Peer/session reference supplied by the caller.
        want: PeerRef,

        /// Current active session id for the peer.
        current_session: SessionId,
    },

    #[error("poisoned lock encountered")]
    /// Registry state could not be accessed because a lock was poisoned.
    PoisonedLock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Identifier for one peer connection session.
pub struct SessionId(
    /// Numeric session id.
    pub u64,
);

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H512;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn test_node(byte: u8) -> NodeId {
        let mut bytes = [0u8; 64];
        bytes[63] = byte;
        NodeId(H512::from(bytes))
    }

    #[test]
    fn test_peer_registry_default_is_empty() {
        let registry = PeerRegistry::default();
        let peer = test_node(1);

        assert_eq!(registry.connected(peer), Ok(None));
        assert_eq!(
            registry.record(peer),
            Err(PeerRegistryError::UnknownPeer { peer })
        );
    }

    #[test]
    fn test_connect_registers_peer_and_active_session() {
        let registry = PeerRegistry::new();
        let peer = test_node(2);

        let peer_ref = registry.connect(peer).expect("peer should connect");
        let connected = registry
            .connected(peer)
            .expect("connected lookup should succeed")
            .expect("peer should be connected");
        let record = registry.record(peer).expect("peer record should exist");

        assert_eq!(connected, peer_ref);
        assert_eq!(record.node, peer);
        assert!(!record.malicious);
        assert_eq!(record.strikes, 0);
    }

    #[test]
    fn test_connect_rejects_already_connected_peer() {
        let registry = PeerRegistry::new();
        let peer = test_node(3);

        assert!(registry.connect(peer).is_ok());
        assert_eq!(
            registry.connect(peer),
            Err(PeerRegistryError::PeerAlreadyConnected { peer })
        );
    }

    #[test]
    fn test_disconnect_removes_active_session_but_keeps_record() {
        let registry = PeerRegistry::new();
        let peer = test_node(4);
        let peer_ref = registry.connect(peer).expect("peer should connect");

        let session = registry
            .disconnect(peer)
            .expect("disconnect should succeed")
            .expect("session should be returned");

        assert_eq!(session.node, peer);
        assert_eq!(session.id, peer_ref.session);
        assert_eq!(registry.connected(peer), Ok(None));
        assert_eq!(registry.record(peer).unwrap().node, peer);
    }

    #[test]
    fn test_disconnect_rejects_disconnected_peer() {
        let registry = PeerRegistry::new();
        let peer = test_node(5);

        assert_eq!(
            registry.disconnect(peer),
            Err(PeerRegistryError::DisconnectedPeer { peer })
        );
    }

    #[test]
    fn test_connect_rejects_malicious_record() {
        let registry = PeerRegistry::new();
        let peer = test_node(6);
        registry.registry.write().unwrap().insert(
            peer,
            Arc::new(PeerRecord {
                node: peer,
                malicious: true,
                strikes: 1,
            }),
        );

        assert_eq!(
            registry.connect(peer),
            Err(PeerRegistryError::MaliciousPeer { peer })
        );
        assert_eq!(registry.connected(peer), Ok(None));
    }

    #[test]
    fn test_registry_lock_poisoning_is_reported() {
        let registry = PeerRegistry::new();
        let peer = test_node(7);

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = registry.registry.write().unwrap();
            panic!("poison registry lock");
        }));

        assert_eq!(registry.connect(peer), Err(PeerRegistryError::PoisonedLock));
        assert_eq!(registry.record(peer), Err(PeerRegistryError::PoisonedLock));
    }

    #[test]
    fn test_connected_lock_poisoning_is_reported() {
        let registry = PeerRegistry::new();
        let peer = test_node(8);

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = registry.connected.write().unwrap();
            panic!("poison connected lock");
        }));

        assert_eq!(
            registry.connected(peer),
            Err(PeerRegistryError::PoisonedLock)
        );
        assert_eq!(
            registry.disconnect(peer),
            Err(PeerRegistryError::PoisonedLock)
        );
    }
}
