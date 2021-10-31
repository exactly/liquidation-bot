use crate::bindings::terminator::Terminator;
use crate::path_finder::service::TradePath;
use ethers::abi::ethereum_types::H256;
use ethers::abi::Address;
use ethers::prelude::{Middleware, Signer, SignerMiddleware, TransactionReceipt, U256};

#[derive(Debug)]
pub struct TerminatorJob {
    pub(crate) credit_manager: Address,
    pub(crate) borrower: Address,
    pub(crate) router: Address,
    pub(crate) paths: Vec<(
        ethers_core::types::U256,
        Vec<ethers_core::types::Address>,
        ethers_core::types::U256,
    )>,
    pub repay_amount: U256,
    pub underlying_token: Address,
}

pub struct TerminatorService<M: Middleware, S: Signer> {
    contract: Terminator<SignerMiddleware<M, S>>,
}

impl<M: Middleware, S: Signer> TerminatorService<M, S> {
    pub async fn new(address: &Address, client: std::sync::Arc<SignerMiddleware<M, S>>) -> Self {
        let contract = Terminator::new(*address, client.clone());

        let is_executor = contract.executors(client.address()).call().await.unwrap();

        if !is_executor {
            let tx = contract
                .allow_executor(client.address())
                .send()
                .await
                .unwrap()
                .await
                .unwrap()
                .unwrap();

            println!("Allow executor {}", tx.transaction_hash);
        } else {
            println!("Executor {} is already allowed", &client.address())
        }

        TerminatorService { contract }
    }

    pub async fn liquidate(&mut self, job: &TerminatorJob) -> TransactionReceipt {
        dbg!(&job);
        println!("Length: {}", &job.paths.len());

        self.contract
            .liquidate_and_sell_on_v2(
                job.credit_manager,
                job.borrower,
                job.router,
                job.paths.clone(),
            )
            .send()
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap()
    }
}