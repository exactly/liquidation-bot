use ethers::prelude::*;

use crate::{
    bindings::{
        AssetSeizedFilter, BorrowAtMaturityFilter, DepositAtMaturityFilter, DepositFilter,
        LiquidateBorrowFilter, RepayAtMaturityFilter, WithdrawAtMaturityFilter, WithdrawFilter,
    },
    errors::LiquidationError,
};
use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Formatter},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Eq, Clone, Debug)]
pub struct BorrowerData {
    fixed_lender: Address,
    asset_symbol: String,
    maturity_supply_positions: HashMap<U256, U256>,
    maturity_borrow_positions: HashMap<U256, U256>,
    smart_pool_assets: U256,
    smart_pool_shares: U256,
    oracle_price: U256,
    penalty_rate: U128,
    adjust_factor: U128,
    decimals: u8,
    is_collateral: bool,
}

impl PartialEq for BorrowerData {
    fn eq(&self, other: &Self) -> bool {
        self.fixed_lender == other.fixed_lender
    }
}

// FixedLender market;
// string assetSymbol;
// uint256 oraclePrice;
// uint128 penaltyRate;
// uint128 adjustFactor;
// uint8 decimals;
// uint8 maxFuturePools;
// bool isCollateral;
// uint256 smartPoolShares;
// uint256 smartPoolAssets;
// MaturityPosition[] maturitySupplyPositions;
// MaturityPosition[] maturityBorrowPositions;

impl BorrowerData {
    pub fn new(
        (
            fixed_lender,
            asset_symbol,
            oracle_price,
            penalty_rate,
            adjust_factor,
            decimals,
            max_future_pools,
            is_collateral,
            smart_pool_shares,
            smart_pool_assets,
            maturity_supply,
            maturity_borrow,
        ): (
            Address,
            String,
            U256,
            U128,
            U128,
            u8,
            u8,
            bool,
            U256,
            U256,
            Vec<(U256, (U256, U256))>,
            Vec<(U256, (U256, U256))>,
        ),
    ) -> Self {
        let mut maturity_supply_positions = HashMap::new();
        maturity_supply.into_iter().for_each(|(k, v)| {
            maturity_supply_positions.insert(k, v.0 + v.1);
        });
        let mut maturity_borrow_positions = HashMap::new();
        maturity_borrow.into_iter().for_each(|(k, v)| {
            maturity_borrow_positions.insert(k, v.0 + v.1);
        });
        BorrowerData {
            fixed_lender,
            asset_symbol,
            maturity_supply_positions,
            maturity_borrow_positions,
            smart_pool_assets,
            smart_pool_shares,
            oracle_price,
            penalty_rate,
            adjust_factor,
            decimals,
            is_collateral,
        }
    }

    pub fn fixed_lender(&self) -> H160 {
        self.fixed_lender
    }

    pub fn maturity_supply_positions(&self) -> &HashMap<U256, U256> {
        &self.maturity_supply_positions
    }

    pub fn supply_at_maturity(&self, maturity: &U256) -> Option<&U256> {
        self.maturity_supply_positions.get(&maturity)
    }

    pub fn maturity_borrow_positions(&self) -> &HashMap<U256, U256> {
        &self.maturity_borrow_positions
    }

    pub fn borrow_at_maturity(&self, maturity: &U256) -> Option<&U256> {
        self.maturity_borrow_positions.get(&maturity)
    }

    pub fn smart_pool_assets(&self) -> U256 {
        self.smart_pool_assets
    }

    pub fn smart_pool_shares(&self) -> U256 {
        self.smart_pool_shares
    }

    pub fn oracle_price(&self) -> U256 {
        self.oracle_price
    }

    pub fn penalty_rate(&self) -> U128 {
        self.penalty_rate
    }

    pub fn decimals(&self) -> u8 {
        self.decimals
    }

    pub fn is_collateral(&self) -> bool {
        self.is_collateral
    }

    pub fn adjust_factor(&self) -> U128 {
        self.adjust_factor
    }
}

#[derive(Eq, Clone)]
pub struct Borrower {
    borrower: Address,
    data: HashMap<Address, BorrowerData>,
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
            "\n==============
\tBorrower               {:?}
\tTotal Collateral       {:?}
\tTotal Debt             {:?}
\tSeizable Collateral    {:?}
\tDebt on Fixed Lender   {:?}
\tData\n{:?}\n",
            self.borrower,
            self.collateral,
            self.debt,
            self.seizable_collateral,
            self.fixed_lender_to_liquidate,
            self.data
        )

        // write!(
        //     f,
        //     "\tBorrower: {:?}\n\tDebt: {:?}\n\tSeizable collateral: {:?}\n\tCollateral: {:?}",
        //     self.borrower, self.debt, self.seizable_collateral, self.collateral
        // )
    }
}

impl Borrower {
    pub fn new_borrower_data(borrower: Address, data: Vec<BorrowerData>) -> Self {
        let mut borrower_data = HashMap::new();
        data.into_iter().for_each(|d| {
            borrower_data.insert(d.fixed_lender, d);
        });
        Borrower {
            borrower,
            data: borrower_data,
            debt: None,
            seizable_collateral: None,
            fixed_lender_to_liquidate: None,
            collateral: None,
        }
    }
    pub fn new(
        borrower: Address,
        account_data: Vec<(
            Address,
            String,
            U256,
            U128,
            U128,
            u8,
            u8,
            bool,
            U256,
            U256,
            Vec<(U256, (U256, U256))>,
            Vec<(U256, (U256, U256))>,
        )>,
    ) -> Self {
        let mut data = HashMap::<Address, BorrowerData>::new();
        for d in account_data {
            data.insert(d.0, BorrowerData::new(d));
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
        for (_, data) in self.data.iter() {
            if data.is_collateral {
                let current_collateral = (data.smart_pool_assets * data.oracle_price
                    / U256::exp10(usize::from(data.decimals)))
                    * U256::from(data.adjust_factor)
                    / U256::exp10(18);
                if current_collateral > seizable_collateral.0 {
                    seizable_collateral = (current_collateral, Some(data.fixed_lender));
                }
                collateral += current_collateral;
            }
            let mut current_debt = U256::zero();
            for (maturity, borrowed) in data.maturity_borrow_positions.iter() {
                let current_timestamp = Self::get_timestamp_seconds();
                current_debt += *borrowed;
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

    pub fn deposit(&mut self, deposit: DepositFilter, fixed_lender: &Address) {
        if self.data.contains_key(fixed_lender) {
            println!("Updating fixed lender");
            let data = self.data.get_mut(fixed_lender).unwrap();
            data.smart_pool_assets += deposit.assets;
            data.smart_pool_shares += deposit.shares;
        } else {
            println!("Adding fixed lender");
            let data = BorrowerData::new((
                fixed_lender.clone(),
                String::new(),
                U256::zero(),
                U128::zero(),
                U128::zero(),
                0u8,
                0u8,
                false,
                deposit.shares,
                deposit.assets,
                Vec::new(),
                Vec::new(),
            ));
            self.data.insert(*fixed_lender, data);
            println!("Fixed lender count: {:?}", self.data.len());
        }
    }

    pub fn withdraw(&mut self, withdraw: WithdrawFilter, fixed_lender: &Address) {
        if self.data.contains_key(fixed_lender) {
            let mut data = self.data.get_mut(fixed_lender).unwrap();
            data.smart_pool_assets -= withdraw.assets;
            data.smart_pool_shares -= withdraw.shares;
        }
    }

    pub fn deposit_at_maturity(
        &mut self,
        deposit: DepositAtMaturityFilter,
        fixed_lender: &Address,
    ) {
        if self.data.contains_key(fixed_lender) {
            let mut data = self.data.get_mut(fixed_lender).unwrap();
            if data
                .maturity_supply_positions
                .contains_key(&deposit.maturity)
            {
                let mut suply = data
                    .maturity_supply_positions
                    .get_mut(&deposit.maturity)
                    .unwrap();
                *suply += deposit.assets + deposit.fee;
            } else {
                data.maturity_supply_positions
                    .insert(deposit.maturity, deposit.assets + deposit.fee);
            }
        }
    }

    pub fn withdraw_at_maturity(
        &mut self,
        withdraw: WithdrawAtMaturityFilter,
        fixed_lender: &Address,
    ) {
        if self.data.contains_key(fixed_lender) {
            let mut data = self.data.get_mut(fixed_lender).unwrap();
            if data
                .maturity_supply_positions
                .contains_key(&withdraw.maturity)
            {
                let mut suply = data
                    .maturity_supply_positions
                    .get_mut(&withdraw.maturity)
                    .unwrap();
                // TODO check if this is correct
                *suply -= withdraw.assets;
            }
        }
    }

    pub fn borrow_at_maturity(&mut self, borrow: BorrowAtMaturityFilter, fixed_lender: &Address) {
        let mut data = if self.data.contains_key(fixed_lender) {
            self.data.get_mut(fixed_lender).unwrap()
        } else {
            let data = BorrowerData::new((
                fixed_lender.clone(),
                String::new(),
                U256::zero(),
                U128::zero(),
                U128::zero(),
                0u8,
                0u8,
                false,
                U256::zero(),
                U256::zero(),
                Vec::new(),
                Vec::new(),
            ));
            self.data.insert(*fixed_lender, data);
            self.data.get_mut(fixed_lender).unwrap()
        };
        if data
            .maturity_borrow_positions
            .contains_key(&borrow.maturity)
        {
            let borrowed = data
                .maturity_borrow_positions
                .get_mut(&borrow.maturity)
                .unwrap();
            *borrowed += borrow.assets + borrow.fee;
        } else {
            data.maturity_borrow_positions
                .insert(borrow.maturity, borrow.assets + borrow.fee);
        }
    }

    pub fn repay_at_maturity(&mut self, repay: RepayAtMaturityFilter, fixed_lender: &Address) {
        if self.data.contains_key(fixed_lender) {
            let mut data = self.data.get_mut(fixed_lender).unwrap();
            if data.maturity_borrow_positions.contains_key(&repay.maturity) {
                let borrowed = data
                    .maturity_borrow_positions
                    .get_mut(&repay.maturity)
                    .unwrap();
                // TODO check if this is correct
                *borrowed -= repay.assets;
            }
        }
    }

    pub fn liquidate_borrow(&mut self, liquidate: LiquidateBorrowFilter, fixed_lender: &Address) {
        // It needs no action since the events emited by the repay_at_maturity are enough to liquidate the borrow
    }

    pub fn asset_seized(&mut self, seize: AssetSeizedFilter, fixed_lender: &Address) {
        // it needs no action since the events emited by the repay_at_maturity are enough to seixe the borrow's assets
    }

    pub fn set_collateral(&mut self, fixed_lender: &Address) {
        if self.data.contains_key(fixed_lender) {
            println!("Setting as collateral");
            let mut data = self.data.get_mut(fixed_lender).unwrap();
            data.is_collateral = true;
        }
    }

    pub fn unset_collateral(&mut self, fixed_lender: &Address) {
        if self.data.contains_key(fixed_lender) {
            let mut data = self.data.get_mut(fixed_lender).unwrap();
            data.is_collateral = false;
        }
    }

    pub fn data(&self) -> &HashMap<Address, BorrowerData> {
        &self.data
    }

    pub fn add_markets(&mut self, markets: &HashSet<Address>) {
        for market in markets {
            if !self.data.contains_key(market) {
                let data = BorrowerData::new((
                    market.clone(),
                    String::new(),
                    U256::zero(),
                    U128::zero(),
                    U128::zero(),
                    0u8,
                    0u8,
                    false,
                    U256::zero(),
                    U256::zero(),
                    Vec::new(),
                    Vec::new(),
                ));
                self.data.insert(market.clone(), data);
            }
        }
    }
}
