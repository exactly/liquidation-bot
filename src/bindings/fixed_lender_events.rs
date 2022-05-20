use ethers::{
    contract::{self as ethers_contract},
    core::{
        self as ethers_core,
        types::{Address, H256, U256},
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

#[allow(non_snake_case)]
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

#[allow(non_snake_case)]
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

#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "LiquidateBorrow",
    abi = "LiquidateBorrow(address,address,uint256,address,uint256)"
)]
pub struct LiquidateBorrowFilter {
    #[ethevent(indexed)]
    pub receiver: Address,
    #[ethevent(indexed)]
    pub borrower: Address,
    pub assets: U256,
    #[ethevent(indexed)]
    pub collateralFixedLender: Address,
    pub seizedAssets: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "AssetSeized", abi = "AssetSeized(address,address,uint256)")]
pub struct AssetSeizedFilter {
    #[ethevent(indexed)]
    pub liquidator: Address,
    #[ethevent(indexed)]
    pub borrower: Address,
    pub assets: U256,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "AccumulatedEarningsSmoothFactorUpdated",
    abi = "AccumulatedEarningsSmoothFactorUpdated(uint256)"
)]
pub struct AccumulatedEarningsSmoothFactorUpdatedFilter {
    pub newAccumulatedEarningsSmoothFactor: U256,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "SmartPoolEarningsAccrued",
    abi = "SmartPoolEarningsAccrued(uint256,uint256)"
)]
pub struct SmartPoolEarningsAccruedFilter {
    pub previousAssets: U256,
    pub earnings: U256,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "MaxFuturePoolsUpdate", abi = "MaxFuturePoolsUpdate(uint256)")]
pub struct MaxFuturePoolsUpdatedFilter {
    pub newMaxFuturePools: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Transfer", abi = "Transfer(address,address,uint256)")]
pub struct TransferFilter {
    #[ethevent(indexed)]
    from: Address,
    #[ethevent(indexed)]
    to: Address,
    amount: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Deposit", abi = "Deposit(address,address,uint256,uint256)")]
pub struct DepositFilter {
    #[ethevent(indexed)]
    caller: Address,
    #[ethevent(indexed)]
    owner: Address,
    assets: U256,
    shares: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "Withdraw",
    abi = "Withdraw(address,address,address,uint256,uint256)"
)]
pub struct WithdrawFilter {
    #[ethevent(indexed)]
    caller: Address,
    #[ethevent(indexed)]
    receiver: Address,
    #[ethevent(indexed)]
    owner: Address,
    assets: U256,
    shares: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Approval", abi = "Approval(address,address,uint256)")]
pub struct ApprovalFilter {
    #[ethevent(indexed)]
    owner: Address,
    #[ethevent(indexed)]
    spender: Address,
    amount: U256,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "RoleGranted", abi = "RoleGranted(bytes32,address,address)")]
pub struct RoleGrantedFilter {
    #[ethevent(indexed)]
    pub role: H256,
    #[ethevent(indexed)]
    pub account: Address,
    #[ethevent(indexed)]
    pub sender: Address,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "RoleAdminChanged",
    abi = "RoleAdminChanged(bytes32,bytes32,bytes32)"
)]
pub struct RoleAdminChangedFilter {
    #[ethevent(indexed)]
    pub role: H256,
    #[ethevent(indexed)]
    pub previousAdminRole: H256,
    #[ethevent(indexed)]
    pub newAdminRole: H256,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "RoleRevoked", abi = "RoleRevoked(bytes32,address,address)")]
pub struct RoleRevokedFilter {
    #[ethevent(indexed)]
    pub role: H256,
    #[ethevent(indexed)]
    pub account: Address,
    #[ethevent(indexed)]
    pub sender: Address,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixedLenderEvents {
    TransferFilter(TransferFilter),
    DepositFilter(DepositFilter),
    WithdrawFilter(WithdrawFilter),
    ApprovalFilter(ApprovalFilter),
    DepositAtMaturityFilter(DepositAtMaturityFilter),
    WithdrawAtMaturityFilter(WithdrawAtMaturityFilter),
    BorrowAtMaturityFilter(BorrowAtMaturityFilter),
    RepayAtMaturityFilter(RepayAtMaturityFilter),
    LiquidateBorrowFilter(LiquidateBorrowFilter),
    AssetSeizedFilter(AssetSeizedFilter),
    AccumulatedEarningsSmoothFactorUpdatedFilter(AccumulatedEarningsSmoothFactorUpdatedFilter),
    MaxFuturePoolsUpdatedFilter(MaxFuturePoolsUpdatedFilter),
    SmartPoolEarningsAccruedFilter(SmartPoolEarningsAccruedFilter),
    RoleGrantedFilter(RoleGrantedFilter),
    RoleAdminChangedFilter(RoleAdminChangedFilter),
    RoleRevokedFilter(RoleRevokedFilter),
}

impl ethers_core::abi::Tokenizable for FixedLenderEvents {
    fn from_token(token: ethers::abi::Token) -> Result<Self, ethers::abi::InvalidOutputType>
    where
        Self: Sized,
    {
        if let Ok(decoded) = RoleGrantedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::RoleGrantedFilter(decoded));
        }
        if let Ok(decoded) = RoleAdminChangedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::RoleAdminChangedFilter(decoded));
        }
        if let Ok(decoded) = RoleGrantedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::RoleGrantedFilter(decoded));
        }
        if let Ok(decoded) = TransferFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::TransferFilter(decoded));
        }
        if let Ok(decoded) = DepositFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::DepositFilter(decoded));
        }
        if let Ok(decoded) = WithdrawFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::WithdrawFilter(decoded));
        }
        if let Ok(decoded) = ApprovalFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::ApprovalFilter(decoded));
        }
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
        if let Ok(decoded) = SmartPoolEarningsAccruedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::SmartPoolEarningsAccruedFilter(decoded));
        }
        Err(ethers::abi::InvalidOutputType(String::from(
            "Event Unknown",
        )))
    }

    fn into_token(self) -> ethers::abi::Token {
        match self {
            FixedLenderEvents::RoleGrantedFilter(element) => element.into_token(),
            FixedLenderEvents::RoleAdminChangedFilter(element) => element.into_token(),
            FixedLenderEvents::RoleRevokedFilter(element) => element.into_token(),
            FixedLenderEvents::TransferFilter(element) => element.into_token(),
            FixedLenderEvents::DepositFilter(element) => element.into_token(),
            FixedLenderEvents::WithdrawFilter(element) => element.into_token(),
            FixedLenderEvents::ApprovalFilter(element) => element.into_token(),
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
            FixedLenderEvents::SmartPoolEarningsAccruedFilter(element) => element.into_token(),
        }
    }
}

impl ethers_core::abi::TokenizableItem for FixedLenderEvents {}

impl EthLogDecode for FixedLenderEvents {
    fn decode_log(log: &ethers::abi::RawLog) -> Result<Self, ethers::abi::Error>
    where
        Self: Sized,
    {
        if let Ok(decoded) = RoleGrantedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::RoleGrantedFilter(decoded));
        }
        if let Ok(decoded) = RoleAdminChangedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::RoleAdminChangedFilter(decoded));
        }
        if let Ok(decoded) = RoleRevokedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::RoleRevokedFilter(decoded));
        }

        if let Ok(decoded) = TransferFilter::decode_log(log) {
            return Ok(FixedLenderEvents::TransferFilter(decoded));
        }
        if let Ok(decoded) = DepositFilter::decode_log(log) {
            return Ok(FixedLenderEvents::DepositFilter(decoded));
        }
        if let Ok(decoded) = WithdrawFilter::decode_log(log) {
            return Ok(FixedLenderEvents::WithdrawFilter(decoded));
        }
        if let Ok(decoded) = ApprovalFilter::decode_log(log) {
            return Ok(FixedLenderEvents::ApprovalFilter(decoded));
        }
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
        if let Ok(decoded) = SmartPoolEarningsAccruedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::SmartPoolEarningsAccruedFilter(decoded));
        }
        println!("Missing event: {:?}", log);
        Err(ethers::abi::Error::InvalidData)
    }
}
