use std::{sync::Arc, time::Duration};

use neptune_privacy::application::json_rpc::core::{
    api::rpc::RpcApi,
    model::wallet::transaction::{RpcTransaction, RpcTransactionProof},
};
use tokio::sync::RwLock;
use tracing::{info, warn};
use xnt_rpc_client::http::HttpClient;

use crate::upgrader::{prover::Prover, tasks::Tasks};

#[derive(Clone)]
pub struct Upgrader {
    client: HttpClient,
    txs: Arc<RwLock<Tasks>>,
    prover: Arc<Prover>,
}

impl Upgrader {
    pub fn new(client: HttpClient) -> Self {
        let txs = Arc::new(RwLock::new(Tasks::new()));

        Upgrader {
            client: client.clone(),
            txs: txs.clone(),
            prover: Arc::new(Prover::new(client, txs)),
        }
    }

    pub async fn main_loop(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            interval.tick().await;
            self.scan_mempool().await;
            self.prover.check_jobs().await;
        }
    }

    pub async fn scan_mempool(&self) {
        let tip = self.client.tip_kernel().await.unwrap().kernel;
        let tx_ids = self.client.transactions().await.unwrap().transactions;
        let mut tasks_guard = self.txs.write().await;

        for id in tx_ids {
            let proof = self.client.get_transaction_proof(id).await.unwrap().proof;
            let Some(proof) = proof else {
                warn!("Proof of transaction {id} not found.");
                continue;
            };

            match &proof {
                RpcTransactionProof::ProofCollection(_) => {
                    let kernel = self.client.get_transaction_kernel(id).await.unwrap().kernel;
                    let Some(kernel) = kernel else {
                        continue;
                    };

                    let transaction = RpcTransaction { kernel, proof };
                    tasks_guard.record(id, transaction, tip.clone());
                }
                RpcTransactionProof::SingleProof(_) => {
                    tasks_guard.forget(&id);
                }
            }
        }
    }
}
