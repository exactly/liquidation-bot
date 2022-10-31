import "dotenv/config";
import "hardhat-deploy";
import "@typechain/hardhat";
import "@nomiclabs/hardhat-ethers";
import { env } from "process";
import type { HardhatUserConfig } from "hardhat/types";

export default {
  solidity: { version: "0.8.17", settings: { optimizer: { enabled: true, runs: 6_666_666 } } },
  networks: {
    mainnet: {
      url: env.MAINNET_NODE ?? "https://mainnet.infura.io/",
      ...(env.MNEMONIC && { accounts: { mnemonic: env.MNEMONIC } }),
    },
    goerli: {
      url: env.GOERLI_NODE ?? "https://goerli.infura.io/",
      ...(env.MNEMONIC && { accounts: { mnemonic: env.MNEMONIC } }),
    },
  },
  typechain: { outDir: "types" },
  namedAccounts: {
    deployer: { default: 0 },
    owner: {
      default: 0,
      mainnet: "0x382d89aa156C473Fdb1c9565dF309e80e8fA4437",
      goerli: "0x1801f5EAeAbA3fD02cBF4b7ED1A7b58AD84C0705",
    },
  },
} as HardhatUserConfig;
