use crate::ffi::rustaxa_ffi::ProposedBlockLookup;
use crate::ffi::BridgeApp;
use ethereum_types::H256;

impl BridgeApp {
    /// Publishes one proposed block effect from canonical signed block bytes.
    ///
    /// The native application root derives every identity field and preserves
    /// durable-before-live publication. The result is true only for a newly
    /// published proposal; malformed bytes and storage failures cross as bridge
    /// errors, while an existing proposal returns false after durable repair.
    pub fn pbft_service_publish_proposed_block_effect(
        &self,
        canonical_signed_block_rlp: Vec<u8>,
    ) -> Result<bool, anyhow::Error> {
        self.0
            .publish_proposed_block_effect(canonical_signed_block_rlp)
    }

    /// Publishes a proposed PBFT block through the native PBFT service.
    ///
    /// Storage is committed before live index mutation so failed writes or
    /// sidecar/RLP mismatches cannot leave memory ahead of durable state.
    /// Existing live entries return `false` after their durable row is
    /// overwritten. The native service holds one write guard across the
    /// unconditional storage write and live duplicate detection, preserving
    /// the legacy durability and repair ordering.
    pub fn pbft_service_publish_proposed_block(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        pivot_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> Result<bool, anyhow::Error> {
        self.0.publish_proposed_block(
            period,
            H256::from(*block_hash),
            H256::from(*pivot_hash),
            block_rlp,
        )
    }

    /// Loads one proposed PBFT block and its cached validation flag.
    ///
    /// The returned carrier owns canonical block bytes for compatibility
    /// materialization. Missing entries return `found = false` and do not mutate
    /// service state.
    pub fn pbft_service_proposed_blocks_get(
        &self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> ProposedBlockLookup {
        self.0
            .proposed_block(period, H256::from(*block_hash))
            .map(|entry| ProposedBlockLookup {
                found: true,
                is_valid: entry.is_valid,
                pivot_hash: entry.pivot_hash.into(),
                block_rlp: entry.block_rlp,
            })
            .unwrap_or(ProposedBlockLookup {
                found: false,
                is_valid: false,
                pivot_hash: [0; 32],
                block_rlp: Vec::new(),
            })
    }
}
