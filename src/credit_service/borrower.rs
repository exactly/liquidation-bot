use ethers::prelude::*;

use crate::errors::LiquidationError;
use std::{
    fmt::{Debug, Formatter},
    time::{SystemTime, UNIX_EPOCH},
};

use super::FixedLender;

#[derive(Hash, Eq, Clone, Debug)]
struct BorrowerData {
    fixed_lender: Address,
    asset_symbol: String,
    maturity_supply_positions: Vec<(U256, (U256, U256))>,
    maturity_borrow_positions: Vec<(U256, (U256, U256))>,
    smart_pool_assets: U256,
    smart_pool_shares: U256,
    oracle_price: U256,
    penalty_rate: U128,
    collateral_factor: U128,
    decimals: u8,
    is_collateral: bool,
}

impl PartialEq for BorrowerData {
    fn eq(&self, other: &Self) -> bool {
        self.fixed_lender == other.fixed_lender
    }
}

impl BorrowerData {
    fn new(
        (
            fixed_lender,
            asset_symbol,
            maturity_supply_positions,
            maturity_borrow_positions,
            smart_pool_assets,
            smart_pool_shares,
            oracle_price,
            penalty_rate,
            collateral_factor,
            decimals,
            is_collateral,
        ): (
            Address,
            String,
            Vec<(U256, (U256, U256))>,
            Vec<(U256, (U256, U256))>,
            U256,
            U256,
            U256,
            U128,
            U128,
            u8,
            bool,
        ),
    ) -> Self {
        BorrowerData {
            fixed_lender,
            asset_symbol,
            maturity_supply_positions,
            maturity_borrow_positions,
            smart_pool_assets,
            smart_pool_shares,
            oracle_price,
            penalty_rate,
            collateral_factor,
            decimals,
            is_collateral,
        }
    }
}

#[derive(Hash, Eq, Clone)]
pub struct Borrower {
    borrower: Address,
    data: Vec<BorrowerData>,
    debt: Option<U256>,
    seizable_collateral: Option<Address>,
    fixed_lender_to_liquidate: Option<Address>,
    collateral: Option<U256>,
}

impl PartialEq for Borrower {
    fn eq(&self, other: &Self) -> bool {
        self.borrower == other.borrower
    }
}

impl Debug for Borrower {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "==============
\tBorrower               {:?}
\tTotal Collateral       {:?}
\tTotal Debt             {:?}
\tSeizable Collateral    {:?}
\tDebt on Fixed Lender   {:?}",
            self.borrower,
            self.collateral,
            self.debt,
            self.seizable_collateral,
            self.fixed_lender_to_liquidate,
        )

        // write!(
        //     f,
        //     "\tBorrower: {:?}\n\tDebt: {:?}\n\tSeizable collateral: {:?}\n\tCollateral: {:?}",
        //     self.borrower, self.debt, self.seizable_collateral, self.collateral
        // )
    }
}

impl Borrower {
    pub fn new(
        borrower: Address,
        account_data: Vec<(
            Address,
            String,
            Vec<(U256, (U256, U256))>,
            Vec<(U256, (U256, U256))>,
            U256,
            U256,
            U256,
            U128,
            U128,
            u8,
            bool,
        )>,
    ) -> Self {
        let mut data = Vec::<BorrowerData>::new();
        for d in account_data {
            data.push(BorrowerData::new(d));
        }
        Borrower {
            borrower,
            data,
            debt: None,
            seizable_collateral: None,
            fixed_lender_to_liquidate: None,
            collateral: None,
        }
    }

    pub fn compute_hf(&mut self) -> Result<U256, LiquidationError> {
        let mut collateral: U256 = U256::zero();
        let mut debt: U256 = U256::zero();
        let mut seizable_collateral: (U256, Option<Address>) = (U256::zero(), None);
        let mut fixed_lender_to_liquidate: (U256, Option<Address>) = (U256::zero(), None);
        for data in self.data.iter() {
            if data.is_collateral {
                let current_collateral = (data.smart_pool_assets * data.oracle_price
                    / U256::exp10(usize::from(data.decimals)))
                    * U256::from(data.collateral_factor)
                    / U256::exp10(18);
                if current_collateral > seizable_collateral.0 {
                    seizable_collateral = (current_collateral, Some(data.fixed_lender));
                }
                collateral += current_collateral;
            }
            let mut current_debt = U256::zero();
            for (maturity, (principal, fee)) in data.maturity_borrow_positions.iter() {
                let current_timestamp = Self::get_timestamp_seconds();
                current_debt += principal + fee;
                if *maturity < current_timestamp {
                    current_debt += (current_timestamp - maturity) * U256::from(data.penalty_rate)
                }
            }
            debt += current_debt;
            if current_debt > fixed_lender_to_liquidate.0 {
                fixed_lender_to_liquidate = (current_debt, Some(data.fixed_lender));
            }
        }
        self.collateral = Some(collateral);
        self.seizable_collateral = seizable_collateral.1;
        self.fixed_lender_to_liquidate = fixed_lender_to_liquidate.1;
        self.debt = Some(debt);
        let hf = if debt == U256::zero() {
            collateral
        } else {
            U256::exp10(18) * collateral / debt
        };
        println!("==============");
        println!("Borrower               {:?}", self.borrower);
        println!("Total Collateral       {:?}", collateral);
        println!("Total Debt             {:?}", debt);
        println!("Seizable Collateral    {:?}", seizable_collateral.1);
        println!("Seizable Collateral  $ {:?}", seizable_collateral.0);
        println!("Debt on Fixed Lender   {:?}", fixed_lender_to_liquidate.1);
        println!("Debt on Fixed Lender $ {:?}", fixed_lender_to_liquidate.0);
        println!("Health factor {:?}\n", hf);
        Ok(hf)
    }

    /// Get the borrower's address.
    pub fn borrower(&self) -> H160 {
        self.borrower
    }

    fn get_timestamp_seconds() -> U256 {
        U256::from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
    }

    /// Get the borrower's debt.
    #[must_use]
    pub fn debt(&self) -> U256 {
        if let Some(debt) = self.debt {
            debt
        } else {
            U256::zero()
        }
    }

    /// Get the borrower's seizable collateral.
    #[must_use]
    pub fn seizable_collateral(&self) -> Option<Address> {
        self.seizable_collateral
    }

    /// Get the borrower's collateral.
    #[must_use]
    pub fn collateral(&self) -> Option<U256> {
        self.collateral
    }

    /// Get the borrower's fixed lender to liquidate.
    #[must_use]
    pub fn fixed_lender_to_liquidate(&self) -> Option<H160> {
        self.fixed_lender_to_liquidate
    }
}
