use super::fixed_point_math::{math, FixedPointMath};
use ethers::prelude::{Address, Middleware, Signer, SignerMiddleware, U256};
use eyre::Result;
use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use tokio::time;

use super::{Account, LiquidationIncentive, Liquidator};

#[derive(Default, Debug)]
pub struct Repay {
    pub price: U256,
    pub decimals: u8,
    pub seize_available: U256,
    pub seizable_collateral: Option<Address>,
    pub market_to_liquidate: Option<Address>,
    pub total_collateral: U256,
    pub adjusted_collateral: U256,
    pub total_debt: U256,
    pub adjusted_debt: U256,
    pub repay_asset_address: Address,
    pub collateral_asset_address: Address,
    pub market_to_liquidate_debt: U256,
}

pub struct Liquidation<M, S> {
    token_pairs: Arc<HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>>,
    tokens: Arc<HashSet<Address>>,
    liquidator: Liquidator<SignerMiddleware<M, S>>,
}

impl<M: 'static + Middleware, S: 'static + Signer> Liquidation<M, S> {
    pub fn new(token_pairs: &str, liquidator: Liquidator<SignerMiddleware<M, S>>) -> Self {
        let (token_pairs, tokens) = parse_token_pairs(token_pairs);
        let token_pairs = Arc::new(token_pairs);
        let tokens = Arc::new(tokens);
        Self {
            token_pairs,
            tokens,
            liquidator,
        }
    }

    pub fn get_tokens(&self) -> Arc<HashSet<Address>> {
        Arc::clone(&self.tokens)
    }

    pub fn get_token_pairs(&self) -> Arc<HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>> {
        Arc::clone(&self.token_pairs)
    }

    pub async fn run(
        this: Arc<Mutex<Self>>,
        mut receiver: Receiver<(
            HashMap<Address, (Account, Repay)>,
            U256,
            LiquidationIncentive,
        )>,
    ) -> Result<()> {
        let mut liquidations;
        let mut liquidations_iter = None;
        let mut eth_price = U256::zero();
        let mut liquidation_incentive = None;
        let d = Duration::from_millis(1);
        loop {
            match time::timeout(d, receiver.recv()).await {
                Ok(Some(data)) => {
                    liquidations = data.0;
                    liquidations_iter = Some(liquidations.iter());
                    eth_price = data.1;
                    liquidation_incentive = Some(data.2);
                }
                Ok(None) => {}
                Err(_) => {
                    if let Some(liquidation) = &mut liquidations_iter {
                        if let Some((_, (account, repay))) = liquidation.next() {
                            this.lock()
                                .await
                                .liquidate(
                                    account,
                                    repay,
                                    liquidation_incentive.as_ref().unwrap(),
                                    0u8.into(),
                                    eth_price,
                                )
                                .await?;
                        }
                    }
                }
            }
        }
    }

    async fn liquidate(
        &self,
        account: &Account,
        repay: &Repay,
        liquidation_incentive: &LiquidationIncentive,
        last_gas_price: U256,
        eth_price: U256,
    ) -> Result<()> {
        println!("Liquidating account {:?}", account);
        if let Some(address) = &repay.market_to_liquidate {
            let (profitable, max_repay, pool_pair, fee) = Self::is_profitable(
                repay,
                liquidation_incentive,
                last_gas_price,
                eth_price,
                &self.token_pairs,
                &self.tokens,
            );

            if !profitable {
                println!("not profitable to liquidate");
                println!(
                    "repay$: {:?}",
                    max_repay.mul_div_up(repay.price, U256::exp10(repay.decimals as usize))
                );
                return Ok(());
            }

            println!("Liquidating on market {:#?}", address);
            println!(
                "seizing                    {:#?}",
                repay.seizable_collateral
            );

            // liquidate using liquidator contract
            let func = self
                .liquidator
                .liquidate(
                    *address,
                    repay.seizable_collateral.unwrap_or(Address::zero()),
                    account.address,
                    max_repay,
                    pool_pair,
                    fee,
                )
                .gas(6_666_666u128);

            let tx = func.send().await;
            println!("tx: {:?}", &tx);
            let tx = tx?;
            println!("waiting receipt");
            let receipt = tx.confirmations(1).await?;
            println!("Liquidation tx {:?}", receipt);
        }
        println!("done liquidating");
        Ok(())
    }

    pub fn is_profitable(
        repay: &Repay,
        liquidation_incentive: &LiquidationIncentive,
        last_gas_price: U256,
        eth_price: U256,
        token_pairs: &HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>,
        tokens: &HashSet<Address>,
    ) -> (bool, U256, Address, u32) {
        let max_repay = Self::max_repay_assets(repay, liquidation_incentive, U256::MAX)
            .mul_wad_down(math::WAD + U256::exp10(14))
            + math::WAD.mul_div_up(U256::exp10(repay.decimals as usize), repay.price);
        let (pool_pair, fee): (Address, u32) = Self::get_flash_pair(repay, token_pairs, tokens);
        let profit = Self::max_profit(repay, max_repay, liquidation_incentive);
        let cost = Self::max_cost(
            repay,
            max_repay,
            liquidation_incentive,
            U256::from(fee),
            last_gas_price,
            U256::from(1500u128),
            eth_price,
        );
        let profitable = profit > cost && profit - cost > math::WAD / U256::exp10(16);
        (profitable, max_repay, pool_pair, fee)
    }

    fn get_flash_pair(
        repay: &Repay,
        token_pairs: &HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>,
        tokens: &HashSet<Address>,
    ) -> (Address, u32) {
        let collateral = repay.collateral_asset_address;
        let repay = repay.repay_asset_address;

        let mut lowest_fee = u32::MAX;
        let mut pair_contract = Address::zero();

        if collateral != repay {
            if let Some(pair) = token_pairs.get(&ordered_addresses(collateral, repay)) {
                return (collateral, pair.peek().unwrap().0);
            }
            return (collateral, 0);
        }

        for token in tokens {
            if *token != collateral {
                if let Some(pair) = token_pairs.get(&ordered_addresses(*token, collateral)) {
                    if let Some(rate) = pair.peek() {
                        if rate.0 < lowest_fee {
                            lowest_fee = rate.0;
                            pair_contract = *token;
                        }
                    }
                }
            }
        }
        (pair_contract, lowest_fee)
    }

    fn max_repay_assets(
        repay: &Repay,
        liquidation_incentive: &LiquidationIncentive,
        max_liquidator_assets: U256,
    ) -> U256 {
        let close_factor = Self::calculate_close_factor(repay, liquidation_incentive);
        U256::min(
            U256::min(
                repay
                    .total_debt
                    .mul_wad_up(U256::min(math::WAD, close_factor)),
                repay.seize_available.div_wad_up(
                    math::WAD + liquidation_incentive.liquidator + liquidation_incentive.lenders,
                ),
            )
            .mul_div_up(U256::exp10(repay.decimals as usize), repay.price),
            if max_liquidator_assets
                < U256::from_str("115792089237316195423570985008687907853269984665640564039457") //// U256::MAX / WAD
                    .unwrap()
            {
                max_liquidator_assets.div_wad_down(math::WAD + liquidation_incentive.lenders)
            } else {
                max_liquidator_assets
            },
        )
    }

    fn max_profit(
        repay: &Repay,
        max_repay: U256,
        liquidation_incentive: &LiquidationIncentive,
    ) -> U256 {
        max_repay
            .mul_div_up(repay.price, U256::exp10(repay.decimals as usize))
            .mul_wad_down(U256::from(
                liquidation_incentive.liquidator + liquidation_incentive.lenders,
            ))
    }

    fn max_cost(
        repay: &Repay,
        max_repay: U256,
        liquidation_incentive: &LiquidationIncentive,
        swap_fee: U256,
        gas_price: U256,
        gas_cost: U256,
        eth_price: U256,
    ) -> U256 {
        let max_repay = max_repay.mul_div_down(repay.price, U256::exp10(repay.decimals as usize));
        max_repay.mul_wad_down(U256::from(liquidation_incentive.lenders))
            + max_repay.mul_wad_down(swap_fee * U256::from(U256::exp10(12)))
            + (gas_price * gas_cost).mul_wad_down(eth_price)
    }

    fn calculate_close_factor(repay: &Repay, liquidation_incentive: &LiquidationIncentive) -> U256 {
        let target_health = U256::exp10(16usize) * 125u32;
        let adjust_factor = repay
            .adjusted_collateral
            .div_wad_up(repay.total_collateral)
            .mul_wad_up(repay.total_debt.div_wad_up(repay.adjusted_debt));
        let close_factor = (target_health
            - repay.adjusted_collateral.div_wad_up(repay.adjusted_debt))
        .div_wad_up(
            target_health
                - adjust_factor.mul_wad_down(
                    math::WAD + liquidation_incentive.liquidator + liquidation_incentive.lenders,
                ),
        );
        close_factor
    }

    pub fn set_liquidator(&mut self, liquidator: Liquidator<SignerMiddleware<M, S>>) {
        self.liquidator = liquidator;
    }
}

fn ordered_addresses(token0: Address, token1: Address) -> (Address, Address) {
    if token0 < token1 {
        (token0, token1)
    } else {
        (token1, token0)
    }
}

#[derive(Deserialize, Debug)]
pub struct TokenPair {
    pub token0: String,
    pub token1: String,
    pub fee: u32,
}

fn parse_token_pairs(
    token_pairs: &str,
) -> (
    HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>,
    HashSet<Address>,
) {
    let mut tokens = HashSet::new();
    let json_pairs: Vec<(String, String, u32)> = serde_json::from_str(token_pairs).unwrap();
    let mut pairs = HashMap::new();
    for (token0, token1, fee) in json_pairs {
        let token0 = Address::from_str(&token0).unwrap();
        let token1 = Address::from_str(&token1).unwrap();
        tokens.insert(token0);
        tokens.insert(token1);
        pairs
            .entry(ordered_addresses(token0, token1))
            .or_insert(BinaryHeap::new())
            .push(Reverse(fee));
    }
    (pairs, tokens)
}

#[cfg(test)]
mod services_test {

    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_parse_token_pairs() {
        let tokens = r#"[[
                            "0x0000000000000000000000000000000000000000", 
                            "0x0000000000000000000000000000000000000001", 
                            3000
                          ],
                          [
                            "0x0000000000000000000000000000000000000000", 
                            "0x0000000000000000000000000000000000000001", 
                            1000
                          ],
                          [
                            "0x0000000000000000000000000000000000000000", 
                            "0x0000000000000000000000000000000000000001", 
                            2000
                          ]]"#;
        let (pairs, _) = parse_token_pairs(tokens);
        assert_eq!(
            pairs
                .get(
                    &(ordered_addresses(
                        Address::from_str(&"0x0000000000000000000000000000000000000001").unwrap(),
                        Address::from_str(&"0x0000000000000000000000000000000000000000").unwrap()
                    ))
                )
                .unwrap()
                .peek()
                .unwrap()
                .0,
            1000
        );
    }
}
