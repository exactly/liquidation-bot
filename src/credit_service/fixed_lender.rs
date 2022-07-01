use std::collections::HashMap;

use ethers::abi::Address;
use ethers::prelude::U256;

const INTERVAL: u64 = 4 * 7 * 86_400;

#[derive(Eq, PartialEq, Debug, Default)]
pub struct FixedPool {
    pub borrowed: U256,
    pub supplied: U256,
    pub unassigned_earnings: U256,
    pub last_accrual: U256,
}

#[derive(Eq, Default)]
pub struct FixedLender {
    pub address: Address,
    // pub contract: Option<Market<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>>,
    pub oracle_price: U256,
    pub penalty_rate: U256,
    pub adjust_factor: U256,
    pub decimals: u8,
    pub smart_pool_assets: U256,
    pub total_shares: U256,
    pub max_future_pools: u8,
    pub fixed_pools: HashMap<U256, FixedPool>,
    pub smart_pool_fee_rate: U256,
    pub smart_pool_earnings_accumulator: U256,
    pub last_accumulated_earnings_accrual: U256,
    pub accumulated_earnings_smooth_factor: u128,
    pub price_feed: Address,
    pub listed: bool,
}

impl PartialEq for FixedLender {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl FixedLender {
    pub fn new(address: Address) -> Self {
        println!("=========== ASDF {:?}", address);
        Self {
            address,
            ..Default::default()
        }
    }

    pub fn total_assets(&self, timestamp: U256) -> U256 {
        let latest = ((timestamp - (timestamp % INTERVAL)) / INTERVAL).as_u64();
        let mut smart_pool_earnings = U256::zero();
        for i in latest..=latest + self.max_future_pools as u64 {
            let maturity = U256::from(i) * INTERVAL;
            if let Some(fixed_pool) = self.fixed_pools.get(&maturity) {
                if maturity > fixed_pool.last_accrual {
                    smart_pool_earnings += fixed_pool.unassigned_earnings
                        * (timestamp - fixed_pool.last_accrual)
                        / (maturity - fixed_pool.last_accrual);
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
                        * (INTERVAL * self.max_future_pools as u64)
                        / U256::exp10(18)))
        } else {
            U256::zero()
        }
    }
}
