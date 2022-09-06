import "dotenv/config";
import "hardhat-deploy";
import "@typechain/hardhat";
import "@nomiclabs/hardhat-waffle";
import { env } from "process";
import { extendConfig } from "hardhat/config";
import type {
  HardhatConfig,
  HardhatNetworkConfig,
  HardhatNetworkForkingConfig as HardhatNetworkForkingConfigExtended,
  HardhatNetworkUserConfig,
  HardhatUserConfig,
} from "hardhat/types";

const config: HardhatUserConfig = {
  solidity: { version: "0.8.16", settings: { optimizer: { enabled: true, runs: 6_666_666 } } },
  networks: {
    hardhat: { ...(env.RINKEBY_NODE && { forking: { network: "rinkeby", url: env.RINKEBY_NODE as string } }) },
    rinkeby: {
      url: env.RINKEBY_NODE ?? "https://rinkeby.infura.io/",
      ...(env.MNEMONIC && { accounts: { mnemonic: env.MNEMONIC } }),
    },
  },
  namedAccounts: { deployer: { default: 0 }, borrower: { default: "0xDb90CDB64CfF03f254e4015C4F705C3F3C834400" } },
  typechain: { outDir: "types" },
};

export default config;

extendConfig((hardhatConfig: HardhatConfig, userConfig: Readonly<HardhatUserConfig>) => {
  if (!userConfig.networks) return;
  for (const [name, networkConfig] of Object.entries(userConfig.networks)) {
    if (!networkConfig) continue;
    const { forking } = networkConfig as HardhatNetworkUserConfig;
    if (!forking?.network) continue;
    ((hardhatConfig.networks[name] as HardhatNetworkConfig).forking as HardhatNetworkForkingConfigExtended).network =
      forking.network;
  }
});

declare module "hardhat/types/config" {
  export interface HardhatNetworkForkingUserConfig {
    network?: string;
  }

  export interface HardhatNetworkForkingConfig {
    network?: string;
  }
}
