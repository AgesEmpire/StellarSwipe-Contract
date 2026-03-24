use crate::errors::OracleError;
use common::{Asset, AssetPair};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

pub struct OrderEntry {
    pub price: i128,
    pub amount: i128,
}

pub struct OrderBook {
    pub bids: Vec<OrderEntry>,
    pub asks: Vec<OrderEntry>,
}

/// Calculate Mid-Market Spot Price
pub fn calculate_spot_price(env: &Env, orderbook: OrderBook) -> Result<i128, OracleError> {
    if orderbook.bids.is_empty() || orderbook.asks.is_empty() {
        return Err(OracleError::EmptyOrderBook);
    }

    let best_bid = orderbook.bids.get(0).unwrap().price;
    let best_ask = orderbook.asks.get(0).unwrap().price;

    // Spread Check
    let spread = (best_ask - best_bid) * 10000 / best_bid; // Spread in basis points

    if spread > 2000 {
        // 20%
        return Err(OracleError::WideSpreadDetected);
    }

    Ok((best_bid + best_ask) / 2)
}

/// Calculate Volume Weighted Average Price (VWAP)
pub fn calculate_vwap(orderbook: OrderBook, trade_amount: i128) -> Result<i128, OracleError> {
    let mut remaining = trade_amount;
    let mut total_cost: i128 = 0;

    for ask in orderbook.asks.iter() {
        let fill_amount = if remaining < ask.amount {
            remaining
        } else {
            ask.amount
        };

        total_cost = total_cost
            .checked_add(
                fill_amount
                    .checked_mul(ask.price)
                    .ok_or(OracleError::Overflow)?,
            )
            .ok_or(OracleError::Overflow)?;

        remaining -= fill_amount;
        if remaining == 0 {
            break;
        }
    }

    if remaining > 0 {
        return Err(OracleError::InsufficientLiquidity);
    }

    Ok(total_cost / trade_amount)
}
