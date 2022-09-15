import type { DeployFunction } from "hardhat-deploy/types";

const func: DeployFunction = async ({ deployments: { deploy, get }, getNamedAccounts }) => {
  const { deployer, owner } = await getNamedAccounts();
  await deploy("Liquidator", {
    args: [owner, (await get("UniswapV3Factory")).address],
    from: deployer,
    log: true,
  });
};

func.tags = ["Liquidator"];

export default func;
