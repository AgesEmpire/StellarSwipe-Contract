use soroban_sdk::{contractclient, Address, Env, String, Vec};
use crate::lib::AssetPair;

#[contractclient(name = "PriceOracleClient")]
pub trait PriceOracleTrait {
    fn get_price(e: Env, pair: AssetPair) -> i128;
    
    fn get_price_with_confidence(e: Env, pair: AssetPair) -> (i128, u32);
    
    fn add_price_source(e: Env, source: Address, weight: u32);
    
    fn remove_price_source(e: Env, source: Address);
    
    fn update_price(e: Env, pair: AssetPair, price: i128, source_name: String, confidence: u32);
}