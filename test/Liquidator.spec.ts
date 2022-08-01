import { expect } from "chai";
import { readFile } from "fs/promises";
import { ethers, deployments, network } from "hardhat";
import type { HardhatNetworkConfig } from "hardhat/types";
import type { SignerWithAddress } from "@nomiclabs/hardhat-ethers/signers";
import type { Liquidator } from "../types";

const { getNamedSigner, getContract } = ethers;

describe("Liquidator", function () {
  let borrower: SignerWithAddress;
  let usdc: string;
  let marketDAI: string;
  let marketWBTC: string;
  let liquidator: Liquidator;

  before(async () => {
    borrower = await getNamedSigner("borrower");
    [usdc, marketDAI, marketWBTC] = (
      await Promise.all(
        ["USDC", "MarketDAI", "MarketWBTC"].map((name) =>
          readFile(
            `node_modules/@exactly-protocol/protocol/deployments/${
              (network.config as HardhatNetworkConfig).forking?.network ?? network.name
            }/${name}.json`,
          ),
        ),
      )
    ).map((json) => JSON.parse(json.toString()).address);
  });

  beforeEach(async () => {
    await deployments.fixture("Liquidator", { keepExistingDeployments: true });
    liquidator = await getContract<Liquidator>("Liquidator");
  });

  describe("liquidate", () => {
    it("should liquidate", async () => {
      await expect(await liquidator.liquidate(marketDAI, marketWBTC, borrower.address, 1_000, usdc, 500, 500)).to.not.be
        .reverted;
    });
  });
});
