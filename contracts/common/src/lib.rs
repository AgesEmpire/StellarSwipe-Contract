//! Common types and helpers shared across contracts.

#![no_std]

use soroban_sdk::{contracttype, token, Address, Env, Vec};

/// Maximum number of holdings returned in a single page.
pub const MAX_HOLDINGS_PAGE_SIZE: u32 = 50;

/// A single portfolio holding keyed by its canonical asset address.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldingEntry {
    pub asset: Address,
    pub amount: i128,
}

/// A bounded page of holdings with a stable continuation cursor.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldingsPage {
    /// Holdings in this page, ordered by canonical asset key ascending.
    pub holdings: Vec<HoldingEntry>,
    /// Cursor to pass as `start_after` for the next page; `None` when final.
    pub next_cursor: Option<Address>,
}

/// Error returned when a requested page size is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoldingsPageError {
    InvalidPageSize,
}

/// Returns the effective page size, capped at [`MAX_HOLDINGS_PAGE_SIZE`].
///
/// A page size of `0` is invalid and yields `None`.
pub fn bounded_page_size(requested: u32) -> Option<u32> {
    if requested == 0 {
        None
    } else if requested > MAX_HOLDINGS_PAGE_SIZE {
        Some(MAX_HOLDINGS_PAGE_SIZE)
    } else {
        Some(requested)
    }
}

/// Builds a bounded, stably ordered page of holdings over canonical asset keys.
///
/// `all` must be sorted ascending by `asset` (canonical key). `start_after` is
/// the exclusive continuation cursor: only entries whose asset is strictly
/// greater than the cursor are considered. Ordering is stable across pages
/// because it depends solely on the canonical asset key.
pub fn paginate_holdings(
    env: &Env,
    all: &Vec<HoldingEntry>,
    start_after: Option<Address>,
    requested_size: u32,
) -> Result<HoldingsPage, HoldingsPageError> {
    let size = match bounded_page_size(requested_size) {
        Some(size) => size,
        None => return Err(HoldingsPageError::InvalidPageSize),
    };

    let mut page: Vec<HoldingEntry> = Vec::new(env);
    let mut next_cursor: Option<Address> = None;
    let mut collected: u32 = 0;

    for entry in all.iter() {
        if let Some(ref cursor) = start_after {
            if entry.asset <= *cursor {
                continue;
            }
        }

        if collected == size {
            // There is at least one more entry beyond this page.
            next_cursor = Some(entry.asset.clone());
            break;
        }

        page.push_back(entry.clone());
        collected += 1;
    }

    Ok(HoldingsPage {
        holdings: page,
        next_cursor,
    })
}

/// Error returned when a token swap cannot be completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwapError {
    /// The requested input amount is not positive.
    InvalidAmount,
    /// The realized output is below the caller's minimum received amount.
    SlippageExceeded,
}

/// Executes a token swap: pulls `amount` of `token_in` from `from` and sends
/// `amount_out` of `token_out` to `to`.
///
/// Slippage protection is enforced by requiring `amount_out >= min_received`;
/// otherwise the call fails with [`SwapError::SlippageExceeded`] before any
/// funds move. On success a `swap` event is emitted with the swap details.
///
/// The caller is responsible for computing `amount_out` (e.g. via an oracle or
/// AMM quote) and for ensuring the contract holds enough `token_out`.
pub fn swap(
    env: &Env,
    from: &Address,
    to: &Address,
    token_in: &Address,
    token_out: &Address,
    amount: i128,
    amount_out: i128,
    min_received: i128,
) -> Result<i128, SwapError> {
    if amount <= 0 {
        return Err(SwapError::InvalidAmount);
    }

    if amount_out < min_received {
        return Err(SwapError::SlippageExceeded);
    }

    let token_in_client = token::Client::new(env, token_in);
    let token_out_client = token::Client::new(env, token_out);

    token_in_client.transfer(from, &env.current_contract_address(), &amount);
    token_out_client.transfer(&env.current_contract_address(), to, &amount_out);

    env.events().publish(
        (soroban_sdk::symbol_short!("swap"), from.clone()),
        (token_in.clone(), token_out.clone(), amount, amount_out),
    );

    Ok(amount_out)
}
