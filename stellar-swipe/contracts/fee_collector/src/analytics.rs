use crate::{Error, StorageKey};
use soroban_sdk::{contracttype, panic_with_error, Address, Env, Vec};

pub const SECONDS_PER_DAY: u64 = 86400;

/// Target ledgers per UTC day at ~5s per ledger (used only for temporary-entry TTL budget).
pub const LEDGERS_PER_DAY: u32 = 17_280;
/// Temporary daily buckets are extended toward this horizon (~30 days of ledgers).
pub const TEMP_FEE_BUCKET_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 30;

pub const WEEKLY_DAYS: u32 = 7;
pub const MONTHLY_DAYS: u32 = 30;

/// Cap distinct tokens recorded per day inside one bucket (storage / gas bound).
pub const MAX_TOKEN_SLOTS_PER_DAY: u32 = 32;
const MAX_MERGED_TOKENS: u32 = 96;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalyticsPeriod {
    Daily = 0,
    Weekly = 1,
    Monthly = 2,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenFeeVol {
    pub token: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DailyFeeBucket {
    pub total_fees: i128,
    pub trade_count: u64,
    pub by_token: Vec<TokenFeeVol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeAnalytics {
    pub total_fees: i128,
    pub trade_count: u64,
    pub avg_fee_per_trade: i128,
    pub top_token: Address,
}

fn current_day_number(env: &Env) -> u64 {
    env.ledger().timestamp() / SECONDS_PER_DAY
}

fn no_trades_top_token_placeholder(env: &Env) -> Address {
    Address::from_str(env, "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF")
}

pub(crate) fn load_bucket(env: &Env, day: u64) -> DailyFeeBucket {
    let key = StorageKey::DailyFees(day);
    env.storage()
        .temporary()
        .get(&key)
        .unwrap_or_else(|| DailyFeeBucket {
            total_fees: 0,
            trade_count: 0,
            by_token: Vec::new(env),
        })
}

fn upsert_day_token(env: &Env, bucket: &mut DailyFeeBucket, token: Address, add: i128) {
    let len = bucket.by_token.len();
    let mut i: u32 = 0;
    while i < len {
        let mut tv = bucket.by_token.get(i).unwrap();
        if tv.token == token {
            tv.amount = tv
                .amount
                .checked_add(add)
                .unwrap_or_else(|| panic_with_error!(&env.clone(), Error::Overflow));
            bucket.by_token.set(i, tv);
            return;
        }
        i += 1;
    }
    if bucket.by_token.len() < MAX_TOKEN_SLOTS_PER_DAY {
        bucket.by_token.push_back(TokenFeeVol { token, amount: add });
    }
}

fn merge_merged(env: &Env, merged: &mut Vec<TokenFeeVol>, token: Address, add: i128) {
    let len = merged.len();
    let mut i: u32 = 0;
    while i < len {
        let mut tv = merged.get(i).unwrap();
        if tv.token == token {
            tv.amount = tv
                .amount
                .checked_add(add)
                .unwrap_or_else(|| panic_with_error!(&env.clone(), Error::Overflow));
            merged.set(i, tv);
            return;
        }
        i += 1;
    }
    if merged.len() < MAX_MERGED_TOKENS {
        merged.push_back(TokenFeeVol { token, amount: add });
    }
}

/// Records one fee-bearing trade into the temporary daily bucket for the current ledger day.
pub(crate) fn record_daily_fee_collection(env: &Env, fee: i128, token: Address) {
    let day = current_day_number(env);
    let mut bucket = load_bucket(env, day);
    bucket.total_fees = bucket
        .total_fees
        .checked_add(fee)
        .unwrap_or_else(|| panic_with_error!(&env.clone(), Error::Overflow));
    bucket.trade_count = bucket
        .trade_count
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(&env.clone(), Error::Overflow));
    upsert_day_token(env, &mut bucket, token, fee);

    let key = StorageKey::DailyFees(day);
    let t = env.storage().temporary();
    t.set(&key, &bucket);
    t.extend_ttl(
        &key,
        TEMP_FEE_BUCKET_TTL_LEDGERS,
        TEMP_FEE_BUCKET_TTL_LEDGERS,
    );
}

pub fn get_fee_analytics(env: &Env, period: AnalyticsPeriod) -> FeeAnalytics {
    let end_day = current_day_number(env);
    let num_days: u32 = match period {
        AnalyticsPeriod::Daily => 1,
        AnalyticsPeriod::Weekly => WEEKLY_DAYS,
        AnalyticsPeriod::Monthly => MONTHLY_DAYS,
    };

    let mut total_fees: i128 = 0;
    let mut trade_count: u64 = 0;
    let mut merged: Vec<TokenFeeVol> = Vec::new(env);

    let mut d: u32 = 0;
    while d < num_days {
        let delta = (num_days - 1).saturating_sub(d) as u64;
        let day = end_day.saturating_sub(delta);
        let bucket = load_bucket(env, day);
        total_fees = total_fees
            .checked_add(bucket.total_fees)
            .unwrap_or_else(|| panic_with_error!(&env.clone(), Error::Overflow));
        trade_count = trade_count
            .checked_add(bucket.trade_count)
            .unwrap_or_else(|| panic_with_error!(&env.clone(), Error::Overflow));

        let blen = bucket.by_token.len();
        let mut bi: u32 = 0;
        while bi < blen {
            let tv = bucket.by_token.get(bi).unwrap();
            merge_merged(env, &mut merged, tv.token.clone(), tv.amount);
            bi += 1;
        }
        d += 1;
    }

    let avg_fee_per_trade = if trade_count == 0 {
        0i128
    } else {
        total_fees / (trade_count as i128)
    };

    let mut top_token = no_trades_top_token_placeholder(env);
    let mut top_amt: i128 = 0;
    if trade_count > 0 {
        let mlen = merged.len();
        let mut j: u32 = 0;
        while j < mlen {
            let tv = merged.get(j).unwrap();
            if tv.amount > top_amt {
                top_amt = tv.amount;
                top_token = tv.token.clone();
            }
            j += 1;
        }
    }

    FeeAnalytics {
        total_fees,
        trade_count,
        avg_fee_per_trade,
        top_token,
    }
}
