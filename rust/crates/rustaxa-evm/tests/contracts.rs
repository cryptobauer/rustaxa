//! Unlinked composition through the existing FinalChain execution port.
//!
//! This checks ownership and identity preflight only. A matching root is not
//! bootstrap authorization, and this descriptor-only reader cannot execute or
//! publish. S2/S4 replace it with qualified state and persisted execution tests.

use anyhow::{Result, bail, ensure};
use rustaxa_consensus::final_chain_execution::FinalChainExecutionLeaf;
use rustaxa_consensus::{
    ConsensusExecutionPort, FinalChain, FinalChainExternalEvmCommittedStateDescriptor,
    FinalChainExternalEvmPreflightReport, FinalChainExternalEvmPreflightRequest,
    FinalChainExternalEvmStateCommitIntent, PillarAnchorStateReport, PillarAnchorStateRequest,
};
use rustaxa_storage::{Config, Storage};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};
use rustaxa_types::{FinalChainBlockNumber, StoredFinalChainBlockHeader};
use std::sync::Arc;

/// A descriptor carries no proof of row retention; all data access fails closed.
struct DescriptorOnlyReader(ConcreteStateIdentity);

impl ConcreteStateRead for DescriptorOnlyReader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.0
    }

    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        Err(ConcreteReadError::HistoryUnavailable(self.0))
    }

    fn storage(
        &self,
        _: [u8; 20],
        _: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Err(ConcreteReadError::HistoryUnavailable(self.0))
    }

    fn code(&self, _: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Err(ConcreteReadError::HistoryUnavailable(self.0))
    }
}

/// Borrows existing owners, with no parallel chain or publication state.
struct PreflightAdapter<'a, R> {
    chain: &'a FinalChain,
    storage: &'a Storage,
    reader: &'a R,
}

impl<R: ConcreteStateRead> ConsensusExecutionPort for PreflightAdapter<'_, R> {
    fn load_final_chain_committed_state(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport> {
        let period = self.chain.last_block_number_typed()?;
        let raw = self
            .storage
            .final_chain()
            .block_header_raw(period.as_u64())?
            .ok_or_else(|| anyhow::anyhow!("missing published header"))?;
        let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw))?;
        let expected = ConcreteStateIdentity {
            period,
            state_root: header.state_root.0,
        };
        let observed = self.reader.identity();
        if expected != observed {
            return Err(ConcreteReadError::IdentityMismatch { expected, observed }.into());
        }
        ensure!(
            request.expected_prior.period == period,
            "stale request period"
        );
        ensure!(
            request.expected_prior.state_root == expected.state_root,
            "stale request root"
        );
        ensure!(
            period.checked_next() == Some(request.next_period),
            "nonconsecutive request"
        );
        ensure!(
            request.concrete_chain_identity == self.chain.concrete_chain_identity()?,
            "chain configuration identity mismatch"
        );
        // Equality is read evidence only. Never manufacture lifecycle rows or
        // claim bootstrap approval for markerless imported state.
        Ok(FinalChainExternalEvmPreflightReport {
            request_id: request.request_id,
            // This rejected descriptor-only observation has no StateAPI owner.
            state_api_epoch: 0,
            committed: request.expected_prior,
            concrete_provenance_rlp: vec![],
            pending_concrete_marker_rlp: vec![],
            succeeded: false,
            error_code: "TEST_COMPOSITION_BOOTSTRAP_UNQUALIFIED".into(),
        })
    }

    fn load_pillar_anchor_state(
        &self,
        _: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport> {
        bail!("TEST_COMPOSITION_PILLAR_UNAVAILABLE")
    }
}

#[test]
fn existing_final_chain_port_checks_pair_without_adopting_or_publishing() -> Result<()> {
    let path = std::env::temp_dir().join(format!("rustaxa-evm-s1-{}", std::process::id()));
    // Never remove an unknown pre-existing directory on test entry.
    std::fs::create_dir(&path)?;
    {
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        // Fresh synthetic application identity, never applied to imported data.
        storage.metadata().set_genesis_hash_if_empty(&[7; 32])?;
        let chain = FinalChain::new(
            storage.clone(),
            1_000_000.into(),
            0,
            vec![],
            vec![],
            Default::default(),
        )?;
        let before = chain.block_header(FinalChainBlockNumber::GENESIS)?;
        let raw = storage.final_chain().block_header_raw(0)?.unwrap();
        let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw))?;
        let identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: header.state_root.0,
        };
        let reader = DescriptorOnlyReader(identity);
        let adapter = PreflightAdapter {
            chain: &chain,
            storage: &storage,
            reader: &reader,
        };
        let request = FinalChainExternalEvmPreflightRequest {
            request_id: [1; 32],
            next_period: 1.into(),
            expected_prior: FinalChainExternalEvmCommittedStateDescriptor {
                period: identity.period,
                state_root: identity.state_root,
            },
            concrete_chain_identity: chain.concrete_chain_identity()?,
        };
        // Exercise the existing blanket application-port -> leaf composition.
        let report = adapter.load_committed_state_descriptor(&request)?;
        assert_eq!(report.request_id, request.request_id);
        assert_eq!(report.state_api_epoch, 0);
        assert_eq!(report.committed, request.expected_prior);
        assert!(!report.succeeded);
        assert_eq!(report.error_code, "TEST_COMPOSITION_BOOTSTRAP_UNQUALIFIED");
        assert!(report.concrete_provenance_rlp.is_empty());
        assert!(
            adapter
                .commit_final_chain_state(&FinalChainExternalEvmStateCommitIntent::default())
                .is_err()
        );
        assert!(reader.account([0; 20]).is_err());
        let mut wrong_config = request;
        wrong_config.concrete_chain_identity = [0; 32];
        assert!(
            adapter
                .load_committed_state_descriptor(&wrong_config)
                .is_err()
        );
        for bad in [
            ConcreteStateIdentity {
                period: 1.into(),
                ..identity
            },
            ConcreteStateIdentity {
                state_root: [0; 32],
                ..identity
            },
        ] {
            let wrong_reader = DescriptorOnlyReader(bad);
            let wrong = PreflightAdapter {
                chain: &chain,
                storage: &storage,
                reader: &wrong_reader,
            };
            let error = wrong.load_committed_state_descriptor(&request).unwrap_err();
            assert!(matches!(
                error.downcast_ref::<ConcreteReadError>(),
                Some(ConcreteReadError::IdentityMismatch { .. })
            ));
        }
        assert_eq!(
            chain.last_block_number_typed()?,
            FinalChainBlockNumber::GENESIS
        );
        assert_eq!(chain.block_header(FinalChainBlockNumber::GENESIS)?, before);
    }
    std::fs::remove_dir_all(path)?;
    Ok(())
}
