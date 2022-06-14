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
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{fixed_lender, FixedLender};

#[derive(Clone, Debug)]
pub struct BorrowerData {
    fixed_lender: Arc<Mutex<FixedLender>>,
    maturity_supply_positions: HashMap<U256, U256>,
    maturity_borrow_positions: HashMap<U256, U256>,
    smart_pool_assets: U256,
    smart_pool_shares: U256,
    is_collateral: bool,
}

impl Eq for BorrowerData {}

impl PartialEq for BorrowerData {
    fn eq(&self, other: &Self) -> bool {
        self.fixed_lender.lock().unwrap().address() == other.fixed_lender.lock().unwrap().address()
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
    pub fn new_with_market(market: Arc<Mutex<FixedLender>>) -> Self {
        let mut maturity_supply_positions = HashMap::new();
        let mut maturity_borrow_positions = HashMap::new();
        BorrowerData {
            fixed_lender: market,
            maturity_supply_positions,
            maturity_borrow_positions,
            smart_pool_assets: U256::zero(),
            smart_pool_shares: U256::zero(),
            is_collateral: false,
        }
    }

    pub fn new_from_previewer(
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
        let mut fixed_lender = FixedLender::new(fixed_lender);

        fixed_lender.set_oracle_price(Some(oracle_price));
        fixed_lender.set_adjust_factor(Some(U256::from(adjust_factor)));
        fixed_lender.set_decimals(Some(decimals));
        fixed_lender.set_penalty_rate(Some(U256::from(penalty_rate)));
        fixed_lender.set_asset_symbol(Some(asset_symbol));

        let fixed_lender = Arc::new(Mutex::new(fixed_lender));
        BorrowerData {
            fixed_lender,
            maturity_supply_positions,
            maturity_borrow_positions,
            smart_pool_assets,
            smart_pool_shares,
            is_collateral,
        }
    }

    pub fn fixed_lender(&self) -> Arc<Mutex<FixedLender>> {
        Arc::clone(&self.fixed_lender)
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

    pub fn oracle_price(&self) -> Option<U256> {
        self.fixed_lender.lock().unwrap().oracle_price()
    }

    pub fn penalty_rate(&self) -> Option<U256> {
        self.fixed_lender.lock().unwrap().penalty_rate()
    }

    pub fn decimals(&self) -> Option<u8> {
        self.fixed_lender.lock().unwrap().decimals()
    }

    pub fn is_collateral(&self) -> bool {
        self.is_collateral
    }

    pub fn adjust_factor(&self) -> Option<U256> {
        self.fixed_lender.lock().unwrap().adjust_factor()
    }
}

#[derive(Eq, Clone)]
pub struct Borrower {
    borrower: Address,
    markets: HashMap<Address, BorrowerData>,
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
            self.markets
        )

        // write!(
        //     f,
        //     "\tBorrower: {:?}\n\tDebt: {:?}\n\tSeizable collateral: {:?}\n\tCollateral: {:?}",
        //     self.borrower, self.debt, self.seizable_collateral, self.collateral
        // )
    }
}

impl Borrower {
    pub fn new_borrower_markets(borrower: Address, data: Vec<BorrowerData>) -> Self {
        let mut markets = HashMap::new();
        data.into_iter().for_each(|d| {
            let address = d.fixed_lender.lock().unwrap().address();
            markets.insert(address, d);
        });
        Borrower {
            borrower,
            markets,
            debt: None,
            seizable_collateral: None,
            fixed_lender_to_liquidate: None,
            collateral: None,
        }
    }
    pub fn new_from_previewer(
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
            data.insert(d.0, BorrowerData::new_from_previewer(d));
        }
        Borrower {
            borrower,
            markets: data,
            debt: None,
            seizable_collateral: None,
            fixed_lender_to_liquidate: None,
            collateral: None,
        }
    }

    pub fn compute_hf(&mut self) -> Result<U256, LiquidationError> {
        // let mut collateral: U256 = U256::zero();
        // let mut debt: U256 = U256::zero();
        // let mut seizable_collateral: (U256, Option<Address>) = (U256::zero(), None);
        // let mut fixed_lender_to_liquidate: (U256, Option<Address>) = (U256::zero(), None);
        // for (_, data) in self.markets.iter() {
        //     if data.is_collateral {
        //         let current_collateral = (data.smart_pool_assets * data.oracle_price
        //             / U256::exp10(usize::from(data.decimals)))
        //             * U256::from(data.adjust_factor)
        //             / U256::exp10(18);
        //         if current_collateral > seizable_collateral.0 {
        //             seizable_collateral = (current_collateral, Some(data.fixed_lender));
        //         }
        //         collateral += current_collateral;
        //     }
        //     let mut current_debt = U256::zero();
        //     for (maturity, borrowed) in data.maturity_borrow_positions.iter() {
        //         let current_timestamp = Self::get_timestamp_seconds();
        //         current_debt += *borrowed;
        //         if *maturity < current_timestamp {
        //             current_debt += (current_timestamp - maturity) * U256::from(data.penalty_rate)
        //         }
        //     }
        //     debt += current_debt;
        //     if current_debt > fixed_lender_to_liquidate.0 {
        //         fixed_lender_to_liquidate = (current_debt, Some(data.fixed_lender));
        //     }
        // }
        // self.collateral = Some(collateral);
        // self.seizable_collateral = seizable_collateral.1;
        // self.fixed_lender_to_liquidate = fixed_lender_to_liquidate.1;
        // self.debt = Some(debt);
        // let hf = if debt == U256::zero() {
        //     collateral
        // } else {
        //     U256::exp10(18) * collateral / debt
        // };
        // println!("==============");
        // println!("Borrower               {:?}", self.borrower);
        // println!("Total Collateral       {:?}", collateral);
        // println!("Total Debt             {:?}", debt);
        // println!("Seizable Collateral    {:?}", seizable_collateral.1);
        // println!("Seizable Collateral  $ {:?}", seizable_collateral.0);
        // println!("Debt on Fixed Lender   {:?}", fixed_lender_to_liquidate.1);
        // println!("Debt on Fixed Lender $ {:?}", fixed_lender_to_liquidate.0);
        // println!("Health factor {:?}\n", hf);
        // Ok(hf)
        Ok(U256::zero())
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
        if self.markets.contains_key(fixed_lender) {
            println!("Updating fixed lender");
            let data = self.markets.get_mut(fixed_lender).unwrap();
            data.smart_pool_assets += deposit.assets;
            data.smart_pool_shares += deposit.shares;
        } else {
            println!("Adding fixed lender");
            let data = BorrowerData::new_from_previewer((
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
            self.markets.insert(*fixed_lender, data);
            println!("Fixed lender count: {:?}", self.markets.len());
        }
    }

    pub fn withdraw(&mut self, withdraw: WithdrawFilter, fixed_lender: &Address) {
        if self.markets.contains_key(fixed_lender) {
            let mut data = self.markets.get_mut(fixed_lender).unwrap();
            data.smart_pool_assets -= withdraw.assets;
            data.smart_pool_shares -= withdraw.shares;
        }
    }

    pub fn deposit_at_maturity(
        &mut self,
        deposit: DepositAtMaturityFilter,
        fixed_lender: &Address,
    ) {
        if self.markets.contains_key(fixed_lender) {
            let mut data = self.markets.get_mut(fixed_lender).unwrap();
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
        if self.markets.contains_key(fixed_lender) {
            let mut data = self.markets.get_mut(fixed_lender).unwrap();
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
        let mut data = if self.markets.contains_key(fixed_lender) {
            self.markets.get_mut(fixed_lender).unwrap()
        } else {
            let data = BorrowerData::new_from_previewer((
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
            self.markets.insert(*fixed_lender, data);
            self.markets.get_mut(fixed_lender).unwrap()
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
        if self.markets.contains_key(fixed_lender) {
            let mut data = self.markets.get_mut(fixed_lender).unwrap();
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
        if self.markets.contains_key(fixed_lender) {
            println!("Setting as collateral");
            let mut data = self.markets.get_mut(fixed_lender).unwrap();
            data.is_collateral = true;
        }
    }

    pub fn unset_collateral(&mut self, fixed_lender: &Address) {
        if self.markets.contains_key(fixed_lender) {
            let mut data = self.markets.get_mut(fixed_lender).unwrap();
            data.is_collateral = false;
        }
    }

    pub fn data(&self) -> &HashMap<Address, BorrowerData> {
        &self.markets
    }

    pub fn add_markets(&mut self, markets: Vec<Arc<FixedLender>>) {
        for market in markets {
            if !self.markets.contains_key(&market.address()) {
                let data = BorrowerData::new_from_previewer((
                    market.address().clone(),
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
                self.markets.insert(market.address().clone(), data);
            }
        }
    }
}
