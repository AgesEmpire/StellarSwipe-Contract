//! Common types and helpers shared across contracts.

use soroban_sdk::{contracttype, Address, Env, Vec};

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
