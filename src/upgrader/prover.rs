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
        let mut task_guard = self.current_task.write().await;

        if let Some(task) = task_guard.as_ref() {
            if task.handle.is_finished() {
                let finished = task_guard.take().unwrap();
                let _ = finished.handle.await;
            }
        }

        if let Some(task) = task_guard.as_ref() {
            let still_valid = {
                let txs = self.tasks.read().await;
                txs.has(&task.transaction_id)
            };

            if !still_valid {
                info!(
                    "Cancelling upgrading of transaction {}...",
                    task.transaction_id
                );

                task.handle.abort();
                task_guard.take();
            }

            return;
        }

        let candidate = {
            let txs = self.tasks.read().await;
            txs.best_candidate().cloned()
        };
        let Some(tx_task) = candidate else {
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
            info!(
                "JSON dump of upgraded tx: \n{}",
                serde_json::to_string_pretty(&upgraded_tx).unwrap()
            );

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

        *task_guard = Some(ProverTask {
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
