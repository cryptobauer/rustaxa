use std::sync::Arc;
use std::time;

use rustaxa_arena::arena::Arena;
use rustaxa_network::network::{Network, NetworkConfig};
use rustaxa_network::packet::{Packet, PacketType};
use rustaxa_network::peers::{PeerRef, SessionId};
use rustaxa_types::ethereum::NodeId;

fn main() {
    let arena = Arc::new(Arena::new(1024).unwrap());
    let mut network = Network::new(arena, NetworkConfig { queue_size: 100 }).unwrap();

    network.start();

    let packet1 = Packet::default();
    let packet2 = Packet::new(
        PacketType::DagBlockPacket,
        PeerRef::new(NodeId::new([1u8; 64]), SessionId(1)),
        bytes::Bytes::new(),
    );

    let _ = network.ingest(packet1.clone());
    let _ = network.ingest(packet2.clone());
    let _ = network.ingest(packet2.clone());
    let _ = network.ingest(packet1.clone());
    let _ = network.ingest(packet1.clone());
    let _ = network.ingest(packet2.clone());

    std::thread::sleep(time::Duration::from_secs(1));
}
