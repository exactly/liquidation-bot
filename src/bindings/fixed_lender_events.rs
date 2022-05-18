use ethers::{
    contract::{self as ethers_contract},
    core::{
        self as ethers_core,
        types::{Address, U256},
    },
    prelude::EthLogDecode,
};

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "DepositAtMaturity",
    abi = "DepositAtMaturity(uint256,address,address,uint256,uint256)"
)]
pub struct DepositAtMaturityFilter {
    #[ethevent(indexed)]
    pub maturity: U256,
    #[ethevent(indexed)]
    pub caller: Address,
    #[ethevent(indexed)]
    pub owner: Address,
    pub assets: U256,
    pub fee: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "WithdrawAtMaturity",
    abi = "WithdrawAtMaturity(uint256,address,address,address,uint256,uint256)"
)]
pub struct WithdrawAtMaturityFilter {
    #[ethevent(indexed)]
    pub maturity: U256,
    pub caller: Address,
    #[ethevent(indexed)]
    pub receiver: Address,
    #[ethevent(indexed)]
    pub owner: Address,
    pub assets: U256,
    pub assetsDiscounted: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "BorrowAtMaturity",
    abi = "BorrowAtMaturity(uint256,address,address,address,uint256,uint256)"
)]
pub struct BorrowAtMaturityFilter {
    #[ethevent(indexed)]
    pub maturity: U256,
    pub caller: Address,
    #[ethevent(indexed)]
    pub receiver: Address,
    #[ethevent(indexed)]
    pub borrower: Address,
    pub assets: U256,
    pub fee: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "RepayAtMaturity",
    abi = "RepayAtMaturity(uint256,address,address,uint256,uint256)"
)]
pub struct RepayAtMaturityFilter {
    #[ethevent(indexed)]
    pub maturity: U256,
    #[ethevent(indexed)]
    pub caller: Address,
    #[ethevent(indexed)]
    pub borrower: Address,
    pub assets: U256,
    pub debtCovered: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "LiquidateBorrow",
    abi = "LiquidateBorrow(uint256,address,address,uint256,address,uint256)"
)]
pub struct LiquidateBorrowFilter {
    #[ethevent(indexed)]
    pub maturity: U256,
    #[ethevent(indexed)]
    pub receiver: Address,
    #[ethevent(indexed)]
    pub borrower: Address,
    pub assets: U256,
    pub collateralFixedLender: Address,
    pub seizedAssets: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "AssetSeized", abi = "AssetSeized(address,address,uint256)")]
pub struct AssetSeizedFilter {
    pub liquidator: Address,
    pub borrower: Address,
    pub assets: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "AccumulatedEarningsSmoothFactorUpdated",
    abi = "AccumulatedEarningsSmoothFactorUpdated(uint256)"
)]
pub struct AccumulatedEarningsSmoothFactorUpdatedFilter {
    pub newAccumulatedEarningsSmoothFactor: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "MaxFuturePoolsUpdate", abi = "MaxFuturePoolsUpdate(uint256)")]
pub struct MaxFuturePoolsUpdatedFilter {
    pub newMaxFuturePools: U256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixedLenderEvents {
    DepositAtMaturityFilter(DepositAtMaturityFilter),
    WithdrawAtMaturityFilter(WithdrawAtMaturityFilter),
    BorrowAtMaturityFilter(BorrowAtMaturityFilter),
    RepayAtMaturityFilter(RepayAtMaturityFilter),
    LiquidateBorrowFilter(LiquidateBorrowFilter),
    AssetSeizedFilter(AssetSeizedFilter),
    AccumulatedEarningsSmoothFactorUpdatedFilter(AccumulatedEarningsSmoothFactorUpdatedFilter),
    MaxFuturePoolsUpdatedFilter(MaxFuturePoolsUpdatedFilter),
}

impl ethers_core::abi::Tokenizable for FixedLenderEvents {
    fn from_token(token: ethers::abi::Token) -> Result<Self, ethers::abi::InvalidOutputType>
    where
        Self: Sized,
    {
        if let Ok(decoded) = DepositAtMaturityFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::DepositAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = WithdrawAtMaturityFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::WithdrawAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = BorrowAtMaturityFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::BorrowAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = RepayAtMaturityFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::RepayAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = LiquidateBorrowFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::LiquidateBorrowFilter(decoded));
        }
        if let Ok(decoded) = AssetSeizedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::AssetSeizedFilter(decoded));
        }
        if let Ok(decoded) = AccumulatedEarningsSmoothFactorUpdatedFilter::from_token(token.clone())
        {
            return Ok(FixedLenderEvents::AccumulatedEarningsSmoothFactorUpdatedFilter(decoded));
        }
        if let Ok(decoded) = MaxFuturePoolsUpdatedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::MaxFuturePoolsUpdatedFilter(decoded));
        }
        Err(ethers::abi::InvalidOutputType(String::from(
            "Event Unknown",
        )))
    }

    fn into_token(self) -> ethers::abi::Token {
        match self {
            FixedLenderEvents::DepositAtMaturityFilter(element) => element.into_token(),
            FixedLenderEvents::WithdrawAtMaturityFilter(element) => element.into_token(),
            FixedLenderEvents::BorrowAtMaturityFilter(element) => element.into_token(),
            FixedLenderEvents::RepayAtMaturityFilter(element) => element.into_token(),
            FixedLenderEvents::LiquidateBorrowFilter(element) => element.into_token(),
            FixedLenderEvents::AssetSeizedFilter(element) => element.into_token(),
            FixedLenderEvents::AccumulatedEarningsSmoothFactorUpdatedFilter(element) => {
                element.into_token()
            }
            FixedLenderEvents::MaxFuturePoolsUpdatedFilter(element) => element.into_token(),
        }
    }
}

impl ethers_core::abi::TokenizableItem for FixedLenderEvents {}

impl EthLogDecode for FixedLenderEvents {
    fn decode_log(log: &ethers::abi::RawLog) -> Result<Self, ethers::abi::Error>
    where
        Self: Sized,
    {
        if let Ok(decoded) = DepositAtMaturityFilter::decode_log(log) {
            return Ok(FixedLenderEvents::DepositAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = WithdrawAtMaturityFilter::decode_log(log) {
            return Ok(FixedLenderEvents::WithdrawAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = BorrowAtMaturityFilter::decode_log(log) {
            return Ok(FixedLenderEvents::BorrowAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = RepayAtMaturityFilter::decode_log(log) {
            return Ok(FixedLenderEvents::RepayAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = LiquidateBorrowFilter::decode_log(log) {
            return Ok(FixedLenderEvents::LiquidateBorrowFilter(decoded));
        }
        if let Ok(decoded) = AssetSeizedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::AssetSeizedFilter(decoded));
        }
        if let Ok(decoded) = AccumulatedEarningsSmoothFactorUpdatedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::AccumulatedEarningsSmoothFactorUpdatedFilter(decoded));
        }
        if let Ok(decoded) = MaxFuturePoolsUpdatedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::MaxFuturePoolsUpdatedFilter(decoded));
        }
        Err(ethers::abi::Error::InvalidData)
    }
}
