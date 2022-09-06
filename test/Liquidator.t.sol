// SPDX-License-Identifier: AGPL-3.0-or-later
pragma solidity 0.8.16;

import { Test, stdJson } from "forge-std/Test.sol";
import { ISwapRouter } from "@uniswap/v3-periphery/contracts/interfaces/ISwapRouter.sol";
import { Liquidator } from "../contracts/Liquidator.sol";

contract LiquidatorTest is Test {
  using stdJson for string;

  Liquidator internal liquidator;

  function setUp() public {
    string memory network;
    if (block.chainid == 4) network = "rinkeby";

    liquidator = new Liquidator(
      vm.readFile(string.concat("deployments/", network, "/UniswapV3Factory.json")).readAddress(".address"),
      ISwapRouter(vm.readFile(string.concat("deployments/", network, "/UniswapV3Router.json")).readAddress(".address"))
    );
  }
}
