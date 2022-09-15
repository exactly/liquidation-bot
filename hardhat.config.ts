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
    owner: { default: 0, rinkeby: "0x755DF607BA55ff6430FEE0126A52Bf82D1e57F5f" },
  },
} as HardhatUserConfig;
