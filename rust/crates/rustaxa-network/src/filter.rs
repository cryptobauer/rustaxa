//! Packet filtering helpers for the ingress path.

use crate::peers::PeerRegistry;

#[derive(Debug)]
pub enum Flag {
    PeerDisconnected,
}

pub trait PacketFilter {
    fn peer_connected(&self, registry: &PeerRegistry) -> Result<bool, anyhow::Error>;
}
