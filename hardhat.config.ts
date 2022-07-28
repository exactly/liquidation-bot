import "dotenv/config";
import "hardhat-deploy";
import { env } from "process";
import type { HardhatUserConfig } from "hardhat/types";

const config: HardhatUserConfig = {
  solidity: { version: "0.8.15", settings: { optimizer: { enabled: true, runs: 6_666_666 } } },
  networks: {
    rinkeby: {
      url: env.RINKEBY_NODE ?? "https://rinkeby.infura.io/",
      ...(env.MNEMONIC && { accounts: { mnemonic: env.MNEMONIC } }),
    },
  },
  namedAccounts: { deployer: { default: 0 } },
};

export default config;
