use std::collections::HashMap;

use neptune_privacy::{
    application::json_rpc::core::model::{
        block::{RpcBlockKernel, transaction_kernel::RpcTransactionKernelId},
        wallet::transaction::{RpcTransaction, RpcTransactionProof},
    },
    protocol::consensus::transaction::{
        transaction_kernel::TransactionKernel, validity::proof_collection::ProofCollection,
    },
};
use tracing::info;

#[derive(Clone)]
pub struct TxTask {
    pub id: RpcTransactionKernelId,
    pub kernel: TransactionKernel,
    pub proof: ProofCollection,
    pub synced_block: RpcBlockKernel,
}

impl TxTask {
    pub fn new(
        id: RpcTransactionKernelId,
        transaction: RpcTransaction,
        synced_block: RpcBlockKernel,
    ) -> Self {
        let proof = match transaction.proof {
            RpcTransactionProof::ProofCollection(pc) => *pc,
            _ => panic!("Only proof collection txs can be upgraded"),
        };

        TxTask {
            id,
            kernel: transaction.kernel.into(),
            proof: proof.into(),
            synced_block,
        }
    }
}

#[derive(Clone)]
pub struct Tasks {
    pub transactions: HashMap<RpcTransactionKernelId, TxTask>,
}

impl Tasks {
    pub fn new() -> Self {
        Tasks {
            transactions: HashMap::new(),
        }
    }

    pub fn has(&self, id: &RpcTransactionKernelId) -> bool {
        self.transactions.contains_key(id)
    }

    pub fn best_candidate(&self) -> Option<&TxTask> {
        self.transactions.values().next()
    }

    pub fn record(
        &mut self,
        id: RpcTransactionKernelId,
        transaction: RpcTransaction,
        synced_block: RpcBlockKernel,
    ) {
        if let Some(old) = self.transactions.get(&id) {
            let old_height = old.synced_block.header.height;
            let new_height = synced_block.header.height;

            // Ignore older blocks
            if new_height < old_height {
                return;
            }

            // Same height, same block -> nothing to do
            if new_height == old_height && synced_block.header == old.synced_block.header {
                return;
            }
        }

        // New tx, higher height, or reorg at same height
        if self
            .transactions
            .insert(id, TxTask::new(id, transaction, synced_block))
            .is_none()
        {
            info!("Discovered transaction {}.", id);
        } else {
            info!("Updated transaction {}.", id);
        }
    }

    pub fn forget(&mut self, id: &RpcTransactionKernelId) {
        if self.transactions.remove(id).is_some() {
            info!("Transaction {} is no longer upgradable.", id);
        }
    }
}
