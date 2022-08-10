# Liquidation bot

## Dependencies

- Rust;
- Node.

The liquidation bot is written in Rust; therefore, it must be installed and set up on the machine.

## How to install and run it

Cloning the project:

```shell
git clone git@github.com:exactly-protocol/liquidation-bot.git
```

Deploying flash loan contracts:

```shell
npm i --legacy-peer-deps
npx hardhat --network <network> deploy
```

Running the project:

A `.env` file should be created on the root directory of the project with the following parameters:

```env
CHAIN_ID=[ID of the chain used]
ETH_[CHAIN_NAME]_PROVIDER=[Link to the RPC provider]
PRIVATE_KEY=[WALLET PRIVATE KEY]
```

After the `.env` file has been created, run the project with the command:

```shell
cargo run
```

## How it works

### The Bot

The bot works by remounting the users' positions using the protocol's emitted events with the minimum number of calls directly to the contracts. This makes the bot more efficient in recreating such states.

After the bot connects to the RPC provider through WebSocket, it subscribes to receive the events stream.

Each one of those events is parsed and transcribed into the user's data.

Whenever there's an idle moment on receiving new events, the bot does a check for liquidations.

If a user is in a state to be liquidated (with a health factor less than 1), the flash loan contract's liquidation function is called.

To avoid reentrancy issues on Uniswap's contract, the bot must pick a pair of assets that is NOT the same pair of debt/collateral.

The debt is liquidated.

The bot liquidates the user's debt seizing their collateral with the highest value.

All the users that could be liquidated will be.
After the liquidations, the bot returns to wait for more events and recreate the user's positions.

### Flash loan contract

The flash loan contract calls protocol's liquidation function.

It checks its amount on the specific debt asset available on contract's balance to repay the user's debt. In case it has less than the amount needed to liquidate the user, it does as follow:

1. Borrow on Uniswap V3 the difference between what it has and the user's debt;
1. Waits for a callback notifying it that the amount was received;
1. Repays the debt;
1. Receive the collateral;
1. Swaps it to the same as the user's debt;
1. Repays Uniswap.

## Structure

The project is structured as follows:

- main.rs

    Setup the bot to connect correctly to RPC Provider.

    Starts the service and handles most of the errors.

- service.rs

    It's where most of the tasks are executed.

  - A subscription to the event's stream is made on the main thread;

  - Each event received is parsed;

  - Positions are created;

  - A debounce for idleness is made in another thread;

  - When the bot is idle for enough time, this thread checks for liquidations;

- borrower.rs

    This structure is used to store users' data.

- exactly_oracle.rs

    Helper to access price protocol used by Exactly's protocol.

- fixed_lender.rs

    Stores updated information created by the protocol's emitted events about all the markets.

- exactly_events.rs

    Redirect the events to suitable structures.

- config.rs

    Handle environment variables such as RPC provider link access, wallet's private key, etc.
