// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.16;

import { Test, stdJson } from "forge-std/Test.sol";
import { Market } from "@exactly-protocol/protocol/contracts/Market.sol";
import { Auditor } from "@exactly-protocol/protocol/contracts/Auditor.sol";
import { MockPriceFeed } from "@exactly-protocol/protocol/contracts/mocks/MockPriceFeed.sol";
import { ExactlyOracle, AggregatorV2V3Interface } from "@exactly-protocol/protocol/contracts/ExactlyOracle.sol";
import { Liquidator, IMarket, ERC20 } from "../contracts/Liquidator.sol";

contract LiquidatorTest is Test {
  using stdJson for string;

  address internal constant ALICE = address(0x420);

  Liquidator internal liquidator;
  ExactlyOracle internal oracle;
  Market internal marketUSDC;
  Market internal marketDAI;
  Auditor internal auditor;
  address internal timelock;

  function setUp() public {
    liquidator = new Liquidator(getAddress("UniswapV3Factory", ""));

    marketUSDC = Market(getAddress("MarketUSDC", "node_modules/@exactly-protocol/protocol/"));
    marketDAI = Market(getAddress("MarketDAI", "node_modules/@exactly-protocol/protocol/"));
    auditor = Auditor(getAddress("Auditor", "node_modules/@exactly-protocol/protocol/"));
    timelock = getAddress("TimelockController", "node_modules/@exactly-protocol/protocol/");
    oracle = ExactlyOracle(getAddress("ExactlyOracle", "node_modules/@exactly-protocol/protocol/"));
  }

  function testMultiMarketLiquidation() external {
    deal(address(marketUSDC.asset()), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    marketUSDC.asset().approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketDAI.borrow(70_000 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    oracle.setPriceFeed(marketDAI, AggregatorV2V3Interface(address(new MockPriceFeed(1.4e8))));
    vm.stopPrank();

    uint256 balanceBefore = marketUSDC.asset().balanceOf(address(liquidator));
    liquidator.liquidate(
      IMarket(address(marketDAI)),
      IMarket(address(marketUSDC)),
      ALICE,
      67_300 ether,
      address(marketUSDC.asset()),
      500
    );
    assertGt(marketUSDC.asset().balanceOf(address(liquidator)), balanceBefore);
  }

  function testSingleMarketLiquidation() external {
    deal(address(marketUSDC.asset()), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    marketUSDC.asset().approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketDAI.borrow(70_000 ether, ALICE, ALICE);
    marketUSDC.borrow(2_000e6, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    oracle.setPriceFeed(marketDAI, AggregatorV2V3Interface(address(new MockPriceFeed(1.4e8))));
    vm.stopPrank();

    liquidator.liquidate(
      IMarket(address(marketUSDC)),
      IMarket(address(marketUSDC)),
      ALICE,
      2_000e6,
      address(marketDAI.asset()),
      500
    );
    assertGt(marketUSDC.asset().balanceOf(address(liquidator)), 0);
  }

  function getAddress(string memory name, string memory base) internal returns (address) {
    string memory network;
    if (block.chainid == 4) network = "rinkeby";
    return vm.readFile(string.concat(base, "deployments/", network, "/", name, ".json")).readAddress(".address");
  }
}
