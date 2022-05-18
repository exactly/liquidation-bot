use ethers::prelude::*;

use crate::{errors::LiquidationError, price_oracle::oracle::PriceOracle};

pub struct Borrower {
    borrower: Address,
    sum_collateral: U256,
    sum_debts: U256,
    health_factor: u64,
}

impl Borrower {
    pub fn new(borrower: Address, sum_collateral: U256, sum_debts: U256) -> Self {
        Borrower {
            borrower,
            sum_collateral,
            sum_debts,
            health_factor: 0,
        }
    }

    pub fn compute_hf<M: Middleware, S: Signer>(
        &mut self,
        _underlying_token: Address,
        _cumulative_index_now: &U256,
        _price_oracle: &PriceOracle<M, S>,
        // credit_filter: &CreditFilter<SignerMiddleware<M, S>>,
    ) -> Result<u64, LiquidationError> {
        // let mut total: U256 = 0.into();
        // for asset in self.balances.clone() {
        //     total += price_oracle.convert(asset.1, asset.0, underlying_token)?
        //         * credit_filter.liquidation_thresholds.get(&asset.0).unwrap();
        // }

        // let borrowed_amount_plus_interest =
        //     self.borrowed_amount * cumulative_index_now / self.cumulative_index_at_open;
        // self.health_factor = ((total / borrowed_amount_plus_interest).as_u64());

        Ok(self.health_factor)
    }
}
