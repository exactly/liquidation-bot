// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.15;

import { ERC20 } from "solmate/src/tokens/ERC20.sol";
import { Owned } from "solmate/src/auth/Owned.sol";
import { SafeTransferLib } from "solmate/src/utils/SafeTransferLib.sol";
import { IUniswapV3FlashCallback } from "@uniswap/v3-core/contracts/interfaces/callback/IUniswapV3FlashCallback.sol";
import { IUniswapV3Pool } from "@uniswap/v3-core/contracts/interfaces/IUniswapV3Pool.sol";
import { ISwapRouter } from "@uniswap/v3-periphery/contracts/interfaces/ISwapRouter.sol";

contract Liquidator is Owned, IUniswapV3FlashCallback {
  using SafeTransferLib for ERC20;

  address public immutable factory;
  ISwapRouter public immutable swapRouter;

  constructor(address factory_, ISwapRouter swapRouter_) Owned(msg.sender) {
    factory = factory_;
    swapRouter = swapRouter_;
  }

  function liquidate(
    IMarket repayMarket,
    IMarket seizeMarket,
    address borrower,
    uint256 maxRepay,
    address flashPair,
    uint24 flashFee,
    uint24 swapFee
  ) external onlyOwner {
    ERC20 repayAsset = repayMarket.asset();
    uint256 availableRepay = repayAsset.balanceOf(address(this));

    if (availableRepay >= maxRepay) {
      repayAsset.safeApprove(address(repayMarket), maxRepay);
      repayMarket.liquidate(borrower, maxRepay, seizeMarket);
    } else {
      uint256 flashBorrow = maxRepay - availableRepay;
      PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), flashPair, flashFee);
      bytes memory data = abi.encode(
        FlashCallbackData({
          repayMarket: repayMarket,
          seizeMarket: seizeMarket,
          borrower: borrower,
          maxRepay: maxRepay,
          flashBorrow: flashBorrow,
          flashPair: flashPair,
          flashFee: flashFee,
          swapFee: swapFee
        })
      );
      IUniswapV3Pool(PoolAddress.computeAddress(factory, poolKey)).flash(
        address(this),
        address(repayAsset) == poolKey.token0 ? flashBorrow : 0,
        address(repayAsset) == poolKey.token1 ? flashBorrow : 0,
        data
      );
    }
  }

  function uniswapV3FlashCallback(
    uint256 fee0,
    uint256 fee1,
    bytes calldata data
  ) external {
    FlashCallbackData memory f = abi.decode(data, (FlashCallbackData));
    ERC20 repayAsset = f.repayMarket.asset();
    PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), f.flashPair, f.flashFee);

    require(msg.sender == PoolAddress.computeAddress(factory, poolKey));

    repayAsset.safeApprove(address(f.repayMarket), f.maxRepay);
    f.repayMarket.liquidate(f.borrower, f.maxRepay, f.seizeMarket);

    uint256 flashRepay = f.flashBorrow + (address(repayAsset) == poolKey.token0 ? fee0 : fee1);

    ERC20 seizeAsset = f.seizeMarket.asset();
    ISwapRouter memSwapRouter = swapRouter;
    seizeAsset.safeApprove(address(memSwapRouter), type(uint256).max);
    memSwapRouter.exactOutputSingle(
      ISwapRouter.ExactOutputSingleParams({
        tokenIn: address(seizeAsset),
        tokenOut: address(repayAsset),
        fee: f.swapFee,
        recipient: address(this),
        deadline: block.timestamp,
        amountOut: flashRepay,
        amountInMaximum: type(uint256).max,
        sqrtPriceLimitX96: 0
      })
    );

    repayAsset.safeTransfer(msg.sender, flashRepay);
  }

  function drain(ERC20 asset) external onlyOwner {
    asset.safeTransfer(owner, asset.balanceOf(address(this)));
  }
}

struct FlashCallbackData {
  IMarket repayMarket;
  IMarket seizeMarket;
  address borrower;
  uint256 maxRepay;
  uint256 flashBorrow;
  address flashPair;
  uint24 flashFee;
  uint24 swapFee;
}

interface IMarket {
  function asset() external view returns (ERC20);

  function liquidate(
    address borrower,
    uint256 maxAssets,
    IMarket seizeMarket
  ) external returns (uint256 repaidAssets);
}

// https://github.com/Uniswap/v3-periphery/pull/289
library PoolAddress {
  bytes32 internal constant POOL_INIT_CODE_HASH = 0xe34f199b19b2b4f47f68442619d555527d244f78a3297ea89325f843f87b8b54;

  struct PoolKey {
    address token0;
    address token1;
    uint24 fee;
  }

  function getPoolKey(
    address tokenA,
    address tokenB,
    uint24 fee
  ) internal pure returns (PoolKey memory) {
    if (tokenA > tokenB) (tokenA, tokenB) = (tokenB, tokenA);
    return PoolKey({ token0: tokenA, token1: tokenB, fee: fee });
  }

  function computeAddress(address factory, PoolKey memory key) internal pure returns (address pool) {
    require(key.token0 < key.token1);
    pool = address(
      uint160(
        uint256(
          keccak256(
            abi.encodePacked(
              hex"ff",
              factory,
              keccak256(abi.encode(key.token0, key.token1, key.fee)),
              POOL_INIT_CODE_HASH
            )
          )
        )
      )
    );
  }
}
