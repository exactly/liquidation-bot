// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.17;

import { ERC20 } from "solmate/src/tokens/ERC20.sol";
import { Auth, Authority } from "solmate/src/auth/Auth.sol";
import { SafeTransferLib } from "solmate/src/utils/SafeTransferLib.sol";
import { IUniswapV3FlashCallback } from "@uniswap/v3-core/contracts/interfaces/callback/IUniswapV3FlashCallback.sol";
import { IUniswapV3SwapCallback } from "@uniswap/v3-core/contracts/interfaces/callback/IUniswapV3SwapCallback.sol";
import { IUniswapV3Pool } from "@uniswap/v3-core/contracts/interfaces/IUniswapV3Pool.sol";
import { ISwapRouter } from "@uniswap/v3-periphery/contracts/interfaces/ISwapRouter.sol";

contract Liquidator is Auth, Authority, IUniswapV3FlashCallback, IUniswapV3SwapCallback {
  using SafeTransferLib for ERC20;

  /// @dev The minimum value that can be returned from #getSqrtRatioAtTick. Equivalent to getSqrtRatioAtTick(MIN_TICK)
  uint160 internal constant MIN_SQRT_RATIO = 4295128739;
  /// @dev The maximum value that can be returned from #getSqrtRatioAtTick. Equivalent to getSqrtRatioAtTick(MAX_TICK)
  uint160 internal constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

  ISwapRouter public immutable swapRouter;
  address public immutable uniswapFactory;
  IVelodromeFactory public immutable velodromeFactory;

  mapping(address => bool) public callers;

  constructor(
    address owner_,
    ISwapRouter swapRouter_,
    address uniswapFactory_,
    IVelodromeFactory velodromeFactory_
  ) Auth(owner_, this) {
    swapRouter = swapRouter_;
    uniswapFactory = uniswapFactory_;
    velodromeFactory = velodromeFactory_;
  }

  function liquidateUniswap(
    IMarket repayMarket,
    IMarket seizeMarket,
    address borrower,
    uint256 maxRepay,
    address poolPair,
    uint24 fee,
    uint24 pairFee
  ) external requiresAuth {
    ERC20 repayAsset = repayMarket.asset();
    uint256 availableRepay = repayAsset.balanceOf(address(this));

    if (availableRepay >= maxRepay) {
      repayAsset.safeApprove(address(repayMarket), maxRepay);
      repayMarket.liquidate(borrower, maxRepay, seizeMarket);
    } else {
      uint256 flashBorrow = maxRepay - availableRepay;
      if (repayMarket != seizeMarket) {
        PoolAddress.PoolKey memory poolKey;
        bytes memory data;
        if (poolPair == address(0)) {
          ERC20 seizeAsset = seizeMarket.asset();
          poolKey = PoolAddress.getPoolKey(address(repayAsset), address(seizeAsset), fee);
          data = abi.encode(
            SwapCallbackData({
              repayMarket: repayMarket,
              seizeMarket: seizeMarket,
              borrower: borrower,
              poolPair: address(seizeAsset),
              fee: fee,
              pairFee: 0
            })
          );
        } else {
          poolKey = PoolAddress.getPoolKey(address(repayAsset), poolPair, fee);
          data = abi.encode(
            SwapCallbackData({
              repayMarket: repayMarket,
              seizeMarket: seizeMarket,
              borrower: borrower,
              poolPair: poolPair,
              fee: fee,
              pairFee: pairFee
            })
          );
        }
        IUniswapV3Pool(PoolAddress.computeAddress(uniswapFactory, poolKey)).swap(
          address(this),
          address(repayAsset) == poolKey.token1,
          -int256(maxRepay),
          address(repayAsset) == poolKey.token1 ? MIN_SQRT_RATIO + 1 : MAX_SQRT_RATIO - 1,
          data
        );
      } else {
        PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), poolPair, fee);
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
        IUniswapV3Pool(PoolAddress.computeAddress(uniswapFactory, poolKey)).flash(
          address(this),
          address(repayAsset) == poolKey.token0 ? flashBorrow : 0,
          address(repayAsset) == poolKey.token1 ? flashBorrow : 0,
          data
        );
      }
    }
  }

  function liquidateVelodrome(
    IMarket repayMarket,
    IMarket seizeMarket,
    address borrower,
    uint256 maxRepay,
    ERC20 poolPair,
    bool isStable,
    bool isPairStable
  ) external requiresAuth {
    LiquidateVars memory v;
    v.repayAsset = repayMarket.asset();

    if (v.repayAsset.balanceOf(address(this)) >= maxRepay) {
      v.repayAsset.safeApprove(address(repayMarket), maxRepay);
      repayMarket.liquidate(borrower, maxRepay, seizeMarket);
    } else {
      if (repayMarket != seizeMarket) {
        if (address(poolPair) == address(0)) {
          v.seizeAsset = seizeMarket.asset();
          v.pool = velodromeFactory.getPool(v.repayAsset, v.seizeAsset, isStable);
          IVelodromePool(v.pool).swap(
            address(v.seizeAsset) > address(v.repayAsset) ? maxRepay : 0,
            address(v.seizeAsset) < address(v.repayAsset) ? maxRepay : 0,
            address(this),
            abi.encode(
              VelodromeCallbackData({
                repayMarket: repayMarket,
                seizeMarket: seizeMarket,
                borrower: borrower,
                poolPair: address(v.seizeAsset),
                fee: velodromeFactory.getFee(v.pool, isStable),
                pairFee: 0,
                isStable: isStable,
                isPairStable: false
              })
            )
          );
        } else {
          v.pool = velodromeFactory.getPool(v.repayAsset, poolPair, isStable);
          IVelodromePool(v.pool).swap(
            address(poolPair) > address(v.repayAsset) ? maxRepay : 0,
            address(poolPair) < address(v.repayAsset) ? maxRepay : 0,
            address(this),
            abi.encode(
              VelodromeCallbackData({
                repayMarket: repayMarket,
                seizeMarket: seizeMarket,
                borrower: borrower,
                poolPair: address(poolPair),
                fee: 0,
                pairFee: velodromeFactory.getFee(v.pool, isStable),
                isStable: isStable,
                isPairStable: isPairStable
              })
            )
          );
        }
      }
    }
  }

  function hook(address sender, uint256 amount0Out, uint256 amount1Out, bytes calldata data) external {
    require(sender == address(this));

    VelodromeCallbackData memory v = abi.decode(data, (VelodromeCallbackData));
    ERC20 seizeAsset = v.seizeMarket.asset();

    if (v.borrower != address(0)) {
      ERC20 repayAsset = v.repayMarket.asset();
      require(msg.sender == velodromeFactory.getPool(repayAsset, ERC20(v.poolPair), v.isStable));

      uint256 maxRepay = amount0Out == 0 ? amount1Out : amount0Out;
      repayAsset.safeApprove(address(v.repayMarket), maxRepay);
      v.repayMarket.liquidate(v.borrower, maxRepay, v.seizeMarket);

      if (v.pairFee > 0) {
        address pool = velodromeFactory.getPool(seizeAsset, ERC20(v.poolPair), v.isPairStable);
        uint256 amount = getAmountIn(msg.sender, maxRepay, amount0Out == 0, v.pairFee);

        IVelodromePool(pool).swap(
          address(seizeAsset) > v.poolPair ? amount : 0,
          address(seizeAsset) < v.poolPair ? amount : 0,
          address(this),
          abi.encode(
            VelodromeCallbackData({
              repayMarket: IMarket(address(0)),
              seizeMarket: v.seizeMarket,
              borrower: address(0),
              poolPair: v.poolPair,
              fee: velodromeFactory.getFee(pool, v.isPairStable),
              pairFee: 0,
              isStable: false,
              isPairStable: v.isPairStable
            })
          )
        );

        ERC20(v.poolPair).safeTransfer(msg.sender, amount);
      } else {
        seizeAsset.transfer(msg.sender, getAmountIn(msg.sender, maxRepay, amount0Out == 0, v.fee));
      }
    } else {
      require(msg.sender == velodromeFactory.getPool(ERC20(v.poolPair), seizeAsset, v.isPairStable));

      seizeAsset.safeTransfer(
        msg.sender,
        getAmountIn(msg.sender, amount0Out == 0 ? amount1Out : amount0Out, amount0Out == 0, v.fee)
      );
    }
  }

  // slither-disable-next-line similar-names
  function uniswapV3SwapCallback(int256 amount0Delta, int256 amount1Delta, bytes calldata data) external {
    SwapCallbackData memory s = abi.decode(data, (SwapCallbackData));
    ERC20 seizeAsset = s.seizeMarket.asset();
    if (s.borrower != address(0)) {
      ERC20 repayAsset = s.repayMarket.asset();
      PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), s.poolPair, s.fee);
      require(msg.sender == PoolAddress.computeAddress(uniswapFactory, poolKey));

      uint256 maxRepay = uint256(-(address(repayAsset) == poolKey.token0 ? amount0Delta : amount1Delta));
      repayAsset.safeApprove(address(s.repayMarket), maxRepay);
      s.repayMarket.liquidate(s.borrower, maxRepay, s.seizeMarket);
      if (s.pairFee > 0) {
        PoolAddress.PoolKey memory swapPoolKey = PoolAddress.getPoolKey(address(seizeAsset), s.poolPair, s.pairFee);
        IUniswapV3Pool(PoolAddress.computeAddress(uniswapFactory, swapPoolKey)).swap(
          address(this),
          address(seizeAsset) == swapPoolKey.token0,
          -int256(s.poolPair == poolKey.token0 ? amount0Delta : amount1Delta),
          address(seizeAsset) == swapPoolKey.token0 ? MIN_SQRT_RATIO + 1 : MAX_SQRT_RATIO - 1,
          abi.encode(
            SwapCallbackData({
              repayMarket: IMarket(address(0)),
              seizeMarket: s.seizeMarket,
              borrower: address(0),
              poolPair: s.poolPair,
              fee: 0,
              pairFee: s.pairFee
            })
          )
        );

        ERC20(s.poolPair).safeTransfer(msg.sender, uint256(s.poolPair == poolKey.token0 ? amount0Delta : amount1Delta));
      } else {
        seizeAsset.safeTransfer(
          msg.sender,
          uint256(address(seizeAsset) == poolKey.token0 ? amount0Delta : amount1Delta)
        );
      }
    } else {
      PoolAddress.PoolKey memory swapPoolKey = PoolAddress.getPoolKey(address(seizeAsset), s.poolPair, s.pairFee);
      require(msg.sender == PoolAddress.computeAddress(uniswapFactory, swapPoolKey));

      seizeAsset.safeTransfer(
        msg.sender,
        uint256(address(seizeAsset) == swapPoolKey.token0 ? amount0Delta : amount1Delta)
      );
    }
  }

  function uniswapV3FlashCallback(uint256 fee0, uint256 fee1, bytes calldata data) external {
    FlashCallbackData memory f = abi.decode(data, (FlashCallbackData));
    ERC20 repayAsset = f.repayMarket.asset();
    PoolAddress.PoolKey memory poolKey = PoolAddress.getPoolKey(address(repayAsset), f.poolPair, f.fee);

    require(msg.sender == PoolAddress.computeAddress(uniswapFactory, poolKey));

    repayAsset.safeApprove(address(f.repayMarket), f.maxRepay);
    f.repayMarket.liquidate(f.borrower, f.maxRepay, f.seizeMarket);

    repayAsset.safeTransfer(msg.sender, f.flashBorrow + (address(repayAsset) == poolKey.token0 ? fee0 : fee1));
  }

  function getAmountIn(address pool, uint256 amountOut, bool isToken0, uint256 fee) internal view returns (uint256) {
    (uint256 reserve0, uint256 reserve1, ) = IVelodromePool(pool).getReserves();
    return
      (
        isToken0
          ? (reserve0 * amountOut * 10_000) / ((reserve1 - amountOut) * (10_000 - fee))
          : (reserve1 * amountOut * 10_000) / ((reserve0 - amountOut) * (10_000 - fee))
      ) + 1;
  }

  function swap(
    ERC20 assetIn,
    uint256 amountIn,
    ERC20 assetOut,
    uint256 amountOutMinimum,
    uint24 fee
  ) external requiresAuth {
    assetIn.safeApprove(address(swapRouter), amountIn);
    swapRouter.exactInputSingle(
      ISwapRouter.ExactInputSingleParams({
        tokenIn: address(assetIn),
        tokenOut: address(assetOut),
        fee: fee,
        recipient: address(this),
        deadline: block.timestamp,
        amountIn: amountIn,
        amountOutMinimum: amountOutMinimum,
        sqrtPriceLimitX96: 0
      })
    );
  }

  function transfer(ERC20 asset, address to, uint256 amount) external requiresAuth {
    asset.safeTransfer(to, amount);
  }

  function canCall(address caller, address, bytes4 functionSig) external view returns (bool) {
    return
      (functionSig == this.liquidateUniswap.selector || functionSig == this.liquidateVelodrome.selector) &&
      callers[caller];
  }

  function addCaller(address caller) external requiresAuth {
    callers[caller] = true;
  }

  function removeCaller(address caller) external requiresAuth {
    delete callers[caller];
  }
}

struct LiquidateVars {
  ERC20 repayAsset;
  ERC20 seizeAsset;
  address pool;
}

struct VelodromeCallbackData {
  IMarket repayMarket;
  IMarket seizeMarket;
  address borrower;
  address poolPair;
  uint24 fee;
  uint24 pairFee;
  bool isStable;
  bool isPairStable;
}

struct SwapCallbackData {
  IMarket repayMarket;
  IMarket seizeMarket;
  address borrower;
  address poolPair;
  uint24 fee;
  uint24 pairFee;
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

  function liquidate(address borrower, uint256 maxAssets, IMarket seizeMarket) external returns (uint256 repaidAssets);
}

interface IVelodromeFactory {
  function getPool(ERC20 tokenA, ERC20 tokenB, bool stable) external view returns (address);

  function getFee(address pool, bool stable) external view returns (uint24);
}

interface IVelodromePool {
  function swap(uint256 amount0Out, uint256 amount1Out, address to, bytes calldata data) external;

  function getReserves() external view returns (uint256 reserve0, uint256 reserve1, uint256 blockTimestampLast);
}

// https://github.com/Uniswap/v3-periphery/pull/289
library PoolAddress {
  bytes32 internal constant POOL_INIT_CODE_HASH = 0xe34f199b19b2b4f47f68442619d555527d244f78a3297ea89325f843f87b8b54;

  struct PoolKey {
    address token0;
    address token1;
    uint24 fee;
  }

  function getPoolKey(address tokenA, address tokenB, uint24 fee) internal pure returns (PoolKey memory) {
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
