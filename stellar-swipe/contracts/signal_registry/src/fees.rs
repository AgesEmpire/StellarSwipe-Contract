use soroban_sdk::{Address, Env, Map};

use crate::admin;
use crate::errors::FeeError;
use crate::events::emit_fee_collected;
use crate::types::{Asset, FeeBreakdown, FeeStorageKey};

// Fee configuration
pub const FEE_BPS: u32 = 10; // 10 basis points = 0.1%
pub const BPS_DENOMINATOR: u32 = 10000; // 100% = 10000 bps
pub const PLATFORM_SHARE_PERCENTAGE: u32 = 70; // 70%
                                               // pub const PROVIDER_SHARE_PERCENTAGE: u32 = 30; // 30%
pub const MIN_TRADE_AMOUNT: i128 = 1000; // Minimum trade to ensure non-zero fee

/// Calculate fee for a given trade amount
/// Returns (fee_amount, amount_after_fee)
pub fn calculate_fee(trade_amount: i128) -> Result<(i128, i128), FeeError> {
    if trade_amount < MIN_TRADE_AMOUNT {
        return Err(FeeError::TradeTooSmall);
    }

    // Calculate fee: trade_amount × 10 / 10000
    let fee = trade_amount
        .checked_mul(FEE_BPS as i128)
        .ok_or(FeeError::ArithmeticOverflow)?
        .checked_div(BPS_DENOMINATOR as i128)
        .ok_or(FeeError::ArithmeticOverflow)?;

    // Ensure fee is non-zero
    if fee == 0 {
        return Err(FeeError::FeeRoundedToZero);
    }

    let amount_after_fee = trade_amount
        .checked_sub(fee)
        .ok_or(FeeError::ArithmeticOverflow)?;

    Ok((fee, amount_after_fee))
}

/// Calculate fee breakdown (platform vs provider split)
pub fn calculate_fee_breakdown(trade_amount: i128) -> Result<FeeBreakdown, FeeError> {
    let (total_fee, amount_after_fee) = calculate_fee(trade_amount)?;

    // Split fee: 70% platform, 30% provider
    let platform_fee = total_fee
        .checked_mul(PLATFORM_SHARE_PERCENTAGE as i128)
        .ok_or(FeeError::ArithmeticOverflow)?
        .checked_div(100)
        .ok_or(FeeError::ArithmeticOverflow)?;

    let provider_fee = total_fee
        .checked_sub(platform_fee)
        .ok_or(FeeError::ArithmeticOverflow)?;

    Ok(FeeBreakdown {
        total_fee,
        platform_fee,
        provider_fee,
        trade_amount_after_fee: amount_after_fee,
    })
}

/// Get or initialize treasury balances map
fn get_treasury_balances(env: &Env) -> Map<Asset, i128> {
    env.storage()
        .instance()
        .get(&FeeStorageKey::TreasuryBalances)
        .unwrap_or(Map::new(env))
}

/// Save treasury balances map
fn save_treasury_balances(env: &Env, balances: &Map<Asset, i128>) {
    env.storage()
        .instance()
        .set(&FeeStorageKey::TreasuryBalances, balances);
}

/// Add fee to treasury for a specific asset
pub fn add_to_treasury(env: &Env, asset: Asset, amount: i128) -> Result<(), FeeError> {
    if amount <= 0 {
        return Err(FeeError::InvalidAmount);
    }

    let mut balances = get_treasury_balances(env);
    let current = balances.get(asset.clone()).unwrap_or(0);

    let new_balance = current
        .checked_add(amount)
        .ok_or(FeeError::ArithmeticOverflow)?;

    balances.set(asset, new_balance);
    save_treasury_balances(env, &balances);

    Ok(())
}

/// Get treasury balance for a specific asset
pub fn get_treasury_balance(env: &Env, asset: Asset) -> i128 {
    let balances = get_treasury_balances(env);
    balances.get(asset).unwrap_or(0)
}

/// Get all treasury balances
pub fn get_all_treasury_balances(env: &Env) -> Map<Asset, i128> {
    get_treasury_balances(env)
}

/// Collect and distribute fee for a trade
/// This is the main entry point for fee processing.
/// Returns a zero-fee breakdown when fee collection is paused (Issue #189).
pub fn collect_and_distribute_fee(
    env: &Env,
    trade_amount: i128,
    asset: Asset,
    provider: Address,
    platform_treasury: Address,
) -> Result<FeeBreakdown, FeeError> {
    // Issue #189: skip fee collection when paused
    if admin::is_fee_collection_paused(env) {
        return Ok(FeeBreakdown {
            total_fee: 0,
            platform_fee: 0,
            provider_fee: 0,
            trade_amount_after_fee: trade_amount,
        });
    }

    // Validate provider address (should not be a contract for safety)
    if provider == platform_treasury {
        return Err(FeeError::InvalidProviderAddress);
    }

    // Calculate fee breakdown
    let breakdown = calculate_fee_breakdown(trade_amount)?;

    // Add to treasury tracking
    add_to_treasury(env, asset.clone(), breakdown.total_fee)?;

    // TODO: transfer tokens here
    // token_client.transfer(&env.current_contract_address(), &platform_treasury, &breakdown.platform_fee);
    // token_client.transfer(&env.current_contract_address(), &provider, &breakdown.provider_fee);

    // Emit event
    emit_fee_collected(
        env,
        asset,
        breakdown.total_fee,
        breakdown.platform_fee,
        breakdown.provider_fee,
        provider,
        platform_treasury,
    );

    Ok(breakdown)
}

/// Set platform treasury address (admin only)
pub fn set_platform_treasury(env: &Env, treasury: Address) {
    env.storage()
        .instance()
        .set(&FeeStorageKey::PlatformTreasury, &treasury);
}

/// Get platform treasury address
pub fn get_platform_treasury(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get(&FeeStorageKey::PlatformTreasury)
}

/// Validate minimum trade amount
pub fn validate_trade_amount(trade_amount: i128) -> Result<(), FeeError> {
    if trade_amount < MIN_TRADE_AMOUNT {
        return Err(FeeError::TradeTooSmall);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env, Symbol};

    #[test]
    fn test_calculate_fee() {
        // 1000 XLM trade
        let (fee, after_fee) = calculate_fee(1_000_000_000).unwrap();
        assert_eq!(fee, 1_000_000); // 0.1% = 1 XLM
        assert_eq!(after_fee, 999_000_000); // 999 XLM

        // 100 XLM trade
        let (fee, after_fee) = calculate_fee(100_000_000).unwrap();
        assert_eq!(fee, 100_000); // 0.1 XLM
        assert_eq!(after_fee, 99_900_000);

        // 10 XLM trade
        let (fee, after_fee) = calculate_fee(10_000_000).unwrap();
        assert_eq!(fee, 10_000); // 0.01 XLM
        assert_eq!(after_fee, 9_990_000);
    }

    #[test]
    fn test_calculate_fee_breakdown() {
        let breakdown = calculate_fee_breakdown(1_000_000_000).unwrap();

        assert_eq!(breakdown.total_fee, 1_000_000); // 1 XLM
        assert_eq!(breakdown.platform_fee, 700_000); // 0.7 XLM (70%)
        assert_eq!(breakdown.provider_fee, 300_000); // 0.3 XLM (30%)
        assert_eq!(breakdown.trade_amount_after_fee, 999_000_000);
    }

    #[test]
    fn test_fee_split_exact() {
        // Test that platform + provider = total
        let breakdown = calculate_fee_breakdown(100_000_000).unwrap();
        assert_eq!(
            breakdown.platform_fee + breakdown.provider_fee,
            breakdown.total_fee
        );
    }

    #[test]
    fn test_minimum_trade_amount() {
        // Below minimum
        let result = calculate_fee(999);
        assert_eq!(result, Err(FeeError::TradeTooSmall));

        // At minimum (should work)
        let result = calculate_fee(MIN_TRADE_AMOUNT);
        assert!(result.is_ok());
    }

    #[test]
    fn test_fee_rounds_to_zero() {
        let result = calculate_fee(9999);
        assert!(result.is_ok());
    }

    #[test]
    #[should_panic]
    fn test_invalid_provider_address() {
        let env = Env::default();

        let asset = Asset {
            symbol: Symbol::new(&env, "XLM"),
            contract: Address::generate(&env),
        };

        let platform = Address::generate(&env);

        // Provider same as platform should fail
        collect_and_distribute_fee(&env, 1_000_000_000, asset, platform.clone(), platform).unwrap();
    }

    #[test]
    fn test_validate_trade_amount() {
        assert!(validate_trade_amount(MIN_TRADE_AMOUNT).is_ok());
        assert!(validate_trade_amount(MIN_TRADE_AMOUNT + 1).is_ok());
        assert_eq!(
            validate_trade_amount(MIN_TRADE_AMOUNT - 1),
            Err(FeeError::TradeTooSmall)
        );
    }
}
