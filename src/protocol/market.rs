use std::collections::HashMap;
use std::sync::Arc;

use ethers::abi::Address;
use ethers::prelude::{abigen, Middleware, Signer, SignerMiddleware, U256};

use ethers::types::I256;

use super::fixed_point_math::FixedPointMath;

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

pub struct Market<M, S> {
    pub contract: crate::protocol::market_mod::Market<SignerMiddleware<M, S>>,
    pub interest_rate_model: Address,
    pub oracle_price: U256,
    pub penalty_rate: U256,
    pub adjust_factor: U256,
    pub decimals: u8,
    pub floating_assets: U256,
    pub floating_deposit_shares: U256,
    pub floating_debt: U256,
    pub floating_borrow_shares: U256,
    pub floating_utilization: U256,
    pub last_floating_debt_update: U256,
    pub max_future_pools: u8,
    pub fixed_pools: HashMap<u32, FixedPool>,
    pub smart_pool_fee_rate: U256,
    pub earnings_accumulator: U256,
    pub last_accumulator_accrual: U256,
    pub earnings_accumulator_smooth_factor: u128,
    pub price_feed: Address,
    pub listed: bool,
    pub floating_full_utilization: u128,
    pub floating_a: u128,
    pub floating_b: i128,
    pub floating_max_utilization: u128,
    pub treasury_fee_rate: u128,
    pub asset: Address,
}

impl<M: 'static + Middleware, S: 'static + Signer> Eq for Market<M, S> {}

impl<M: 'static + Middleware, S: 'static + Signer> PartialEq for Market<M, S> {
    fn eq(&self, other: &Self) -> bool {
        (*self.contract).address() == (*other.contract).address()
    }
}

impl<M: 'static + Middleware, S: 'static + Signer> Market<M, S> {
    pub fn new(address: Address, client: &Arc<SignerMiddleware<M, S>>) -> Self {
        Self {
            contract: crate::protocol::market_mod::Market::new(address, Arc::clone(client)),
            interest_rate_model: Default::default(),
            oracle_price: Default::default(),
            penalty_rate: Default::default(),
            adjust_factor: Default::default(),
            decimals: Default::default(),
            floating_assets: Default::default(),
            floating_deposit_shares: Default::default(),
            floating_debt: Default::default(),
            floating_borrow_shares: Default::default(),
            floating_utilization: Default::default(),
            last_floating_debt_update: Default::default(),
            max_future_pools: Default::default(),
            fixed_pools: Default::default(),
            smart_pool_fee_rate: Default::default(),
            earnings_accumulator: Default::default(),
            last_accumulator_accrual: Default::default(),
            earnings_accumulator_smooth_factor: Default::default(),
            price_feed: Default::default(),
            listed: Default::default(),
            floating_full_utilization: Default::default(),
            floating_a: Default::default(),
            floating_b: Default::default(),
            floating_max_utilization: Default::default(),
            treasury_fee_rate: Default::default(),
            asset: Default::default(),
        }
    }

    pub fn total_assets(&self, timestamp: U256) -> U256 {
        let latest = ((timestamp - (timestamp % INTERVAL)) / INTERVAL).as_u32();
        let mut smart_pool_earnings = U256::zero();
        for i in latest..=latest + self.max_future_pools as u32 {
            let maturity = i * INTERVAL;
            if let Some(fixed_pool) = self.fixed_pools.get(&maturity) {
                let maturity = U256::from(maturity);
                if maturity > fixed_pool.last_accrual {
                    smart_pool_earnings += if timestamp < maturity {
                        fixed_pool.unassigned_earnings.mul_div_down(
                            timestamp - fixed_pool.last_accrual,
                            maturity - fixed_pool.last_accrual,
                        )
                    } else {
                        fixed_pool.unassigned_earnings
                    }
                }
            }
        }
        // println!("---------------");
        self.floating_assets
            + smart_pool_earnings
            + self.accumulated_earnings(timestamp)
            + (self.total_floating_borrow_assets(timestamp) - self.floating_debt)
                .mul_wad_down(U256::exp10(18) - self.treasury_fee_rate)
    }

    pub fn accumulated_earnings(&self, timestamp: U256) -> U256 {
        let elapsed = timestamp - self.last_accumulator_accrual;
        if elapsed > U256::zero() {
            self.earnings_accumulator.mul_div_down(
                elapsed,
                elapsed
                    + U256::from(self.earnings_accumulator_smooth_factor)
                        .mul_wad_down(U256::from(INTERVAL * self.max_future_pools as u32)),
            )
        } else {
            U256::zero()
        }
    }

    fn floating_borrow_rate(&self, utilization_before: U256, utilization_after: U256) -> U256 {
        let r = if utilization_after - utilization_before < U256::exp10(8) * 25u32 {
            U256::from(self.floating_a)
                .div_wad_down(U256::from(self.floating_max_utilization) - utilization_before)
        } else {
            U256::from(self.floating_a).mul_div_down(
                (U256::from(self.floating_max_utilization) - utilization_before)
                    .div_wad_down(U256::from(self.floating_max_utilization) - utilization_after)
                    .ln_wad()
                    .into_raw(),
                utilization_after - utilization_before,
            )
        };
        (I256::from_raw(r) + I256::from(self.floating_b)).into_raw()
    }

    pub fn total_floating_borrow_assets(&self, timestamp: U256) -> U256 {
        let new_floating_utilization = if self.floating_assets > U256::zero() {
            self.floating_debt.div_wad_up(self.floating_assets)
        } else {
            U256::zero()
        };
        let new_debt = self.floating_debt.mul_wad_down(
            self.floating_borrow_rate(
                U256::min(self.floating_utilization, new_floating_utilization),
                U256::max(self.floating_utilization, new_floating_utilization),
            )
            .mul_div_down(
                timestamp - self.last_floating_debt_update,
                U256::from(365 * 24 * 60 * 60),
            ),
        );
        self.floating_debt + new_debt
    }
}
