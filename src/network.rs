use ethers::types::U256;

use crate::{config::Config, fixed_point_math::FixedPointMath};

#[derive(Clone, Copy, Default, Debug)]
pub struct NetworkStatus {
    pub gas_price: Option<U256>,
    pub gas_used: Option<U256>,
    pub eth_price: U256,
}

pub trait NetworkActions {
    fn tx_cost(&self, data: &Network, status: NetworkStatus) -> U256;
}

pub struct Network {
    gas_price: U256,
    gas_used: U256,
    actions: Box<dyn NetworkActions + Send + Sync>,
}

impl Network {
    pub fn from_config(config: &Config) -> Self {
        Self {
            gas_price: config.gas_price,
            gas_used: config.gas_used,
            actions: Self::get_network_from(config),
        }
    }

    pub fn tx_cost(&self, status: NetworkStatus) -> U256 {
        self.actions.tx_cost(self, status)
    }

    pub fn default_gas_used(&self) -> U256 {
        self.gas_used
    }

    fn get_network_from(config: &Config) -> Box<dyn NetworkActions + Send + Sync> {
        match config.chain_id {
            1 | 5 => Self::get_network::<Ethereum>(),
            10 => Box::new(Optimism {
                l1_gas_used: config.l1_gas_used,
                l1_gas_price: config.l1_gas_price,
            }),
            _ => panic!("Unknown network!"),
        }
    }

    fn get_network<T: 'static + NetworkActions + Default + Send + Sync>(
    ) -> Box<dyn NetworkActions + Send + Sync> {
        Box::<T>::default()
    }
}

#[derive(Default)]
struct Ethereum;

#[derive(Default)]
struct Optimism {
    l1_gas_used: U256,
    l1_gas_price: U256,
}

impl NetworkActions for Ethereum {
    fn tx_cost(&self, data: &Network, status: NetworkStatus) -> U256 {
        (status.gas_price.unwrap_or(data.gas_price) * status.gas_used.unwrap_or(data.gas_used))
            .mul_wad_down(status.eth_price)
    }
}

impl NetworkActions for Optimism {
    fn tx_cost(&self, _: &Network, status: NetworkStatus) -> U256 {
        (self.l1_gas_price * self.l1_gas_used).mul_wad_down(status.eth_price)
    }
}
