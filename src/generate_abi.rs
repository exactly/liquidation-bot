use ethers::prelude::abigen;

abigen!(
    MarketProtocol,
    "node_modules/@exactly-protocol/protocol/deployments/goerli/MarketWETH.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Previewer,
    "node_modules/@exactly-protocol/protocol/deployments/goerli/Previewer.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    PriceFeedWrapper,
    "node_modules/@exactly-protocol/protocol/deployments/goerli/PriceFeedwstETH.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Auditor,
    "node_modules/@exactly-protocol/protocol/deployments/goerli/Auditor.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    InterestRateModel,
    "node_modules/@exactly-protocol/protocol/deployments/goerli/InterestRateModelWETH.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    PriceFeed,
    "node_modules/@chainlink/contracts/abi/v0.8/AggregatorV2V3Interface.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    PriceFeedLido,
    "lib/abi/Lido.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    AggregatorProxy,
    "node_modules/@chainlink/contracts/abi/v0.7/AggregatorProxyInterface.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Liquidator,
    "deployments/goerli/Liquidator.json",
    event_derives(serde::Deserialize, serde::Serialize)
);
