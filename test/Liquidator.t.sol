// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.16;

import { Test, stdJson } from "forge-std/Test.sol";
import { Market } from "@exactly-protocol/protocol/contracts/Market.sol";
import { Auditor } from "@exactly-protocol/protocol/contracts/Auditor.sol";
import { MockPriceFeed } from "@exactly-protocol/protocol/contracts/mocks/MockPriceFeed.sol";
import { ExactlyOracle, AggregatorV2V3Interface } from "@exactly-protocol/protocol/contracts/ExactlyOracle.sol";
import { Liquidator, IMarket, ERC20, PoolAddress } from "../contracts/Liquidator.sol";

contract LiquidatorTest is Test {
  using stdJson for string;

  address internal constant ALICE = address(0x420);

  Liquidator internal liquidator;
  ExactlyOracle internal oracle;
  Market internal marketUSDC;
  Market internal marketDAI;
  Auditor internal auditor;
  address internal timelock;
  ERC20 internal usdc;
  ERC20 internal dai;

  function setUp() public {
    liquidator = new Liquidator(getAddress("UniswapV3Factory", ""));

    dai = ERC20(getAddress("DAI", "node_modules/@exactly-protocol/protocol/"));
    usdc = ERC20(getAddress("USDC", "node_modules/@exactly-protocol/protocol/"));
    oracle = ExactlyOracle(getAddress("ExactlyOracle", "node_modules/@exactly-protocol/protocol/"));
    oracle = ExactlyOracle(getAddress("ExactlyOracle", "node_modules/@exactly-protocol/protocol/"));
    auditor = Auditor(getAddress("Auditor", "node_modules/@exactly-protocol/protocol/"));
    timelock = getAddress("TimelockController", "node_modules/@exactly-protocol/protocol/");
    marketDAI = Market(getAddress("MarketDAI", "node_modules/@exactly-protocol/protocol/"));
    marketUSDC = Market(getAddress("MarketUSDC", "node_modules/@exactly-protocol/protocol/"));

    vm.label(ALICE, "alice");
    if (address(dai) != address(0)) {
      vm.label(
        PoolAddress.computeAddress(liquidator.factory(), PoolAddress.getPoolKey(address(dai), address(usdc), 500)),
        "DAI/USDC/500"
      );
    }
  }

  function testMultiMarketLiquidation() external {
    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketDAI.borrow(70_000 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    oracle.setPriceFeed(marketDAI, AggregatorV2V3Interface(address(new MockPriceFeed(1.4e8))));
    vm.stopPrank();

    uint256 balanceBefore = usdc.balanceOf(address(liquidator));
    liquidator.liquidate(
      IMarket(address(marketDAI)),
      IMarket(address(marketUSDC)),
      ALICE,
      67_300 ether,
      address(usdc),
      500
    );
    assertGt(usdc.balanceOf(address(liquidator)), balanceBefore);
  }

  function testSingleMarketLiquidation() external {
    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketDAI.borrow(70_000 ether, ALICE, ALICE);
    marketUSDC.borrow(2_000e6, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    oracle.setPriceFeed(marketDAI, AggregatorV2V3Interface(address(new MockPriceFeed(1.4e8))));
    vm.stopPrank();

    liquidator.liquidate(IMarket(address(marketUSDC)), IMarket(address(marketUSDC)), ALICE, 2_000e6, address(dai), 500);
    assertGt(usdc.balanceOf(address(liquidator)), 0);
  }

  function testTransfer() external {
    deal(address(usdc), address(liquidator), 666_666e6);
    assertEq(usdc.balanceOf(address(this)), 0);
    assertEq(usdc.balanceOf(address(liquidator)), 666_666e6);

    liquidator.transfer(usdc, address(this), 666_666e6);

    assertEq(usdc.balanceOf(address(this)), 666_666e6);
    assertEq(usdc.balanceOf(address(liquidator)), 0);
  }

  function getAddress(string memory name, string memory base) internal returns (address addr) {
    if (block.chainid == 31337) return address(0);

    string memory network;
    if (block.chainid == 1) network = "mainnet";
    else if (block.chainid == 4) network = "rinkeby";
    else if (block.chainid == 5) network = "goerli";

    addr = vm.readFile(string.concat(base, "deployments/", network, "/", name, ".json")).readAddress(".address");
    vm.label(addr, name);
  }
}
