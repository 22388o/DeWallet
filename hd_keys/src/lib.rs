pub mod bip32;
pub use bip32::{BIP32};
use walletd_coins::{CryptoCoin, CryptoWallet};
use std::fmt;

#[derive(Default, PartialEq, Eq)]
pub enum DerivType {
    #[default]
    BIP32,
    BIP44, 
    BIP49,
    BIP84,
}
impl DerivType {
    pub fn purpose(&self) -> &str {
        match self {
            DerivType::BIP32 => "0'",
            DerivType::BIP44 => "44'",
            DerivType::BIP49 => "49'",
            DerivType::BIP84 => "84'",
        }
    }

    pub fn derive_first_account(&self, master_node: &BIP32, coin: &CryptoCoin) -> Result<BIP32, String> {
        let derived_account_path = format!("{}{}{}{}", "m/", &self.purpose(), coin.coin_type(), "'/0'");
        BIP32::derived_from_master_with_specified_path(&master_node, derived_account_path)
    }
    pub fn derive_first_address(&self, master_node: &BIP32, coin: &CryptoCoin) -> Result<BIP32, String> {
        let derived_first_account = &self.derive_first_account(master_node, coin)?;
        println!("First Derived Account HD Key Info: \n{}", derived_first_account);
        let bip84_deriv_path = format!("{}{}{}{}", "m/", &self.purpose(), coin.coin_type(), "'/0'/0/0");
        BIP32::derived_from_master_with_specified_path(
            &master_node,
            bip84_deriv_path)
    }

}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DerivPathComponent {
    Master,
    IndexHardened(u32),
    IndexNotHardened(u32),
}



#[derive(Default, PartialEq, Eq, Copy, Clone)]
pub enum NetworkType {
    #[default]
    MainNet,
    TestNet,
}

impl fmt::Display for NetworkType {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NetworkType::MainNet => fmt.write_str("mainnet")?,
            NetworkType::TestNet => fmt.write_str("testnet")?,
        };
        Ok(())
    }
}


