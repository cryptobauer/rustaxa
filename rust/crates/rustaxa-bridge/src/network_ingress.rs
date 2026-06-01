use crate::ffi::{BridgeNetwork, BridgePacketArena};
use bytes::Bytes;
use num_enum::TryFromPrimitive;
use rustaxa_network::{
    network::{Network, NetworkConfig},
    packet::{Packet, PacketType},
};
use rustaxa_types::ethereum::NodeId;

pub fn create_network(
    arena: &BridgePacketArena,
    queue_size: usize,
) -> Result<Box<BridgeNetwork>, anyhow::Error> {
    let config = NetworkConfig { queue_size };
    let network = Network::new(arena.0.clone(), config)?;
    Ok(Box::new(BridgeNetwork(network)))
}

impl BridgeNetwork {
    pub fn ingest_network_packet(
        self: &mut BridgeNetwork,
        packet_type: u8,
        from_node: [u8; 64],
        data: Vec<u8>,
    ) -> Result<bool, anyhow::Error> {
        let packet = Packet::new(
            PacketType::try_from_primitive(packet_type)?,
            NodeId::new(from_node),
            Bytes::from(data),
        );
        Ok(self.0.ingest(packet)?)
    }
}
