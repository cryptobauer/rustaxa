//! Events passed between network ingress pipeline stages.

use rustaxa_arena::arena::SlotId;

/// Event notifying the ingress worker that a packet slot is ready.
pub struct NetworkEvent {
    /// Slot id for a packet stored in the shared packet arena.
    pub slot: SlotId,
}

/// Event notifying a DAG ingress stage that a packet is ready to process.
pub struct DagEvent {
    slot: SlotId,
}

impl DagEvent {
    /// Creates a DAG ingress event for a packet stored in the arena.
    pub fn new(slot: SlotId) -> Self {
        Self { slot }
    }

    /// Returns the arena slot id for the packet to process.
    pub fn packet_id(&self) -> SlotId {
        self.slot
    }
}
