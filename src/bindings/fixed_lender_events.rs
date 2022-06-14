use ethers::{
    contract::{self as ethers_contract},
    core::{
        self as ethers_core,
        types::{Address, H256, U256},
    },
    prelude::{k256::elliptic_curve::consts::U8, EthLogDecode},
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
    pub assets_discounted: U256,
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
    pub debt_covered: U256,
}

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
    pub collateral_fixed_lender: Address,
    pub seized_assets: U256,
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

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "AccumulatedEarningsSmoothFactorSet",
    abi = "AccumulatedEarningsSmoothFactorSet(uint256)"
)]
pub struct AccumulatedEarningsSmoothFactorSetFilter {
    pub new_accumulated_earnings_smooth_factor: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "SmartPoolEarningsAccrued",
    abi = "SmartPoolEarningsAccrued(uint256,uint256)"
)]
pub struct SmartPoolEarningsAccruedFilter {
    pub previous_assets: U256,
    pub earnings: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "MaxFuturePoolsUpdate", abi = "MaxFuturePoolsUpdate(uint256)")]
pub struct MaxFuturePoolsUpdatedFilter {
    pub new_max_future_pools: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Transfer", abi = "Transfer(address,address,uint256)")]
pub struct TransferFilter {
    #[ethevent(indexed)]
    pub from: Address,
    #[ethevent(indexed)]
    pub to: Address,
    pub amount: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Deposit", abi = "Deposit(address,address,uint256,uint256)")]
pub struct DepositFilter {
    #[ethevent(indexed)]
    pub caller: Address,
    #[ethevent(indexed)]
    pub owner: Address,
    pub assets: U256,
    pub shares: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "Withdraw",
    abi = "Withdraw(address,address,address,uint256,uint256)"
)]
pub struct WithdrawFilter {
    #[ethevent(indexed)]
    pub caller: Address,
    #[ethevent(indexed)]
    pub receiver: Address,
    #[ethevent(indexed)]
    pub owner: Address,
    pub assets: U256,
    pub shares: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Approval", abi = "Approval(address,address,uint256)")]
pub struct ApprovalFilter {
    #[ethevent(indexed)]
    pub owner: Address,
    #[ethevent(indexed)]
    pub spender: Address,
    pub amount: U256,
}

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

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "RoleAdminChanged",
    abi = "RoleAdminChanged(bytes32,bytes32,bytes32)"
)]
pub struct RoleAdminChangedFilter {
    #[ethevent(indexed)]
    pub role: H256,
    #[ethevent(indexed)]
    pub previous_admin_role: H256,
    #[ethevent(indexed)]
    pub new_admin_role: H256,
}

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

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Paused", abi = "Paused(address)")]
pub struct PausedFilter {
    pub account: Address,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "Unpaused", abi = "Unpaused(address)")]
pub struct UnpausedFilter {
    pub account: Address,
}
/////////////////////

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "MarketListed", abi = "MarketListed(address,uint8)")]
pub struct MarketListedFilter {
    pub fixed_lender: Address,
    pub decimals: u8,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "MarketEntered", abi = "MarketEntered(address,address)")]
pub struct MarketEnteredFilter {
    #[ethevent(indexed)]
    pub fixed_lender: Address,
    #[ethevent(indexed)]
    pub account: Address,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "MarketExited", abi = "MarketExited(address,address)")]
pub struct MarketExitedFilter {
    #[ethevent(indexed)]
    pub fixed_lender: Address,
    #[ethevent(indexed)]
    pub account: Address,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "OracleSet", abi = "OracleSet(address)")]
pub struct OracleSetFilter {
    pub new_oracle: Address,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "LiquidationINcentiveSet",
    abi = "LiquidationINcentiveSet(uint256)"
)]
pub struct LiquidationIncentiveSetFilter {
    pub new_liquidation_incentive: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "BorrowCapUpdated", abi = "BorrowCapUpdated(address,uint256)")]
pub struct BorrowCapUpdatedFilter {
    #[ethevent(indexed)]
    pub fixed_lender: Address,
    pub new_borrow_cap: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "AdjustFactorSet", abi = "AdjustFactorSet(address,uint256)")]
pub struct AdjustFactorSetFilter {
    #[ethevent(indexed)]
    pub fixed_lender: Address,
    pub new_adjust_factor: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "InterestRateModelSet", abi = "InterestRateModelSet(address)")]
pub struct InterestRateModelSetFilter {
    #[ethevent(indexed)]
    pub new_interest_rate_model: Address,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "PenaltyRateSet", abi = "PenaltyRateSet(uint256)")]
pub struct PenaltyRateSetFilter {
    pub new_penalty_rate: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(
    name = "SmartPoolReserveFactorSet",
    abi = "SmartPoolReserveFactorSet(uint256)"
)]
pub struct SmartPoolReserveFactorSetFilter {
    new_smart_pool_reserve_factor: U256,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, ethers_contract :: EthEvent)]
#[ethevent(name = "DampSpeedUpdated", abi = "DampSpeedUpdated(uint256,uint256)")]
pub struct DampSpeedUpdatedFilter {
    new_damp_speed_up: U256,
    new_damp_speed_down: U256,
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
    AccumulatedEarningsSmoothFactorSetFilter(AccumulatedEarningsSmoothFactorSetFilter),
    MaxFuturePoolsUpdatedFilter(MaxFuturePoolsUpdatedFilter),
    SmartPoolEarningsAccruedFilter(SmartPoolEarningsAccruedFilter),
    RoleGrantedFilter(RoleGrantedFilter),
    RoleAdminChangedFilter(RoleAdminChangedFilter),
    RoleRevokedFilter(RoleRevokedFilter),
    PausedFilter(PausedFilter),
    UnpausedFilter(UnpausedFilter),

    // Auditor events
    MarketListedFilter(MarketListedFilter),
    MarketEnteredFilter(MarketEnteredFilter),
    MarketExitedFilter(MarketExitedFilter),
    OracleSetFilter(OracleSetFilter),
    LiquidationIncentiveSetFilter(LiquidationIncentiveSetFilter),
    BorrowCapUpdatedFilter(BorrowCapUpdatedFilter),
    AdjustFactorSetFilter(AdjustFactorSetFilter),

    // PoolAccounting events
    InterestRateModelSetFilter(InterestRateModelSetFilter),
    PenaltyRateSetFilter(PenaltyRateSetFilter),
    SmartPoolReserveFactorSetFilter(SmartPoolReserveFactorSetFilter),
    DampSpeedUpdatedFilter(DampSpeedUpdatedFilter),
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
        if let Ok(decoded) = AccumulatedEarningsSmoothFactorSetFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::AccumulatedEarningsSmoothFactorSetFilter(
                decoded,
            ));
        }
        if let Ok(decoded) = MaxFuturePoolsUpdatedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::MaxFuturePoolsUpdatedFilter(decoded));
        }
        if let Ok(decoded) = SmartPoolEarningsAccruedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::SmartPoolEarningsAccruedFilter(decoded));
        }
        if let Ok(decoded) = PausedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::PausedFilter(decoded));
        }
        if let Ok(decoded) = UnpausedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::UnpausedFilter(decoded));
        }

        // Auditor events
        if let Ok(decoded) = MarketListedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::MarketListedFilter(decoded));
        }
        if let Ok(decoded) = MarketEnteredFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::MarketEnteredFilter(decoded));
        }
        if let Ok(decoded) = MarketExitedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::MarketExitedFilter(decoded));
        }
        if let Ok(decoded) = OracleSetFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::OracleSetFilter(decoded));
        }
        if let Ok(decoded) = LiquidationIncentiveSetFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::LiquidationIncentiveSetFilter(decoded));
        }
        if let Ok(decoded) = BorrowCapUpdatedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::BorrowCapUpdatedFilter(decoded));
        }
        if let Ok(decoded) = AdjustFactorSetFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::AdjustFactorSetFilter(decoded));
        }

        // PoolAccounting events
        if let Ok(decoded) = InterestRateModelSetFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::InterestRateModelSetFilter(decoded));
        }
        if let Ok(decoded) = PenaltyRateSetFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::PenaltyRateSetFilter(decoded));
        }
        if let Ok(decoded) = SmartPoolReserveFactorSetFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::SmartPoolReserveFactorSetFilter(decoded));
        }
        if let Ok(decoded) = DampSpeedUpdatedFilter::from_token(token.clone()) {
            return Ok(FixedLenderEvents::DampSpeedUpdatedFilter(decoded));
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
            FixedLenderEvents::AccumulatedEarningsSmoothFactorSetFilter(element) => {
                element.into_token()
            }
            FixedLenderEvents::MaxFuturePoolsUpdatedFilter(element) => element.into_token(),
            FixedLenderEvents::SmartPoolEarningsAccruedFilter(element) => element.into_token(),
            FixedLenderEvents::PausedFilter(element) => element.into_token(),
            FixedLenderEvents::UnpausedFilter(element) => element.into_token(),

            //Auditor events
            FixedLenderEvents::MarketListedFilter(element) => element.into_token(),
            FixedLenderEvents::MarketEnteredFilter(element) => element.into_token(),
            FixedLenderEvents::MarketExitedFilter(element) => element.into_token(),
            FixedLenderEvents::OracleSetFilter(element) => element.into_token(),
            FixedLenderEvents::LiquidationIncentiveSetFilter(element) => element.into_token(),
            FixedLenderEvents::BorrowCapUpdatedFilter(element) => element.into_token(),
            FixedLenderEvents::AdjustFactorSetFilter(element) => element.into_token(),

            //PoolAccounting events
            FixedLenderEvents::InterestRateModelSetFilter(element) => element.into_token(),
            FixedLenderEvents::PenaltyRateSetFilter(element) => element.into_token(),
            FixedLenderEvents::SmartPoolReserveFactorSetFilter(element) => element.into_token(),
            FixedLenderEvents::DampSpeedUpdatedFilter(element) => element.into_token(),
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
        if let Ok(decoded) = AccumulatedEarningsSmoothFactorSetFilter::decode_log(log) {
            return Ok(FixedLenderEvents::AccumulatedEarningsSmoothFactorSetFilter(
                decoded,
            ));
        }
        if let Ok(decoded) = MaxFuturePoolsUpdatedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::MaxFuturePoolsUpdatedFilter(decoded));
        }
        if let Ok(decoded) = SmartPoolEarningsAccruedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::SmartPoolEarningsAccruedFilter(decoded));
        }
        if let Ok(decoded) = PausedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::PausedFilter(decoded));
        }
        if let Ok(decoded) = UnpausedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::UnpausedFilter(decoded));
        }

        // Auditor events
        if let Ok(decoded) = MarketListedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::MarketListedFilter(decoded));
        }
        if let Ok(decoded) = MarketEnteredFilter::decode_log(log) {
            return Ok(FixedLenderEvents::MarketEnteredFilter(decoded));
        }
        if let Ok(decoded) = MarketExitedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::MarketExitedFilter(decoded));
        }
        if let Ok(decoded) = OracleSetFilter::decode_log(log) {
            return Ok(FixedLenderEvents::OracleSetFilter(decoded));
        }
        if let Ok(decoded) = LiquidationIncentiveSetFilter::decode_log(log) {
            return Ok(FixedLenderEvents::LiquidationIncentiveSetFilter(decoded));
        }
        if let Ok(decoded) = BorrowCapUpdatedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::BorrowCapUpdatedFilter(decoded));
        }
        if let Ok(decoded) = AdjustFactorSetFilter::decode_log(log) {
            return Ok(FixedLenderEvents::AdjustFactorSetFilter(decoded));
        }

        // PoolAccounting events
        if let Ok(decoded) = InterestRateModelSetFilter::decode_log(log) {
            return Ok(FixedLenderEvents::InterestRateModelSetFilter(decoded));
        }
        if let Ok(decoded) = PenaltyRateSetFilter::decode_log(log) {
            return Ok(FixedLenderEvents::PenaltyRateSetFilter(decoded));
        }
        if let Ok(decoded) = SmartPoolReserveFactorSetFilter::decode_log(log) {
            return Ok(FixedLenderEvents::SmartPoolReserveFactorSetFilter(decoded));
        }
        if let Ok(decoded) = DampSpeedUpdatedFilter::decode_log(log) {
            return Ok(FixedLenderEvents::DampSpeedUpdatedFilter(decoded));
        }
        println!("Missing event: {:?}", log);
        Err(ethers::abi::Error::InvalidData)
    }
}
