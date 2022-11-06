use ethers::{
    abi::{Error, RawLog},
    prelude::EthLogDecode,
    types::H256,
};
use std::str::FromStr;

use crate::protocol::{
    auditor::{
        AdminChangedFilter, InitializedFilter, RoleAdminChangedFilter, RoleGrantedFilter,
        RoleRevokedFilter, UpgradedFilter,
    },
    price_feed::{AnswerUpdatedFilter, NewRoundFilter},
    AccumulatorAccrualFilter, AdjustFactorSetFilter, BackupFeeRateSetFilter,
    BorrowAtMaturityFilter, BorrowFilter, DampSpeedSetFilter, DepositAtMaturityFilter,
    DepositFilter, EarningsAccumulatorSmoothFactorSetFilter, FixedEarningsUpdateFilter,
    FloatingDebtUpdateFilter, InterestRateModelSetFilter, LiquidateFilter,
    LiquidationIncentiveSetFilter, MarketEnteredFilter, MarketExitedFilter, MarketListedFilter,
    MarketUpdateFilter, MaxFuturePoolsSetFilter, PausedFilter, PenaltyRateSetFilter,
    PriceFeedSetFilter, ProtocolContactsSetFilter, RepayAtMaturityFilter, RepayFilter,
    ReserveFactorSetFilter, ResumedFilter, SeizeFilter, StakingLimitRemovedFilter,
    StakingLimitSetFilter, StakingPausedFilter, StakingResumedFilter, StoppedFilter,
    TreasurySetFilter, UnpausedFilter, WithdrawAtMaturityFilter, WithdrawFilter,
    WithdrawalCredentialsSetFilter, WithdrawalFilter,
};
use crate::protocol::{market_protocol::ApprovalFilter, ElrewardsReceivedFilter};
use crate::protocol::{
    market_protocol::TransferFilter, ElrewardsVaultSetFilter, ElrewardsWithdrawalLimitSetFilter,
    FeeDistributionSetFilter, FeeSetFilter, RecoverToVaultFilter, ScriptResultFilter,
    SharesBurntFilter, SubmittedFilter, TransferSharesFilter, UnbufferedFilter,
};
use aggregator_mod::NewTransmissionFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactlyEvents {
    TransferFilter(TransferFilter),
    DepositFilter(DepositFilter),
    WithdrawFilter(WithdrawFilter),
    ApprovalFilter(ApprovalFilter),
    DepositAtMaturityFilter(DepositAtMaturityFilter),
    WithdrawAtMaturityFilter(WithdrawAtMaturityFilter),
    BorrowAtMaturityFilter(BorrowAtMaturityFilter),
    RepayAtMaturityFilter(RepayAtMaturityFilter),
    LiquidateFilter(LiquidateFilter),
    SeizeFilter(SeizeFilter),
    EarningsAccumulatorSmoothFactorSetFilter(EarningsAccumulatorSmoothFactorSetFilter),
    MaxFuturePoolsSetFilter(MaxFuturePoolsSetFilter),
    TreasurySetFilter(TreasurySetFilter),
    RoleGrantedFilter(RoleGrantedFilter),
    RoleAdminChangedFilter(RoleAdminChangedFilter),
    RoleRevokedFilter(RoleRevokedFilter),
    PausedFilter(PausedFilter),
    UnpausedFilter(UnpausedFilter),
    MarketUpdateFilter(MarketUpdateFilter),
    FixedEarningsUpdateFilter(FixedEarningsUpdateFilter),
    AccumulatorAccrualFilter(AccumulatorAccrualFilter),
    FloatingDebtUpdateFilter(FloatingDebtUpdateFilter),
    BorrowFilter(BorrowFilter),
    RepayFilter(RepayFilter),
    BackupFeeRateSetFilter(BackupFeeRateSetFilter),

    // Auditor events
    MarketListedFilter(MarketListedFilter),
    MarketEnteredFilter(MarketEnteredFilter),
    MarketExitedFilter(MarketExitedFilter),
    LiquidationIncentiveSetFilter(LiquidationIncentiveSetFilter),
    AdjustFactorSetFilter(AdjustFactorSetFilter),
    UpgradedFilter(UpgradedFilter),
    InitializedFilter(InitializedFilter),
    AdminChangedFilter(AdminChangedFilter),

    // PoolAccounting events
    InterestRateModelSetFilter(InterestRateModelSetFilter),
    PenaltyRateSetFilter(PenaltyRateSetFilter),
    ReserveFactorSetFilter(ReserveFactorSetFilter),
    DampSpeedSetFilter(DampSpeedSetFilter),

    // ExactlyOracle events
    PriceFeedSetFilter(PriceFeedSetFilter),
    // PriceFeed
    AnswerUpdatedFilter(AnswerUpdatedFilter),
    NewRoundFilter(NewRoundFilter),
    NewTransmissionFilter(NewTransmissionFilter),

    UpdateLidoPriceFilter,

    IgnoredFilter,
}

macro_rules! map_filter {
    ($ext_filter:ident, $exactly_filter:ident, $log:ident) => {
        if let Ok(_) = $ext_filter::decode_log($log) {
            return Ok(ExactlyEvents::$exactly_filter);
        }
    };
}

impl EthLogDecode for ExactlyEvents {
    fn decode_log(log: &RawLog) -> Result<Self, Error>
    where
        Self: Sized,
    {
        if let Ok(decoded) = RoleGrantedFilter::decode_log(log) {
            return Ok(ExactlyEvents::RoleGrantedFilter(decoded));
        }
        if let Ok(decoded) = RoleAdminChangedFilter::decode_log(log) {
            return Ok(ExactlyEvents::RoleAdminChangedFilter(decoded));
        }
        if let Ok(decoded) = RoleRevokedFilter::decode_log(log) {
            return Ok(ExactlyEvents::RoleRevokedFilter(decoded));
        }
        if let Ok(decoded) = TransferFilter::decode_log(log) {
            return Ok(ExactlyEvents::TransferFilter(decoded));
        }
        if let Ok(decoded) = DepositFilter::decode_log(log) {
            return Ok(ExactlyEvents::DepositFilter(decoded));
        }
        if let Ok(decoded) = WithdrawFilter::decode_log(log) {
            return Ok(ExactlyEvents::WithdrawFilter(decoded));
        }
        if let Ok(decoded) = ApprovalFilter::decode_log(log) {
            return Ok(ExactlyEvents::ApprovalFilter(decoded));
        }
        if let Ok(decoded) = DepositAtMaturityFilter::decode_log(log) {
            return Ok(ExactlyEvents::DepositAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = WithdrawAtMaturityFilter::decode_log(log) {
            return Ok(ExactlyEvents::WithdrawAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = BorrowAtMaturityFilter::decode_log(log) {
            return Ok(ExactlyEvents::BorrowAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = RepayAtMaturityFilter::decode_log(log) {
            return Ok(ExactlyEvents::RepayAtMaturityFilter(decoded));
        }
        if let Ok(decoded) = LiquidateFilter::decode_log(log) {
            return Ok(ExactlyEvents::LiquidateFilter(decoded));
        }
        if let Ok(decoded) = SeizeFilter::decode_log(log) {
            return Ok(ExactlyEvents::SeizeFilter(decoded));
        }
        if let Ok(decoded) = EarningsAccumulatorSmoothFactorSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::EarningsAccumulatorSmoothFactorSetFilter(
                decoded,
            ));
        }
        if let Ok(decoded) = MaxFuturePoolsSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::MaxFuturePoolsSetFilter(decoded));
        }
        if let Ok(decoded) = PausedFilter::decode_log(log) {
            return Ok(ExactlyEvents::PausedFilter(decoded));
        }
        if let Ok(decoded) = UnpausedFilter::decode_log(log) {
            return Ok(ExactlyEvents::UnpausedFilter(decoded));
        }
        if let Ok(decoded) = MarketUpdateFilter::decode_log(log) {
            return Ok(ExactlyEvents::MarketUpdateFilter(decoded));
        }
        if let Ok(decoded) = FixedEarningsUpdateFilter::decode_log(log) {
            return Ok(ExactlyEvents::FixedEarningsUpdateFilter(decoded));
        }
        if let Ok(decoded) = AccumulatorAccrualFilter::decode_log(log) {
            return Ok(ExactlyEvents::AccumulatorAccrualFilter(decoded));
        }
        if let Ok(decoded) = FloatingDebtUpdateFilter::decode_log(log) {
            return Ok(ExactlyEvents::FloatingDebtUpdateFilter(decoded));
        }
        if let Ok(decoded) = TreasurySetFilter::decode_log(log) {
            return Ok(ExactlyEvents::TreasurySetFilter(decoded));
        }
        if let Ok(decoded) = BorrowFilter::decode_log(log) {
            return Ok(ExactlyEvents::BorrowFilter(decoded));
        }
        if let Ok(decoded) = RepayFilter::decode_log(log) {
            return Ok(ExactlyEvents::RepayFilter(decoded));
        }
        if let Ok(decoded) = BackupFeeRateSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::BackupFeeRateSetFilter(decoded));
        }

        // Auditor events
        if let Ok(decoded) = MarketListedFilter::decode_log(log) {
            return Ok(ExactlyEvents::MarketListedFilter(decoded));
        }
        if let Ok(decoded) = MarketEnteredFilter::decode_log(log) {
            return Ok(ExactlyEvents::MarketEnteredFilter(decoded));
        }
        if let Ok(decoded) = MarketExitedFilter::decode_log(log) {
            return Ok(ExactlyEvents::MarketExitedFilter(decoded));
        }
        if let Ok(decoded) = LiquidationIncentiveSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::LiquidationIncentiveSetFilter(decoded));
        }
        if let Ok(decoded) = AdjustFactorSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::AdjustFactorSetFilter(decoded));
        }
        if let Ok(decoded) = AdminChangedFilter::decode_log(log) {
            return Ok(ExactlyEvents::AdminChangedFilter(decoded));
        }
        if let Ok(decoded) = UpgradedFilter::decode_log(log) {
            return Ok(ExactlyEvents::UpgradedFilter(decoded));
        }
        if let Ok(decoded) = InitializedFilter::decode_log(log) {
            return Ok(ExactlyEvents::InitializedFilter(decoded));
        }

        // PoolAccounting events
        if let Ok(decoded) = InterestRateModelSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::InterestRateModelSetFilter(decoded));
        }
        if let Ok(decoded) = PenaltyRateSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::PenaltyRateSetFilter(decoded));
        }
        if let Ok(decoded) = ReserveFactorSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::ReserveFactorSetFilter(decoded));
        }
        if let Ok(decoded) = DampSpeedSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::DampSpeedSetFilter(decoded));
        }

        // ExactlyOracle events
        if let Ok(decoded) = PriceFeedSetFilter::decode_log(log) {
            return Ok(ExactlyEvents::PriceFeedSetFilter(decoded));
        }

        // PriceFeed
        if let Ok(decoded) = AnswerUpdatedFilter::decode_log(log) {
            return Ok(ExactlyEvents::AnswerUpdatedFilter(decoded));
        }

        if let Ok(decoded) = NewRoundFilter::decode_log(log) {
            return Ok(ExactlyEvents::NewRoundFilter(decoded));
        }

        if let Ok(decoded) = NewTransmissionFilter::decode_log(log) {
            return Ok(ExactlyEvents::NewTransmissionFilter(decoded));
        }

        // Lido events to update price
        map_filter!(ElrewardsReceivedFilter, UpdateLidoPriceFilter, log);
        map_filter!(ElrewardsVaultSetFilter, UpdateLidoPriceFilter, log);
        map_filter!(
            ElrewardsWithdrawalLimitSetFilter,
            UpdateLidoPriceFilter,
            log
        );
        map_filter!(FeeDistributionSetFilter, UpdateLidoPriceFilter, log);
        map_filter!(FeeSetFilter, UpdateLidoPriceFilter, log);
        map_filter!(RecoverToVaultFilter, UpdateLidoPriceFilter, log);
        map_filter!(ScriptResultFilter, UpdateLidoPriceFilter, log);
        map_filter!(SharesBurntFilter, UpdateLidoPriceFilter, log);
        map_filter!(UnbufferedFilter, UpdateLidoPriceFilter, log);
        map_filter!(WithdrawalFilter, UpdateLidoPriceFilter, log);

        map_filter!(SubmittedFilter, IgnoredFilter, log);
        map_filter!(TransferSharesFilter, IgnoredFilter, log);
        map_filter!(ResumedFilter, IgnoredFilter, log);
        map_filter!(ProtocolContactsSetFilter, IgnoredFilter, log);
        map_filter!(StakingLimitRemovedFilter, IgnoredFilter, log);
        map_filter!(StakingLimitSetFilter, IgnoredFilter, log);
        map_filter!(StakingPausedFilter, IgnoredFilter, log);
        map_filter!(StakingResumedFilter, IgnoredFilter, log);
        map_filter!(StoppedFilter, IgnoredFilter, log);
        map_filter!(WithdrawalCredentialsSetFilter, IgnoredFilter, log);

        let ignored_events: Vec<H256> = [
            "0xe8ec50e5150ae28ae37e493ff389ffab7ffaec2dc4dccfca03f12a3de29d12b2",
            "0xd0d9486a2c673e2a4b57fc82e4c8a556b3e2b82dd5db07e2c04a920ca0f469b6",
            "0xd0b1dac935d85bd54cf0a33b0d41d39f8cf53a968465fc7ea2377526b8ac712c",
            "0x25d719d88a4512dd76c7442b910a83360845505894eb444ef299409e180f8fb9", // ConfigSet(uint32,uint64,address[],address[],uint8,uint64,bytes)
            "0x3ea16a923ff4b1df6526e854c9e3a995c43385d70e73359e10623c74f0b52037", // RoundRequested(address,bytes16,uint32,uint8)
        ]
        .iter()
        .map(|x| H256::from_str(x).unwrap())
        .collect();
        if log
            .topics
            .iter()
            .find(|topic| {
                ignored_events
                    .iter()
                    .find(|ignored_topic| topic == ignored_topic)
                    .is_some()
            })
            .is_some()
        {
            return Ok(ExactlyEvents::IgnoredFilter);
        };

        println!("Missing event: {:?}", log);
        Err(Error::InvalidData)
    }
}

mod aggregator_mod {
    use ethers::{
        prelude::{EthDisplay, EthEvent},
        types::{Address, Bytes, I256},
    };

    #[derive(
        Clone,
        Debug,
        Default,
        Eq,
        PartialEq,
        EthEvent,
        EthDisplay,
        serde::Deserialize,
        serde::Serialize,
    )]
    #[ethevent(
        name = "NewTransmission",
        abi = "NewTransmission(uint32,int192,address,int192[],bytes,bytes32)"
    )]
    pub struct NewTransmissionFilter {
        #[ethevent(indexed)]
        pub aggregator_round_id: u32,
        pub answer: I256,
        pub transmitter: Address,
        pub observations: Vec<I256>,
        pub observers: Bytes,
        pub raw_report_context: [u8; 32],
    }
}
