import "dotenv/config";
import "hardhat-deploy";
import "@typechain/hardhat";
import "@nomiclabs/hardhat-ethers";
import { env } from "process";
import type { HardhatUserConfig } from "hardhat/types";

export default {
  solidity: { version: "0.8.16", settings: { optimizer: { enabled: true, runs: 6_666_666 } } },
  networks: {
    rinkeby: {
      url: env.RINKEBY_NODE ?? "https://rinkeby.infura.io/",
      ...(env.MNEMONIC && { accounts: { mnemonic: env.MNEMONIC } }),
    },
  },
  typechain: { outDir: "types" },
  namedAccounts: {
    deployer: { default: 0 },
  },
} as HardhatUserConfig;
