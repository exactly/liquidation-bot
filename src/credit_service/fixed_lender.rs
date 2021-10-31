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
    pub oracle_price: Option<U256>,
    pub penalty_rate: Option<U256>,
    pub adjust_factor: Option<U256>,
    pub decimals: Option<u8>,
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

    pub fn set_oracle_price(&mut self, oracle_price: Option<U256>) {
        println!("Price: {:?}", oracle_price.unwrap());
        self.oracle_price = oracle_price;
    }

    pub fn set_penalty_rate(&mut self, penalty_rate: Option<U256>) {
        self.penalty_rate = penalty_rate;
    }

    pub fn set_adjust_factor(&mut self, adjust_factor: Option<U256>) {
        self.adjust_factor = adjust_factor;
    }

    pub fn set_decimals(&mut self, decimals: Option<u8>) {
        self.decimals = decimals;
    }

    pub fn add_shares(&mut self, shares: U256) {
        self.total_shares = self.total_shares + shares;
    }

    pub fn sub_shares(&mut self, shares: U256) {
        println!("Subtracting shares: {} from {}", shares, &self.total_shares);
        self.total_shares = self.total_shares - shares;
    }

    pub fn oracle_price(&self) -> Option<U256> {
        self.oracle_price
    }

    pub fn penalty_rate(&self) -> Option<U256> {
        self.penalty_rate
    }

    pub fn adjust_factor(&self) -> Option<U256> {
        self.adjust_factor
    }

    pub fn decimals(&self) -> Option<u8> {
        self.decimals
    }

    pub fn total_shares(&self) -> U256 {
        self.total_shares
    }

    pub fn set_max_future_pools(&mut self, max_future_pools: u8) {
        self.max_future_pools = max_future_pools;
    }

    pub fn get_yield_for_deposit(
        smart_pool_borrowed: U256,
        unassigned_earnings: U256,
        amount: U256,
        smart_pool_fee_rate: U256,
    ) -> (U256, U256) {
        println!("smart_pool_borrowed     : {}", smart_pool_borrowed);
        if smart_pool_borrowed != U256::zero() {
            let earnings_share =
                unassigned_earnings * U256::min(smart_pool_borrowed, amount) / smart_pool_borrowed;
            let earnings_share_smart_pool = earnings_share * smart_pool_fee_rate / U256::exp10(18);
            println!("earnings_share           : {}", earnings_share);
            println!("earnings_share_smart_pool: {}", earnings_share_smart_pool);
            println!("unassigned_earnings      : {}", unassigned_earnings);

            return (
                earnings_share - earnings_share_smart_pool,
                earnings_share_smart_pool,
            );
        }
        (U256::zero(), U256::zero())
    }

    pub fn smart_pool_borrowed(fixed_pool: &FixedPool) -> U256 {
        fixed_pool.borrowed - U256::min(fixed_pool.borrowed, fixed_pool.supplied)
    }

    pub fn deposit_at_maturity(
        &mut self,
        maturity: U256,
        principal: U256,
        fee: U256,
        timestamp: U256,
    ) {
        let fixed_pool = self.fixed_pools.entry(maturity).or_default();
        let smart_pool_earnings = Self::accrue_earnings(fixed_pool, maturity, timestamp);
        let (new_fee, smart_pool_fee) = Self::get_yield_for_deposit(
            Self::smart_pool_borrowed(fixed_pool),
            fixed_pool.unassigned_earnings,
            principal,
            self.smart_pool_fee_rate,
        );
        println!("self.smart_pool_fee_rate: {:?}", self.smart_pool_fee_rate);
        println!("new_fee: {:?}, fee: {:?}", new_fee, fee);
        fixed_pool.supplied += principal;
        fixed_pool.unassigned_earnings -= new_fee + smart_pool_fee;
        self.smart_pool_earnings_accumulator += smart_pool_fee;
        self.smart_pool_assets += smart_pool_earnings;
    }

    pub fn borrow_at_maturity(
        &mut self,
        maturity: U256,
        principal: U256,
        fee: U256,
        timestamp: U256,
    ) {
        let fixed_pool = self.fixed_pools.entry(maturity).or_default();
        let smart_pool_earnings = Self::accrue_earnings(fixed_pool, maturity, timestamp);
        fixed_pool.borrowed += principal;
        let (new_unassigned_earnings, new_smart_pool_earnings) =
            Self::distribute_earnings_accordingly(
                fee,
                Self::smart_pool_borrowed(fixed_pool),
                principal,
            );
        fixed_pool.unassigned_earnings += new_unassigned_earnings;
        self.smart_pool_earnings_accumulator += new_smart_pool_earnings;
        self.smart_pool_assets += smart_pool_earnings;
    }

    pub fn repay_at_maturity(
        &mut self,
        maturity: U256,
        assets: U256,
        position_assets: U256,
        timestamp: U256,
        principal_covered: U256,
    ) {
        let fixed_pool = self.fixed_pools.entry(maturity).or_default();
        let smart_pool_earnings = Self::accrue_earnings(fixed_pool, maturity, timestamp);
        if timestamp < maturity {
            let (discount, smart_pool_fee) = Self::get_yield_for_deposit(
                Self::smart_pool_borrowed(fixed_pool),
                fixed_pool.unassigned_earnings,
                principal_covered,
                self.smart_pool_fee_rate,
            );
            fixed_pool.unassigned_earnings -= discount + smart_pool_fee;
            self.smart_pool_earnings_accumulator += smart_pool_fee;
        } else {
            self.smart_pool_earnings_accumulator += assets - position_assets;
        }
        fixed_pool.borrowed -= principal_covered;
        self.smart_pool_assets += smart_pool_earnings;
    }

    pub fn scale_proportionally(position: (U256, U256), amount: U256) -> (U256, U256) {
        let principal = amount * position.0 / (position.0 + position.1);
        (principal, amount - principal)
    }

    fn distribute_earnings_accordingly(
        earnings: U256,
        supplied_smart_pool: U256,
        amount_funded: U256,
    ) -> (U256, U256) {
        // println!(
        //     "earnings: {:?}, supplied_smart_pool: {:?}, amount_funded: {:?}",
        //     earnings, supplied_smart_pool, amount_funded
        // );
        let smart_pool_earnings = if amount_funded > U256::zero() {
            earnings * (amount_funded - U256::min(supplied_smart_pool, amount_funded))
                / amount_funded
        } else {
            U256::zero()
        };
        (earnings - smart_pool_earnings, smart_pool_earnings)
    }

    fn seconds_pre(from: U256, to: U256) -> U256 {
        if from < to {
            to - from
        } else {
            U256::zero()
        }
    }

    fn accrue_earnings(fixed_pool: &mut FixedPool, maturity: U256, timestamp: U256) -> U256 {
        let elapsed = Self::seconds_pre(fixed_pool.last_accrual, U256::min(maturity, timestamp));
        let remaining = Self::seconds_pre(fixed_pool.last_accrual, maturity);
        fixed_pool.last_accrual = U256::min(maturity, timestamp);
        let smart_pool_earnings = fixed_pool.unassigned_earnings * elapsed / remaining;
        println!("Fixed pool {:?}", fixed_pool);
        println!("Smart pool earnings {}", smart_pool_earnings);
        fixed_pool.unassigned_earnings -= smart_pool_earnings;
        smart_pool_earnings
    }

    pub fn set_smart_pool_fee_rate(&mut self, smart_pool_fee_rate: U256) {
        self.smart_pool_fee_rate = smart_pool_fee_rate;
    }

    pub fn total_assets(&mut self, timestamp: U256) -> U256 {
        let latest = ((timestamp - (timestamp % INTERVAL)) / INTERVAL).as_u64();
        let mut smart_pool_earnings = U256::zero();
        for i in latest..=latest + self.max_future_pools as u64 {
            let maturity = U256::from(i) * INTERVAL;
            let fixed_pool = self.fixed_pools.entry(maturity).or_default();
            if maturity > fixed_pool.last_accrual {
                smart_pool_earnings += fixed_pool.unassigned_earnings
                    * (timestamp - fixed_pool.last_accrual)
                    / (maturity - fixed_pool.last_accrual);
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

    pub fn set_accumulated_earnings_smooth_factor(
        &mut self,
        accumulated_earnings_smooth_factor: u128,
    ) {
        self.accumulated_earnings_smooth_factor = accumulated_earnings_smooth_factor;
    }

    pub fn deposit(&mut self, assets: U256, timestamp: U256) {
        let earnings = self.smart_pool_accumulated_earnings(timestamp);
        self.last_accumulated_earnings_accrual = timestamp;
        self.smart_pool_earnings_accumulator -= earnings;
        self.smart_pool_assets += assets + earnings;
    }

    pub fn withdraw(&mut self, assets: U256, timestamp: U256) {
        self.last_accumulated_earnings_accrual = timestamp;
        let earnings = self.smart_pool_accumulated_earnings(timestamp);
        self.smart_pool_earnings_accumulator -= earnings;
        self.smart_pool_assets = self.smart_pool_assets + earnings - assets;
    }
}
