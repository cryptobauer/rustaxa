use rustaxa_arena::arena::SlotId;

/// Event notifying a DAG ingress stage that a packet is ready to process.
pub struct IncomingDagEvent {
    packet_id: SlotId,
}

impl IncomingDagEvent {
    /// Creates a DAG ingress event for a packet stored in the arena.
    pub fn new(packet_id: SlotId) -> Self {
        Self { packet_id }
    }

    /// Returns the arena slot id for the packet to process.
    pub fn packet_id(&self) -> SlotId {
        self.packet_id
    }
}
