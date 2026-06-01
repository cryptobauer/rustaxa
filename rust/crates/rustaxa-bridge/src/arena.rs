use std::sync::Arc;

use crate::ffi::BridgePacketArena;
use rustaxa_arena::arena::Arena;

pub fn create_packet_arena(size: usize) -> Result<Box<BridgePacketArena>, anyhow::Error> {
    Ok(Box::new(BridgePacketArena(Arc::new(Arena::new(size)?))))
}
