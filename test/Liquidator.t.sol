// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.17;

import { Test, stdJson } from "forge-std/Test.sol";
import { Market } from "@exactly-protocol/protocol/contracts/Market.sol";
import { Auditor } from "@exactly-protocol/protocol/contracts/Auditor.sol";
import { MockPriceFeed } from "@exactly-protocol/protocol/contracts/mocks/MockPriceFeed.sol";
import { Liquidator, IMarket, ERC20, ISwapRouter, PoolAddress } from "../contracts/Liquidator.sol";

contract LiquidatorTest is Test {
  using stdJson for string;

  address internal constant ALICE = address(0x420);
  address internal constant BOB = address(0x069);

  Liquidator internal liquidator;
  Market internal marketDAI;
  Market internal marketUSDC;
  Market internal marketWBTC;
  Market internal marketwstETH;
  Auditor internal auditor;
  address internal timelock;
  ERC20 internal usdc;
  ERC20 internal dai;
  ERC20 internal weth;
  ERC20 internal wstETH;
  ERC20 internal wbtc;

  function setUp() public {
    liquidator = new Liquidator(
      address(this),
      getAddress("UniswapV3Factory", ""),
      ISwapRouter(getAddress("UniswapV3Router", ""))
    );

    dai = ERC20(getAddress("DAI", "node_modules/@exactly-protocol/protocol/"));
    usdc = ERC20(getAddress("USDC", "node_modules/@exactly-protocol/protocol/"));
    weth = ERC20(getAddress("WETH", "node_modules/@exactly-protocol/protocol/"));
    wbtc = ERC20(getAddress("WBTC", "node_modules/@exactly-protocol/protocol/"));
    wstETH = ERC20(getAddress("wstETH", "node_modules/@exactly-protocol/protocol/"));
    auditor = Auditor(getAddress("Auditor", "node_modules/@exactly-protocol/protocol/"));
    timelock = getAddress("TimelockController", "node_modules/@exactly-protocol/protocol/");
    marketDAI = Market(getAddress("MarketDAI", "node_modules/@exactly-protocol/protocol/"));
    marketUSDC = Market(getAddress("MarketUSDC", "node_modules/@exactly-protocol/protocol/"));
    marketWBTC = Market(getAddress("MarketWBTC", "node_modules/@exactly-protocol/protocol/"));
    marketwstETH = Market(getAddress("MarketwstETH", "node_modules/@exactly-protocol/protocol/"));

    vm.label(ALICE, "alice");
    if (address(dai) != address(0)) {
      vm.label(
        PoolAddress.computeAddress(liquidator.factory(), PoolAddress.getPoolKey(address(dai), address(usdc), 500)),
        "DAI/USDC/500"
      );
    }
  }

  function testMultiMarketLiquidation() external {
    deal(address(dai), BOB, 100_000 ether);
    vm.startPrank(BOB);
    dai.approve(address(marketDAI), type(uint256).max);
    marketDAI.deposit(100_000 ether, BOB);
    vm.stopPrank();

    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketDAI.borrow(70_000 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketDAI, new MockPriceFeed(18, 0.01 ether));
    vm.stopPrank();

    uint256 balanceBefore = usdc.balanceOf(address(liquidator));
    liquidator.liquidate(
      IMarket(address(marketDAI)),
      IMarket(address(marketUSDC)),
      ALICE,
      67_300 ether,
      address(0),
      500,
      0
    );
    assertGt(usdc.balanceOf(address(liquidator)), balanceBefore);
  }

  function testSingleMarketLiquidation() external {
    deal(address(dai), BOB, 100_000 ether);
    vm.startPrank(BOB);
    dai.approve(address(marketDAI), type(uint256).max);
    marketDAI.deposit(100_000 ether, BOB);
    vm.stopPrank();

    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketDAI.borrow(70_000 ether, ALICE, ALICE);
    marketUSDC.borrow(2_000e6, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketDAI, new MockPriceFeed(18, 0.01 ether));
    vm.stopPrank();

    liquidator.liquidate(
      IMarket(address(marketUSDC)),
      IMarket(address(marketUSDC)),
      ALICE,
      2_000e6,
      address(dai),
      500,
      0
    );
    assertGt(usdc.balanceOf(address(liquidator)), 0);
  }

  function testDoubleSwapLiquidation() external {
    deal(address(usdc), BOB, 100_000e6);
    vm.startPrank(BOB);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, BOB);
    vm.stopPrank();

    deal(address(wstETH), ALICE, 100 ether);
    vm.startPrank(ALICE);
    wstETH.approve(address(marketwstETH), type(uint256).max);
    marketwstETH.deposit(100 ether, ALICE);
    auditor.enterMarket(marketwstETH);
    marketUSDC.borrow(60_000e6, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketUSDC, new MockPriceFeed(18, 0.01 ether));
    vm.stopPrank();

    uint256 balancewstETHBefore = wstETH.balanceOf(address(liquidator));
    liquidator.liquidate(
      IMarket(address(marketUSDC)),
      IMarket(address(marketwstETH)),
      ALICE,
      2_000e6,
      address(weth),
      500,
      500
    );
    assertGt(wstETH.balanceOf(address(liquidator)), balancewstETHBefore);
  }

  function testReverseDoubleSwapLiquidation() external {
    deal(address(wstETH), BOB, 100 ether);
    vm.startPrank(BOB);
    wstETH.approve(address(marketwstETH), type(uint256).max);
    marketwstETH.deposit(100 ether, BOB);
    vm.stopPrank();

    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketwstETH.borrow(30 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketwstETH, new MockPriceFeed(18, 2 ether));
    vm.stopPrank();

    uint256 balanceUSDCBefore = usdc.balanceOf(address(liquidator));
    liquidator.liquidate(
      IMarket(address(marketwstETH)),
      IMarket(address(marketUSDC)),
      ALICE,
      20 ether,
      address(weth),
      500,
      500
    );
    assertGt(usdc.balanceOf(address(liquidator)), balanceUSDCBefore);
  }

  function testAnotherDoubleSwapLiquidation() external {
    deal(address(wstETH), BOB, 100 ether);
    vm.startPrank(BOB);
    wstETH.approve(address(marketwstETH), type(uint256).max);
    marketwstETH.deposit(100 ether, BOB);
    vm.stopPrank();

    deal(address(wbtc), ALICE, 100 ether);
    vm.startPrank(ALICE);
    wbtc.approve(address(marketWBTC), type(uint256).max);
    marketWBTC.deposit(1e8, ALICE);
    auditor.enterMarket(marketWBTC);
    marketwstETH.borrow(6 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketWBTC, new MockPriceFeed(18, 1 ether));
    vm.stopPrank();

    uint256 balanceWBTCBefore = wbtc.balanceOf(address(liquidator));
    liquidator.liquidate(
      IMarket(address(marketwstETH)),
      IMarket(address(marketWBTC)),
      ALICE,
      2 ether,
      address(weth),
      500,
      500
    );
    assertGt(wbtc.balanceOf(address(liquidator)), balanceWBTCBefore);
  }

  function testSwap() external {
    deal(address(usdc), address(liquidator), 666_666e6);
    assertEq(dai.balanceOf(address(liquidator)), 0);
    assertEq(usdc.balanceOf(address(liquidator)), 666_666e6);

    liquidator.swap(usdc, 666_666e6, dai, 0, 500);

    assertGt(dai.balanceOf(address(liquidator)), 0);
    assertEq(usdc.balanceOf(address(liquidator)), 0);
  }

  function testTransfer() external {
    deal(address(usdc), address(liquidator), 666_666e6);
    assertEq(usdc.balanceOf(address(this)), 0);
    assertEq(usdc.balanceOf(address(liquidator)), 666_666e6);

    liquidator.transfer(usdc, address(this), 666_666e6);

    assertEq(usdc.balanceOf(address(this)), 666_666e6);
    assertEq(usdc.balanceOf(address(liquidator)), 0);
  }

  function testAuthority() external {
    vm.prank(ALICE);
    vm.expectRevert("UNAUTHORIZED");
    liquidator.addCaller(ALICE);

    vm.prank(ALICE);
    vm.expectRevert("UNAUTHORIZED");
    liquidator.liquidate(IMarket(address(0)), IMarket(address(0)), address(0), 0, address(0), 0, 0);

    liquidator.addCaller(ALICE);

    vm.prank(ALICE);
    vm.expectRevert("UNAUTHORIZED");
    liquidator.addCaller(ALICE);

    vm.prank(ALICE);
    vm.expectRevert("UNAUTHORIZED");
    liquidator.transfer(ERC20(address(0)), address(0), 0);

    vm.prank(ALICE);
    vm.expectRevert();
    liquidator.liquidate(IMarket(address(0)), IMarket(address(0)), address(0), 0, address(0), 0, 0);
  }

  function getAddress(string memory name, string memory base) internal returns (address addr) {
    if (block.chainid == 31337) return address(0);

    string memory network;
    if (block.chainid == 1) network = "mainnet";
    else if (block.chainid == 5) network = "goerli";

    addr = vm.readFile(string.concat(base, "deployments/", network, "/", name, ".json")).readAddress(".address");
    vm.label(addr, name);
  }
}
