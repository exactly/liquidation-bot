// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.16;

import { ERC20 } from "solmate/src/tokens/ERC20.sol";
import { Owned } from "solmate/src/auth/Owned.sol";
import { SafeTransferLib } from "solmate/src/utils/SafeTransferLib.sol";
import { IUniswapV3FlashCallback } from "@uniswap/v3-core/contracts/interfaces/callback/IUniswapV3FlashCallback.sol";
import { IUniswapV3SwapCallback } from "@uniswap/v3-core/contracts/interfaces/callback/IUniswapV3SwapCallback.sol";
import { IUniswapV3Pool } from "@uniswap/v3-core/contracts/interfaces/IUniswapV3Pool.sol";

contract Liquidator is Owned, IUniswapV3FlashCallback, IUniswapV3SwapCallback {
  using SafeTransferLib for ERC20;

  /// @dev The minimum value that can be returned from #getSqrtRatioAtTick. Equivalent to getSqrtRatioAtTick(MIN_TICK)
  uint160 internal constant MIN_SQRT_RATIO = 4295128739;
  /// @dev The maximum value that can be returned from #getSqrtRatioAtTick. Equivalent to getSqrtRatioAtTick(MAX_TICK)
  uint160 internal constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

  address public immutable factory;

  constructor(address factory_) Owned(msg.sender) {
    factory = factory_;
  }

  function liquidate(
    IMarket repayMarket,
    IMarket seizeMarket,
    address borrower,
    uint256 maxRepay,
    address poolPair,
    uint24 fee
  ) external onlyOwner {
    ERC20 repayAsset = repayMarket.asset();
    uint256 availableRepay = repayAsset.balanceOf(address(this));

    if (availableRepay >= maxRepay) {
      repayAsset.safeApprove(address(repayMarket), maxRepay);
      repayMarket.liquidate(borrower, maxRepay, seizeMarket);
    } else {
      uint256 flashBorrow = maxRepay - availableRepay;
      PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), poolPair, fee);
      if (repayMarket != seizeMarket) {
        bytes memory data = abi.encode(
          SwapCallbackData({ repayMarket: repayMarket, seizeMarket: seizeMarket, borrower: borrower, fee: fee })
        );
        IUniswapV3Pool(PoolAddress.computeAddress(factory, poolKey)).swap(
          address(this),
          address(repayAsset) == poolKey.token1,
          -int256(maxRepay),
          address(repayAsset) == poolKey.token1 ? MIN_SQRT_RATIO + 1 : MAX_SQRT_RATIO - 1,
          data
        );
      } else {
        bytes memory data = abi.encode(
          FlashCallbackData({
            repayMarket: repayMarket,
            seizeMarket: seizeMarket,
            borrower: borrower,
            maxRepay: maxRepay,
            flashBorrow: flashBorrow,
            poolPair: poolPair,
            fee: fee
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
  }

  function uniswapV3SwapCallback(
    int256 amount0Delta,
    int256 amount1Delta,
    bytes calldata data
  ) external {
    SwapCallbackData memory s = abi.decode(data, (SwapCallbackData));
    ERC20 repayAsset = s.repayMarket.asset();
    ERC20 seizeAsset = s.seizeMarket.asset();
    PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), address(seizeAsset), s.fee);

    require(msg.sender == PoolAddress.computeAddress(factory, poolKey));

    uint256 maxRepay = uint256(-(address(repayAsset) == poolKey.token0 ? amount0Delta : amount1Delta));
    repayAsset.safeApprove(address(s.repayMarket), maxRepay);
    s.repayMarket.liquidate(s.borrower, maxRepay, s.seizeMarket);

    seizeAsset.safeTransfer(msg.sender, uint256(address(seizeAsset) == poolKey.token0 ? amount0Delta : amount1Delta));
  }

  function uniswapV3FlashCallback(
    uint256 fee0,
    uint256 fee1,
    bytes calldata data
  ) external {
    FlashCallbackData memory f = abi.decode(data, (FlashCallbackData));
    ERC20 repayAsset = f.repayMarket.asset();
    PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), f.poolPair, f.fee);

    require(msg.sender == PoolAddress.computeAddress(factory, poolKey));

    repayAsset.safeApprove(address(f.repayMarket), f.maxRepay);
    f.repayMarket.liquidate(f.borrower, f.maxRepay, f.seizeMarket);

    repayAsset.safeTransfer(msg.sender, f.flashBorrow + (address(repayAsset) == poolKey.token0 ? fee0 : fee1));
  }

  function transfer(ERC20 asset, address to, uint256 amount) external onlyOwner {
    asset.safeTransfer(to, amount);
  }
}

struct SwapCallbackData {
  IMarket repayMarket;
  IMarket seizeMarket;
  address borrower;
  uint24 fee;
}

struct FlashCallbackData {
  IMarket repayMarket;
  IMarket seizeMarket;
  address borrower;
  uint256 maxRepay;
  uint256 flashBorrow;
  address poolPair;
  uint24 fee;
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
