use std::collections::HashMap;
use std::sync::Arc;

use ethers::abi::Address;
use ethers::prelude::{abigen, Middleware, Signer, SignerMiddleware, U256};

use eyre::Result;

use super::Market;

const INTERVAL: u32 = 4 * 7 * 86_400;

abigen!(
    ERC20,
    "node_modules/@exactly-protocol/protocol/deployments/rinkeby/DAI.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

#[derive(Eq, PartialEq, Debug, Default)]
pub struct FixedPool {
    pub borrowed: U256,
    pub supplied: U256,
    pub unassigned_earnings: U256,
    pub last_accrual: U256,
}

pub struct FixedLender<M, S> {
    pub contract: Market<SignerMiddleware<M, S>>,
    pub oracle_price: U256,
    pub penalty_rate: U256,
    pub adjust_factor: U256,
    pub decimals: u8,
    pub smart_pool_assets: U256,
    pub total_shares: U256,
    pub max_future_pools: u8,
    pub fixed_pools: HashMap<u32, FixedPool>,
    pub smart_pool_fee_rate: U256,
    pub smart_pool_earnings_accumulator: U256,
    pub last_accumulated_earnings_accrual: U256,
    pub accumulated_earnings_smooth_factor: u128,
    pub price_feed: Address,
    pub listed: bool,
}

impl<M: 'static + Middleware, S: 'static + Signer> Eq for FixedLender<M, S> {}

impl<M: 'static + Middleware, S: 'static + Signer> PartialEq for FixedLender<M, S> {
    fn eq(&self, other: &Self) -> bool {
        (*self.contract).address() == (*other.contract).address()
    }
}

impl<M: 'static + Middleware, S: 'static + Signer> FixedLender<M, S> {
    pub fn new(address: Address, client: &Arc<SignerMiddleware<M, S>>) -> Self {
        Self {
            contract: Market::new(address, Arc::clone(client)),
            oracle_price: Default::default(),
            penalty_rate: Default::default(),
            adjust_factor: Default::default(),
            decimals: Default::default(),
            smart_pool_assets: Default::default(),
            total_shares: Default::default(),
            max_future_pools: Default::default(),
            fixed_pools: Default::default(),
            smart_pool_fee_rate: Default::default(),
            smart_pool_earnings_accumulator: Default::default(),
            last_accumulated_earnings_accrual: Default::default(),
            accumulated_earnings_smooth_factor: Default::default(),
            price_feed: Default::default(),
            listed: Default::default(),
        }
    }

    pub async fn approve_asset(&self, client: &Arc<SignerMiddleware<M, S>>) -> Result<()> {
        let asset_address = self.contract.asset().call().await?;
        let asset = ERC20::new(asset_address, Arc::clone(client));
        let allowance = asset
            .allowance(client.address(), self.contract.address())
            .call()
            .await?;
        if allowance < U256::MAX / 2u128 {
            let tx = asset.approve(self.contract.address(), U256::MAX);
            let result = tx.send().await;
            if let Ok(receipt) = result {
                let result = receipt.await;
                if let Err(_) = result {
                    println!("Transactions not approved!");
                }
            }
        }
        Ok(())
    }

    pub fn total_assets(&self, timestamp: U256) -> U256 {
        let latest = ((timestamp - (timestamp % INTERVAL)) / INTERVAL).as_u32();
        let mut smart_pool_earnings = U256::zero();
        for i in latest..=latest + self.max_future_pools as u32 {
            let maturity = i * INTERVAL;
            if let Some(fixed_pool) = self.fixed_pools.get(&maturity) {
                if U256::from(maturity) > fixed_pool.last_accrual {
                    smart_pool_earnings += fixed_pool.unassigned_earnings
                        * (timestamp - fixed_pool.last_accrual)
                        / (U256::from(maturity) - fixed_pool.last_accrual);
                }
            }
        }
        // println!("---------------");
        self.smart_pool_assets
            + smart_pool_earnings
            + self.smart_pool_accumulated_earnings(timestamp)
    }

    pub fn smart_pool_accumulated_earnings(&self, timestamp: U256) -> U256 {
        let elapsed = timestamp - self.last_accumulated_earnings_accrual;
        if elapsed > U256::zero() {
            self.smart_pool_earnings_accumulator * elapsed
                / (elapsed
                    + (U256::from(self.accumulated_earnings_smooth_factor)
                        * (INTERVAL * self.max_future_pools as u32)
                        / U256::exp10(18)))
        } else {
            U256::zero()
        }
    }
}
