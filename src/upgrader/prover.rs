use std::sync::Arc;

use neptune_privacy::application::json_rpc::core::model::block::transaction_kernel::RpcTransactionKernelId;
use neptune_privacy::prelude::triton_vm;
use neptune_privacy::protocol::proof_abstractions::tasm::program::ConsensusProgram;
use neptune_privacy::{
    api::export::NeptuneProof,
    application::json_rpc::core::{
        api::rpc::RpcApi,
        model::wallet::transaction::{RpcTransaction, RpcTransactionProof},
    },
    prelude::triton_vm::stark::Stark,
    protocol::{
        consensus::transaction::validity::single_proof::{SingleProof, SingleProofWitness},
        proof_abstractions::SecretWitness,
    },
};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info};
use xnt_rpc_client::http::HttpClient;

use crate::upgrader::tasks::{Tasks, TxTask};

struct ProverTask {
    transaction_id: RpcTransactionKernelId,
    handle: JoinHandle<()>,
}

#[derive(Clone)]
pub struct Prover {
    client: HttpClient,
    tasks: Arc<RwLock<Tasks>>,
    current_task: Arc<RwLock<Option<ProverTask>>>,
}

impl Prover {
    pub fn new(client: HttpClient, tasks: Arc<RwLock<Tasks>>) -> Self {
        Prover {
            client,
            tasks,
            current_task: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn check_jobs(&self) {
        self.cleanup_finished_task().await;

        if self.has_active_task().await {
            return;
        }

        self.start_next_task().await;
    }

    async fn cleanup_finished_task(&self) {
        let mut current = self.current_task.write().await;

        if let Some(task) = current.as_ref()
            && task.handle.is_finished()
        {
            let finished = current.take().unwrap();
            let _ = finished.handle.await;
        }
    }

    async fn has_active_task(&self) -> bool {
        let current = self.current_task.read().await;

        let Some(task) = current.as_ref() else {
            return false;
        };

        let transaction_id = task.transaction_id;
        let tasks = self.tasks.read().await;
        let is_valid = tasks.has(&transaction_id);

        if !is_valid {
            info!("Cancelling upgrading of transaction {}...", transaction_id);
            drop(tasks);
            drop(current);

            let mut current = self.current_task.write().await;
            if let Some(task) = current.take() {
                task.handle.abort();
            }
        }

        true
    }

    async fn start_next_task(&self) {
        let tasks = self.tasks.read().await;
        let Some(tx_task) = tasks.best_candidate().cloned() else {
            return;
        };

        let id = tx_task.id;
        info!("Upgrading transaction {}...", id);

        let client = self.client.clone();
        let handle = tokio::spawn(async move {
            let upgraded_tx = tokio::task::spawn_blocking(move || prove(tx_task))
                .await
                .ok()
                .unwrap();

            match client.submit_transaction(upgraded_tx).await {
                Ok(response) => {
                    info!(
                        "Submitted upgraded transaction {}: {}",
                        response.success, id
                    );
                }
                Err(e) => {
                    error!("Failed to submit upgraded transaction {}: {}", id, e);
                }
            }
        });

        let mut current = self.current_task.write().await;
        *current = Some(ProverTask {
            transaction_id: id,
            handle,
        });
    }
}

fn prove(transaction: TxTask) -> RpcTransaction {
    let single_proof_witness = SingleProofWitness::from_collection(transaction.proof);
    let claim = single_proof_witness.claim();
    let nondeterminism = single_proof_witness.nondeterminism();

    let proof = triton_vm::prove(
        Stark::default(),
        &claim,
        SingleProof.program(),
        nondeterminism,
    )
    .expect("Proving failed - transaction in cache should always be valid");
    let neptune_proof: NeptuneProof = proof.into();

    RpcTransaction {
        kernel: (&transaction.kernel).into(),
        proof: RpcTransactionProof::SingleProof(neptune_proof.into()),
    }
}
