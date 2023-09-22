// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.17;

import { Test, stdJson } from "forge-std/Test.sol";
import { Market } from "@exactly/protocol/contracts/Market.sol";
import { Auditor } from "@exactly/protocol/contracts/Auditor.sol";
import { MockPriceFeed } from "@exactly/protocol/contracts/mocks/MockPriceFeed.sol";
import { Liquidator, IMarket, IVelodromeFactory, ERC20, ISwapRouter, PoolAddress } from "../contracts/Liquidator.sol";

contract LiquidatorTest is Test {
  using stdJson for string;

  address internal constant ALICE = address(0x420);
  address internal constant BOB = address(0x069);

  Liquidator internal liquidator;
  Market internal marketWETH;
  Market internal marketUSDC;
  Market internal marketOP;
  Market internal marketwstETH;
  Auditor internal auditor;
  address internal timelock;
  ERC20 internal usdc;
  ERC20 internal dai;
  ERC20 internal weth;
  ERC20 internal wstETH;
  ERC20 internal op;

  function setUp() public {
    vm.createSelectFork(vm.envString("OPTIMISM_NODE"), 109_683_846);

    liquidator = new Liquidator(
      address(this),
      ISwapRouter(getAddress("UniswapV3Router", "")),
      getAddress("UniswapV3Factory", ""),
      IVelodromeFactory(0xF1046053aa5682b4F9a81b5481394DA16BE5FF5a)
    );

    usdc = ERC20(getAddress("USDC", "node_modules/@exactly/protocol/"));
    weth = ERC20(getAddress("WETH", "node_modules/@exactly/protocol/"));
    op = ERC20(getAddress("OP", "node_modules/@exactly/protocol/"));
    wstETH = ERC20(getAddress("wstETH", "node_modules/@exactly/protocol/"));
    auditor = Auditor(getAddress("Auditor", "node_modules/@exactly/protocol/"));
    timelock = getAddress("TimelockController", "node_modules/@exactly/protocol/");
    marketWETH = Market(getAddress("MarketWETH", "node_modules/@exactly/protocol/"));
    marketUSDC = Market(getAddress("MarketUSDC", "node_modules/@exactly/protocol/"));
    marketOP = Market(getAddress("MarketOP", "node_modules/@exactly/protocol/"));
    marketwstETH = Market(getAddress("MarketwstETH", "node_modules/@exactly/protocol/"));

    vm.label(ALICE, "alice");
    if (address(dai) != address(0)) {
      vm.label(
        PoolAddress.computeAddress(
          liquidator.uniswapFactory(),
          PoolAddress.getPoolKey(address(dai), address(usdc), 500)
        ),
        "DAI/USDC/500"
      );
    }
  }

  function testMultiMarketLiquidation() external {
    deal(address(weth), BOB, 61.34 ether);
    vm.startPrank(BOB);
    weth.approve(address(marketWETH), type(uint256).max);
    marketWETH.deposit(61.34 ether, BOB);
    vm.stopPrank();

    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketWETH.borrow(42.9 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketWETH, new MockPriceFeed(8, 5_000e8));
    vm.stopPrank();

    uint256 balanceBefore = usdc.balanceOf(address(liquidator));
    liquidator.liquidateUniswap(
      IMarket(address(marketWETH)),
      IMarket(address(marketUSDC)),
      ALICE,
      40 ether,
      address(0),
      500,
      0
    );
    assertGt(usdc.balanceOf(address(liquidator)), balanceBefore);
  }

  function testVelodromeMultiMarketLiquidation() external {
    deal(address(weth), BOB, 50 ether);
    vm.startPrank(BOB);
    weth.approve(address(marketWETH), type(uint256).max);
    marketWETH.deposit(50 ether, BOB);
    vm.stopPrank();

    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketWETH.borrow(20 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketWETH, new MockPriceFeed(8, 10 ether));
    vm.stopPrank();

    uint256 balanceBefore = usdc.balanceOf(address(liquidator));
    liquidator.liquidateVelodrome(
      IMarket(address(marketWETH)),
      IMarket(address(marketUSDC)),
      ALICE,
      10 ether,
      ERC20(address(0))
    );
    assertGt(usdc.balanceOf(address(liquidator)), balanceBefore);
  }

  function testVelodromeReverseMultiMarketLiquidation() external {
    deal(address(usdc), BOB, 100_000e6);
    vm.startPrank(BOB);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, BOB);
    vm.stopPrank();

    deal(address(weth), ALICE, 50 ether);
    vm.startPrank(ALICE);
    weth.approve(address(marketWETH), type(uint256).max);
    marketWETH.deposit(50 ether, ALICE);
    auditor.enterMarket(marketWETH);
    marketUSDC.borrow(60_000e6, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketUSDC, new MockPriceFeed(8, 3e8));
    vm.stopPrank();

    uint256 balanceBefore = weth.balanceOf(address(liquidator));
    liquidator.liquidateVelodrome(
      IMarket(address(marketUSDC)),
      IMarket(address(marketWETH)),
      ALICE,
      30_000e6,
      ERC20(address(0))
    );
    assertGt(weth.balanceOf(address(liquidator)), balanceBefore);
  }

  function testVelodromeDoubleSwapLiquidation() external {
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
    auditor.setPriceFeed(marketUSDC, new MockPriceFeed(8, 5e8));
    vm.stopPrank();

    uint256 balancewstETHBefore = wstETH.balanceOf(address(liquidator));
    liquidator.liquidateVelodrome(IMarket(address(marketUSDC)), IMarket(address(marketwstETH)), ALICE, 30_000e6, weth);
    assertGt(wstETH.balanceOf(address(liquidator)), balancewstETHBefore);
  }

  function testVelodromeReverseDoubleSwapLiquidation() external {
    deal(address(wstETH), BOB, 50 ether);
    vm.startPrank(BOB);
    wstETH.approve(address(marketwstETH), type(uint256).max);
    marketwstETH.deposit(50 ether, BOB);
    vm.stopPrank();

    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketwstETH.borrow(30 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketUSDC, new MockPriceFeed(8, 0.2e8));
    vm.stopPrank();

    uint256 balanceBefore = usdc.balanceOf(address(liquidator));
    liquidator.liquidateVelodrome(IMarket(address(marketwstETH)), IMarket(address(marketUSDC)), ALICE, 20 ether, weth);
    assertGt(usdc.balanceOf(address(liquidator)), balanceBefore);
  }

  function testSingleMarketLiquidation() external {
    deal(address(weth), BOB, 61.34 ether);
    vm.startPrank(BOB);
    weth.approve(address(marketWETH), type(uint256).max);
    marketWETH.deposit(61.34 ether, BOB);
    vm.stopPrank();

    deal(address(usdc), ALICE, 100_000e6);
    vm.startPrank(ALICE);
    usdc.approve(address(marketUSDC), type(uint256).max);
    marketUSDC.deposit(100_000e6, ALICE);
    auditor.enterMarket(marketUSDC);
    marketWETH.borrow(40 ether, ALICE, ALICE);
    marketUSDC.borrow(2_000e6, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketWETH, new MockPriceFeed(8, 7_000e8));
    vm.stopPrank();

    liquidator.liquidateUniswap(
      IMarket(address(marketUSDC)),
      IMarket(address(marketUSDC)),
      ALICE,
      2_000e6,
      address(weth),
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
    auditor.setPriceFeed(marketUSDC, new MockPriceFeed(8, 5e8));
    vm.stopPrank();

    uint256 balancewstETHBefore = wstETH.balanceOf(address(liquidator));
    liquidator.liquidateUniswap(
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
    auditor.setPriceFeed(marketwstETH, new MockPriceFeed(8, 3_200e8));
    vm.stopPrank();

    uint256 balanceUSDCBefore = usdc.balanceOf(address(liquidator));
    liquidator.liquidateUniswap(
      IMarket(address(marketwstETH)),
      IMarket(address(marketUSDC)),
      ALICE,
      20 ether,
      address(weth),
      100,
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

    deal(address(op), ALICE, 25_000 ether);
    vm.startPrank(ALICE);
    op.approve(address(marketOP), type(uint256).max);
    marketOP.deposit(25_000 ether, ALICE);
    auditor.enterMarket(marketOP);
    marketwstETH.borrow(6 ether, ALICE, ALICE);
    vm.stopPrank();

    vm.startPrank(timelock);
    auditor.setPriceFeed(marketOP, new MockPriceFeed(8, 0.1e8));
    vm.stopPrank();

    uint256 balanceOPBefore = op.balanceOf(address(liquidator));
    liquidator.liquidateUniswap(
      IMarket(address(marketwstETH)),
      IMarket(address(marketOP)),
      ALICE,
      2 ether,
      address(weth),
      100,
      3000
    );
    assertGt(op.balanceOf(address(liquidator)), balanceOPBefore);
  }

  function testSwap() external {
    deal(address(usdc), address(liquidator), 666_666e6);
    assertEq(weth.balanceOf(address(liquidator)), 0);
    assertEq(usdc.balanceOf(address(liquidator)), 666_666e6);

    liquidator.swap(usdc, 666_666e6, weth, 0, 500);

    assertGt(weth.balanceOf(address(liquidator)), 0);
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
    liquidator.liquidateUniswap(IMarket(address(0)), IMarket(address(0)), address(0), 0, address(0), 0, 0);

    liquidator.addCaller(ALICE);

    vm.prank(ALICE);
    vm.expectRevert("UNAUTHORIZED");
    liquidator.addCaller(ALICE);

    vm.prank(ALICE);
    vm.expectRevert("UNAUTHORIZED");
    liquidator.transfer(ERC20(address(0)), address(0), 0);

    vm.prank(ALICE);
    vm.expectRevert();
    liquidator.liquidateUniswap(IMarket(address(0)), IMarket(address(0)), address(0), 0, address(0), 0, 0);
  }

  function getAddress(string memory name, string memory base) internal returns (address addr) {
    if (block.chainid == 31337) return address(0);

    string memory network;
    if (block.chainid == 1) network = "mainnet";
    else if (block.chainid == 5) network = "goerli";
    else if (block.chainid == 10) network = "optimism";

    addr = vm.readFile(string.concat(base, "deployments/", network, "/", name, ".json")).readAddress(".address");
    vm.label(addr, name);
  }
}
