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
    pub fn start_network(self: &mut BridgeNetwork) -> Result<(), anyhow::Error> {
        self.0.start();
        Ok(())
    }

    pub fn connect_peer(self: &mut BridgeNetwork, node: [u8; 64]) -> Result<bool, anyhow::Error> {
        match self.0.connect(NodeId::new(node)) {
            Err(_) => Ok(false),
            Ok(_) => Ok(true),
        }
    }

    pub fn disconnect_peer(self: &mut BridgeNetwork, node: [u8; 64]) -> Result<(), anyhow::Error> {
        self.0.disconnect(NodeId::new(node))?;
        Ok(())
    }

    pub fn queue_is_full(self: &BridgeNetwork) -> bool {
        self.0.full()
    }

    pub fn ingest_network_packet(
        self: &mut BridgeNetwork,
        packet_type: u8,
        from_node: [u8; 64],
        data: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        let node = NodeId::new(from_node);

        if let Some(peer) = self.0.connected(node)? {
            let packet = Packet::new(
                PacketType::try_from_primitive(packet_type)?,
                peer,
                Bytes::from(data),
            );

            self.0.ingest(packet)?
        }

        Ok(())
    }
}
