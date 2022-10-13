use ethers::prelude::*;

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
};

use super::{
    fixed_point_math::FixedPointMathGen, BorrowAtMaturityFilter, DepositAtMaturityFilter, Market,
    RepayAtMaturityFilter, SeizeFilter, WithdrawAtMaturityFilter,
};

use super::fixed_point_math::FixedPointMath;

#[derive(Clone, Default, Eq, PartialEq)]
pub struct AccountPosition {
    pub fixed_deposit_positions: HashMap<U256, U256>,
    pub fixed_borrow_positions: HashMap<U256, U256>,
    pub floating_deposit_shares: U256,
    pub floating_borrow_shares: U256,
    pub is_collateral: bool,
}

impl Debug for AccountPosition {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // let market = self.market.lock().unwrap();
        write!(
            f,
            "
\t\tfloating_deposit_shares                  : {:#?}
\t\tfloating_borrow_shares                   : {:#?}
\t\tfixed_deposit_positions                  : {:#?}
\t\tfixed_borrow_positions                   : {:#?}
",
            self.floating_deposit_shares,
            self.floating_borrow_shares,
            self.fixed_deposit_positions,
            self.fixed_borrow_positions,
        )
    }
}

impl AccountPosition {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn floating_deposit_assets<M: 'static + Middleware, S: 'static + Signer>(
        &self,
        market: &Market<M, S>,
        timestamp: U256,
    ) -> U256 {
        if market.floating_deposit_shares == U256::zero() {
            self.floating_deposit_shares
        } else {
            self.floating_deposit_shares.mul_div_down(
                market.total_assets(timestamp),
                market.floating_deposit_shares,
            )
        }
    }

    pub fn floating_borrow_assets<M: 'static + Middleware, S: 'static + Signer>(
        &self,
        market: &Market<M, S>,
        timestamp: U256,
    ) -> U256 {
        if market.floating_borrow_shares == U256::zero() {
            self.floating_borrow_shares
        } else {
            self.floating_borrow_shares.mul_div_up(
                market.total_floating_borrow_assets(timestamp),
                market.floating_borrow_shares,
            )
        }
    }
}

#[derive(Eq, Clone, Default)]
pub struct Account {
    pub address: Address,
    pub positions: HashMap<Address, AccountPosition>,
}

impl PartialEq for Account {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl Debug for Account {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "\n==============
\tAccount                {:?}
\tData\n{:?}\n",
            self.address, self.positions
        )
    }
}

impl Account {
    pub fn new<M: Middleware, S: Signer>(
        address: Address,
        market_map: &HashMap<Address, Market<M, S>>,
    ) -> Self {
        let mut markets = HashMap::<Address, AccountPosition>::new();
        for address in market_map.keys() {
            markets.insert(*address, AccountPosition::new());
        }

        Self {
            address,
            positions: markets,
            ..Default::default()
        }
    }

    pub fn deposit_at_maturity(&mut self, deposit: &DepositAtMaturityFilter, market: &Address) {
        let data = self.positions.entry(*market).or_default();
        if data.fixed_deposit_positions.contains_key(&deposit.maturity) {
            let supply = data
                .fixed_deposit_positions
                .get_mut(&deposit.maturity)
                .unwrap();
            *supply += deposit.assets + deposit.fee;
        } else {
            data.fixed_deposit_positions
                .insert(deposit.maturity, deposit.assets + deposit.fee);
        }
    }

    pub fn withdraw_at_maturity(&mut self, withdraw: WithdrawAtMaturityFilter, market: &Address) {
        let data = self.positions.entry(*market).or_default();
        if data
            .fixed_deposit_positions
            .contains_key(&withdraw.maturity)
        {
            let supply = data
                .fixed_deposit_positions
                .get_mut(&withdraw.maturity)
                .unwrap();
            // TODO check if this is correct
            *supply -= withdraw.position_assets;
        }
    }

    pub fn borrow_at_maturity(&mut self, borrow: &BorrowAtMaturityFilter, market: &Address) {
        let data = self.positions.entry(*market).or_default();

        if data.fixed_borrow_positions.contains_key(&borrow.maturity) {
            let borrowed = data
                .fixed_borrow_positions
                .get_mut(&borrow.maturity)
                .unwrap();
            *borrowed += borrow.assets + borrow.fee;
        } else {
            data.fixed_borrow_positions
                .insert(borrow.maturity, borrow.assets + borrow.fee);
        }
    }

    pub fn repay_at_maturity(&mut self, repay: &RepayAtMaturityFilter, market: &Address) {
        let data = self.positions.entry(*market).or_default();
        if let Some(position) = data.fixed_borrow_positions.get_mut(&repay.maturity) {
            *position -= repay.position_assets;
        }
    }

    pub fn asset_seized(&mut self, _seize: SeizeFilter, _fixed_lender: &Address) {
        // it needs no action since the events emitted by the repay_at_maturity are enough to seize the borrow's assets
    }

    pub fn set_collateral(&mut self, market: &Address) {
        println!("Setting as collateral");
        let data = self.positions.entry(*market).or_default();
        data.is_collateral = true;
    }

    pub fn unset_collateral(&mut self, market: &Address) {
        let data = self.positions.entry(*market).or_default();
        data.is_collateral = false;
    }
}
