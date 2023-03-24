use super::config::Config;
use super::fixed_point_math::{math, FixedPointMath, FixedPointMathGen};
use super::protocol::TokenPairFeeMap;
use ethers::prelude::{
    Address, Middleware, Multicall, MulticallVersion, Signer, SignerMiddleware, U256,
};
use eyre::Result;
use log::{error, info, warn};
use sentry::{Breadcrumb, Level};
use serde::Deserialize;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;
use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use tokio::time;

use super::{
    protocol::{self, Protocol},
    Account,
};
use crate::generate_abi::{Auditor, LiquidationIncentive, Liquidator, MarketAccount, Previewer};
use crate::network::{Network, NetworkStatus};

#[derive(Default, Debug)]
pub struct RepaySettings {
    pub max_repay: U256,
    pub pool_pair: Address,
    pub pair_fee: u32,
    pub fee: u32,
}

#[derive(Default, Debug)]
pub struct Repay {
    pub price: U256,
    pub decimals: u8,
    pub market_to_seize: Option<Address>,
    pub market_to_seize_value: U256,
    pub market_to_repay: Option<Address>,
    pub market_to_liquidate_debt: U256,
    pub total_value_collateral: U256,
    pub total_adjusted_collateral: U256,
    pub total_value_debt: U256,
    pub total_adjusted_debt: U256,
    pub repay_asset_address: Address,
    pub collateral_asset_address: Address,
}

#[derive(Debug, Default)]
pub enum LiquidationAction {
    #[default]
    Update,
    Insert,
}

#[derive(Default, Debug)]
pub struct ProtocolState {
    pub gas_price: Option<U256>,
    pub markets: Vec<Address>,
    pub assets: HashMap<Address, Address>,
    pub price_feeds: HashMap<Address, Address>,
    pub price_decimals: U256,
}

#[derive(Default, Debug)]
pub struct LiquidationData {
    pub liquidations: HashMap<Address, (Account, Repay)>,
    pub action: LiquidationAction,
    pub state: ProtocolState,
}

pub struct Liquidation<M, W, S> {
    pub client: Arc<SignerMiddleware<M, S>>,
    token_pairs: Arc<TokenPairFeeMap>,
    tokens: Arc<HashSet<Address>>,
    liquidator: Liquidator<SignerMiddleware<W, S>>,
    previewer: Previewer<SignerMiddleware<M, S>>,
    auditor: Auditor<SignerMiddleware<M, S>>,
    market_weth_address: Address,
    backup: u32,
    liquidate_unprofitable: bool,
    repay_offset: U256,
    network: Arc<Network>,
}

impl<
        M: 'static + Middleware + Clone,
        W: 'static + Middleware + Clone,
        S: 'static + Signer + Clone,
    > Liquidation<M, W, S>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: Arc<SignerMiddleware<M, S>>,
        client_relay: Arc<SignerMiddleware<W, S>>,
        token_pairs: &str,
        previewer: Previewer<SignerMiddleware<M, S>>,
        auditor: Auditor<SignerMiddleware<M, S>>,
        market_weth_address: Address,
        config: &Config,
        network: Arc<Network>,
    ) -> Self {
        let (token_pairs, tokens) = parse_token_pairs(token_pairs);
        let token_pairs = Arc::new(token_pairs);
        let tokens = Arc::new(tokens);
        let liquidator =
            Self::get_contracts(Arc::clone(&client_relay), config.chain_id_name.clone());
        Self {
            client,
            token_pairs,
            tokens,
            liquidator,
            previewer,
            auditor,
            market_weth_address,
            backup: config.backup,
            liquidate_unprofitable: config.liquidate_unprofitable,
            repay_offset: config.repay_offset,
            network,
        }
    }

    fn get_contracts(
        client_relayer: Arc<SignerMiddleware<W, S>>,
        chain_id_name: String,
    ) -> Liquidator<SignerMiddleware<W, S>> {
        let (liquidator_address, _, _) = Protocol::<M, W, S>::parse_abi(&format!(
            "deployments/{}/Liquidator.json",
            chain_id_name
        ));
        Liquidator::new(liquidator_address, Arc::clone(&client_relayer))
    }

    pub fn get_tokens(&self) -> Arc<HashSet<Address>> {
        Arc::clone(&self.tokens)
    }

    pub fn get_token_pairs(&self) -> Arc<TokenPairFeeMap> {
        Arc::clone(&self.token_pairs)
    }

    pub async fn run(
        this: Arc<Mutex<Self>>,
        mut receiver: Receiver<LiquidationData>,
    ) -> Result<()> {
        let mut liquidations = HashMap::new();
        let mut liquidations_iter = None;
        let mut state = ProtocolState::default();
        let backup = this.lock().await.backup;
        let d = Duration::from_millis(1);
        loop {
            match time::timeout(d, receiver.recv()).await {
                Ok(Some(data)) => {
                    match data.action {
                        LiquidationAction::Update => {
                            info!("Updating liquidation data");
                            liquidations = data
                                .liquidations
                                .into_iter()
                                .map(|(account_address, (account, repay))| {
                                    let age = if backup > 0 {
                                        liquidations
                                            .get(&account_address)
                                            .map(|(_, _, age)| *age)
                                            .unwrap_or(0)
                                            + 1
                                    } else {
                                        0
                                    };
                                    (account_address, (account, repay, age))
                                })
                                .collect();
                        }
                        LiquidationAction::Insert => {
                            let mut new_liquidations = data.liquidations;
                            for (k, v) in new_liquidations.drain() {
                                let liquidation = liquidations.entry(k).or_insert((v.0, v.1, 0));
                                if backup > 0 {
                                    liquidation.2 += 1;
                                }
                            }
                        }
                    }
                    liquidations_iter = Some(liquidations.iter());
                    state.gas_price = data.state.gas_price;
                    state.markets = data.state.markets;
                    state.price_feeds = data.state.price_feeds;
                    state.price_decimals = data.state.price_decimals;
                    state.assets = data.state.assets;
                }
                Ok(None) => {}
                Err(_) => {
                    if let Some(liquidation) = &mut liquidations_iter {
                        info!("Check for next to liquidate");
                        if let Some((_, (account, repay, age))) = liquidation.next() {
                            info!("Found");
                            if backup == 0 || *age > backup {
                                if backup > 0 {
                                    info!("backup liquidation - {}", age);
                                }
                                let _ = this.lock().await.liquidate(account, repay, &state).await;
                            } else {
                                info!("backup - not old enough: {}", age);
                            }
                        } else {
                            info!("Not found");
                            liquidations_iter = None;
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
        state: &ProtocolState,
    ) -> Result<()> {
        info!("Liquidating account {:?}", account);
        if let Some(address) = &repay.market_to_repay {
            let response = self.is_profitable_async(account.address, state).await;

            let ((profitable, profit, cost), repay_settings, gas_used, will_revert) = match response
            {
                Some(response) => response,
                None => return Ok(()),
            };
            gen_liq_breadcrumb(account, repay, &repay_settings);
            if will_revert {
                error!("Liquidation would revert - not sent");
                return Ok(());
            }

            if !profitable && !self.liquidate_unprofitable {
                gen_liq_breadcrumb(account, repay, &repay_settings);
                warn!(
                    "liquidation not profitable for {:#?} (profit: {:#?} cost: {:#?})",
                    account.address, profit, cost
                );
                return Ok(());
            }

            info!("Liquidating on market {:#?}", address);
            info!("seizing                    {:#?}", repay.market_to_seize);

            // liquidate using liquidator contract
            info!("repay     : {:#?}", *address);
            info!(
                "seize     : {:#?}",
                repay.market_to_seize.unwrap_or(Address::zero())
            );
            info!("borrower  : {:#?}", account.address);
            info!("max_repay : {:#?}", repay_settings.max_repay);
            info!("pool_pair : {:#?}", repay_settings.pool_pair);
            info!("pair_fee  : {:#?}", repay_settings.pair_fee);
            info!("fee       : {:#?}", repay_settings.fee);

            let func = self
                .liquidator
                .liquidate(
                    *address,
                    repay.market_to_seize.unwrap_or(Address::zero()),
                    account.address,
                    repay_settings.max_repay,
                    repay_settings.pool_pair,
                    repay_settings.pair_fee,
                    repay_settings.fee,
                )
                // Increase gas cost by 20%
                .gas(
                    gas_used
                        .unwrap_or(self.network.default_gas_used())
                        .mul_div_down(U256::from(150), U256::from(100)),
                );
            info!("func: {:#?}", func);
            let tx = func.send().await;
            info!("tx: {:#?}", &tx);
            let tx = tx?;
            info!("waiting receipt");
            let receipt = tx.confirmations(1).await?;
            info!("Liquidation tx {:?}", receipt);
            let (adjusted_collateral, adjusted_debt): (U256, U256) = self
                .auditor
                .account_liquidity(account.address, Address::zero(), U256::zero())
                .call()
                .await?;
            let hf = adjusted_collateral.div_wad_down(adjusted_debt);
            if hf < math::WAD {
                info!("hf        : {:#?}", hf);
                info!("collateral: {:#?}", adjusted_collateral);
                info!("debt      : {:#?}", adjusted_debt);
                error!("liquidation failed");
                return Ok(());
            }
        }
        info!("done liquidating");
        Ok(())
    }

    pub async fn is_profitable_async(
        &self,
        account: Address,
        state: &ProtocolState,
    ) -> Option<((bool, U256, U256), RepaySettings, Option<U256>, bool)> {
        let mut multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                .await
                .unwrap();
        multicall.add_call(self.previewer.exactly(account), false);
        multicall.add_call(
            self.auditor
                .account_liquidity(account, Address::zero(), U256::zero()),
            false,
        );
        multicall.add_call(self.auditor.liquidation_incentive(), false);

        let mut price_multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                .await
                .unwrap();
        state.markets.iter().for_each(|market| {
            if state.price_feeds[market] != Address::zero()
                && state.price_feeds[market] != Address::from_str(protocol::BASE_FEED).unwrap()
            {
                price_multicall
                    .add_call(self.auditor.asset_price(state.price_feeds[market]), false);
            }
        });
        let multicall = multicall.version(MulticallVersion::Multicall);
        price_multicall = price_multicall.version(MulticallVersion::Multicall);
        let response = tokio::try_join!(multicall.call(), price_multicall.call_raw());

        let (data, prices) = if let Ok(response) = response {
            response
        } else {
            if let Err(err) = response {
                info!("error: {:?}", err);
            }
            info!("error getting multicall data");
            return None;
        };

        let (market_account, (adjusted_collateral, adjusted_debt), liquidation_incentive): (
            Vec<MarketAccount>,
            (U256, U256),
            LiquidationIncentive,
        ) = data;

        let mut i = 0;
        let prices: HashMap<Address, U256> = state
            .markets
            .iter()
            .map(|market| {
                if state.price_feeds[market] != Address::zero()
                    && state.price_feeds[market] != Address::from_str(protocol::BASE_FEED).unwrap()
                {
                    let price = (*market, prices[i].clone().unwrap().into_uint().unwrap());
                    i += 1;
                    price
                } else {
                    (*market, U256::exp10(state.price_decimals.as_usize()))
                }
            })
            .collect();

        if adjusted_debt.is_zero() {
            info!("no debt");
            return None;
        }
        let hf = adjusted_collateral.div_wad_down(adjusted_debt);
        if hf > math::WAD {
            info!("healthy");
            return None;
        }
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let repay = Self::pick_markets(&market_account, &prices, timestamp.into(), &state.assets);
        let repay_settings = Self::get_liquidation_settings(
            &repay,
            &liquidation_incentive,
            self.repay_offset,
            &self.token_pairs,
            &self.tokens,
        );

        // check if transaction will revert
        let gas_used = if let Some(address) = &repay.market_to_repay {
            let func = self.liquidator.liquidate(
                *address,
                repay.market_to_seize.unwrap_or(Address::zero()),
                account,
                repay_settings.max_repay,
                repay_settings.pool_pair,
                repay_settings.pair_fee,
                repay_settings.fee,
            );

            match func.estimate_gas().await {
                Ok(gas) => gas,
                Err(err) => {
                    let mut data = BTreeMap::new();
                    data.insert("message".to_string(), Value::String(err.to_string()));
                    sentry::add_breadcrumb(Breadcrumb {
                        ty: "error".to_string(),
                        category: Some("estimating gas".to_string()),
                        level: Level::Error,
                        data,
                        ..Default::default()
                    });
                    return Some((
                        (false, U256::zero(), U256::zero()),
                        repay_settings,
                        None,
                        true,
                    ));
                }
            }
        } else {
            return None;
        };

        let gas_used = if self.network.can_estimate_gas() {
            Some(gas_used)
        } else {
            None
        };

        Some((
            Self::is_profitable(
                &self.network,
                &repay,
                &repay_settings,
                &liquidation_incentive,
                NetworkStatus {
                    gas_price: state.gas_price,
                    gas_used,
                    eth_price: prices[&self.market_weth_address],
                },
            ),
            repay_settings,
            gas_used,
            false,
        ))
    }

    pub fn pick_markets(
        market_account: &Vec<MarketAccount>,
        prices: &HashMap<Address, U256>,
        timestamp: U256,
        assets: &HashMap<Address, Address>,
    ) -> Repay {
        let mut repay = Repay::default();
        for market in market_account {
            if market.is_collateral {
                let collateral_value = market.floating_deposit_assets.mul_div_down(
                    prices[&market.market],
                    U256::exp10(market.decimals as usize),
                );
                let adjusted_collateral = collateral_value.mul_wad_down(market.adjust_factor);
                repay.total_value_collateral += collateral_value;
                repay.total_adjusted_collateral += adjusted_collateral;
                if adjusted_collateral >= repay.market_to_seize_value {
                    repay.market_to_seize_value = adjusted_collateral;
                    repay.market_to_seize = Some(market.market);
                    repay.collateral_asset_address = assets[&market.market];
                }
            };
            let mut market_debt_assets = U256::zero();
            for fixed_position in &market.fixed_borrow_positions {
                let borrowed = fixed_position.position.principal + fixed_position.position.fee;
                market_debt_assets += borrowed;
                if fixed_position.maturity < timestamp {
                    market_debt_assets += borrowed
                        .mul_wad_down((timestamp - fixed_position.maturity) * market.penalty_rate)
                }
            }
            market_debt_assets += market.floating_borrow_assets;
            let market_debt_value = market_debt_assets.mul_div_up(
                prices[&market.market],
                U256::exp10(market.decimals as usize),
            );
            let adjusted_debt = market_debt_value.div_wad_up(market.adjust_factor);
            repay.total_value_debt += market_debt_value;
            repay.total_adjusted_debt += adjusted_debt;
            if adjusted_debt >= repay.market_to_liquidate_debt {
                repay.market_to_liquidate_debt = adjusted_debt;
                repay.market_to_repay = Some(market.market);
                repay.price = prices[&market.market];
                repay.decimals = market.decimals;
                repay.repay_asset_address = assets[&market.market];
            }
        }
        repay
    }

    #[allow(clippy::too_many_arguments)]
    pub fn is_profitable(
        network: &Network,
        repay: &Repay,
        repay_settings: &RepaySettings,
        liquidation_incentive: &LiquidationIncentive,
        network_status: NetworkStatus,
    ) -> (bool, U256, U256) {
        let profit = Self::max_profit(repay, repay_settings.max_repay, liquidation_incentive);
        let cost = Self::max_cost(
            network,
            repay,
            repay_settings.max_repay,
            liquidation_incentive,
            U256::from(repay_settings.pair_fee),
            U256::from(repay_settings.fee),
            network_status,
        );
        (
            profit > cost && profit - cost > math::WAD / U256::exp10(16),
            profit,
            cost,
        )
    }

    fn get_flash_pair(
        repay: &Repay,
        token_pairs: &HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>,
        tokens: &HashSet<Address>,
    ) -> (Address, u32, u32) {
        let collateral = repay.collateral_asset_address;
        let repay = repay.repay_asset_address;

        let mut lowest_fee = u32::MAX;
        let mut pair_contract = Address::zero();

        if collateral != repay {
            if let Some(pair) = token_pairs.get(&ordered_addresses(collateral, repay)) {
                return (Address::zero(), pair.peek().unwrap().0, 0);
            } else {
                let (route, _) = token_pairs.iter().fold(
                    (Address::zero(), u32::MAX),
                    |(address, lowest_fee), ((token0, token1), pair_fee)| {
                        let route = if *token0 == repay {
                            token1
                        } else if *token1 == repay {
                            token0
                        } else {
                            return (address, lowest_fee);
                        };
                        if let Some(fee) = token_pairs.get(&ordered_addresses(collateral, *route)) {
                            let total_fee = pair_fee.peek().unwrap().0 + fee.peek().unwrap().0;
                            if total_fee < lowest_fee {
                                return (*route, total_fee);
                            }
                        }
                        (address, lowest_fee)
                    },
                );
                return match (
                    token_pairs.get(&ordered_addresses(route, repay)),
                    token_pairs.get(&ordered_addresses(collateral, route)),
                ) {
                    (Some(pair_fee), Some(fee)) => {
                        (route, pair_fee.peek().unwrap().0, fee.peek().unwrap().0)
                    }

                    _ => (Address::zero(), 0, 0),
                };
            }
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
        (pair_contract, lowest_fee, 0)
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
                    .total_value_debt
                    .mul_wad_up(U256::min(math::WAD, close_factor)),
                repay.market_to_seize_value.div_wad_up(
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
        .min(repay.market_to_liquidate_debt)
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

    #[allow(clippy::too_many_arguments)]
    fn max_cost(
        network: &Network,
        repay: &Repay,
        max_repay: U256,
        liquidation_incentive: &LiquidationIncentive,
        swap_pair_fee: U256,
        swap_fee: U256,
        network_status: NetworkStatus,
    ) -> U256 {
        let max_repay = max_repay.mul_div_down(repay.price, U256::exp10(repay.decimals as usize));
        max_repay.mul_wad_down(U256::from(liquidation_incentive.lenders))
            + max_repay.mul_wad_down(swap_fee * U256::exp10(12))
            + max_repay.mul_wad_down(swap_pair_fee * U256::exp10(12))
            + network.tx_cost(network_status)
    }

    pub fn calculate_close_factor(
        repay: &Repay,
        liquidation_incentive: &LiquidationIncentive,
    ) -> U256 {
        let target_health = U256::exp10(16usize) * 125u32;
        let adjust_factor = repay
            .total_adjusted_collateral
            .mul_wad_down(repay.total_value_debt)
            .div_wad_up(
                repay
                    .total_adjusted_debt
                    .mul_wad_up(repay.total_value_collateral),
            );
        (target_health
            - repay
                .total_adjusted_collateral
                .div_wad_up(repay.total_adjusted_debt))
        .div_wad_up(
            target_health
                - adjust_factor.mul_wad_down(
                    math::WAD
                        + liquidation_incentive.liquidator
                        + liquidation_incentive.lenders
                        + U256::from(liquidation_incentive.liquidator)
                            .mul_wad_down(liquidation_incentive.lenders.into()),
                ),
        )
    }

    pub fn set_liquidator(
        &mut self,
        client_relay: Arc<SignerMiddleware<W, S>>,
        chain_id_name: String,
    ) {
        self.liquidator = Self::get_contracts(Arc::clone(&client_relay), chain_id_name);
    }

    pub fn get_liquidation_settings(
        repay: &Repay,
        liquidation_incentive: &LiquidationIncentive,
        repay_offset: U256,
        token_pairs: &HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>,
        tokens: &HashSet<Address>,
    ) -> RepaySettings {
        let max_repay = Self::max_repay_assets(repay, liquidation_incentive, U256::MAX)
            .mul_wad_down(math::WAD + U256::exp10(14))
            + repay_offset.mul_div_up(U256::exp10(repay.decimals as usize), repay.price);
        let (pool_pair, pair_fee, fee): (Address, u32, u32) =
            Self::get_flash_pair(repay, token_pairs, tokens);
        RepaySettings {
            max_repay,
            pool_pair,
            pair_fee,
            fee,
        }
    }
}

pub fn gen_liq_breadcrumb(account: &Account, repay: &Repay, repay_settings: &RepaySettings) {
    let mut data = BTreeMap::new();
    data.insert(
        "account".to_string(),
        Value::String(account.address.to_string()),
    );
    data.insert(
        "market to liquidate".to_string(),
        Value::String(
            repay
                .market_to_repay
                .map(|v| v.to_string())
                .unwrap_or("market empty".to_string()),
        ),
    );
    data.insert(
        "market to seize".to_string(),
        Value::String(
            repay
                .market_to_seize
                .map(|v| v.to_string())
                .unwrap_or("market empty".to_string()),
        ),
    );
    data.insert(
        "value to seize".to_string(),
        Value::String(repay.market_to_seize_value.to_string()),
    );
    data.insert(
        "collateral value".to_string(),
        Value::String(repay.total_value_collateral.to_string()),
    );
    data.insert(
        "debt value".to_string(),
        Value::String(repay.total_value_debt.to_string()),
    );
    data.insert(
        "repay value".to_string(),
        Value::String(repay_settings.max_repay.to_string()),
    );
    data.insert(
        "pool pair".to_string(),
        Value::String(repay_settings.pool_pair.to_string()),
    );
    data.insert(
        "pool fee".to_string(),
        Value::String(repay_settings.fee.to_string()),
    );
    data.insert(
        "pool pair fee".to_string(),
        Value::String(repay_settings.pair_fee.to_string()),
    );
    sentry::add_breadcrumb(Breadcrumb {
        ty: "error".to_string(),
        category: Some("Reverting Liquidation".to_string()),
        level: Level::Error,
        data,
        ..Default::default()
    });
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

fn parse_token_pairs(token_pairs: &str) -> (TokenPairFeeMap, HashSet<Address>) {
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
            .or_insert_with(BinaryHeap::new)
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
                        Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
                        Address::from_str("0x0000000000000000000000000000000000000000").unwrap()
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
