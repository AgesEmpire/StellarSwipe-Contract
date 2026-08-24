pub const INSURANCE_FEE_SHARE_BPS: i128 = 500;
pub const BPS_DENOMINATOR: i128 = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub struct InsurancePool {
    pub balance: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeLoss {
    pub trade_id: u64,
    pub slashed_signal: bool,
    pub stop_loss_amount: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InsuranceClaimed<User> {
    pub user: User,
    pub trade_id: u64,
    pub amount_claimed: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContractError {
    InvalidClaim,
    LossWithinStopLoss,
}

pub fn allocate_insurance_fee(pool: &mut InsurancePool, collected_fee: i128) -> i128 {
    let insurance_share = collected_fee.saturating_mul(INSURANCE_FEE_SHARE_BPS) / BPS_DENOMINATOR;
    pool.balance = pool.balance.saturating_add(insurance_share);
    insurance_share
}

pub fn claim_insurance<User: Clone>(
    pool: &mut InsurancePool,
    user: User,
    trade: &TradeLoss,
    loss_amount: i128,
) -> Result<InsuranceClaimed<User>, ContractError> {
    if !trade.slashed_signal {
        return Err(ContractError::InvalidClaim);
    }
    if loss_amount <= trade.stop_loss_amount {
        return Err(ContractError::LossWithinStopLoss);
    }

    let max_loss_payout = loss_amount / 2;
    let amount_claimed = max_loss_payout.min(pool.balance);
    pool.balance -= amount_claimed;

    Ok(InsuranceClaimed {
        user,
        trade_id: trade.trade_id,
        amount_claimed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_claim_pays_half_loss() {
        let mut pool = InsurancePool { balance: 1_000 };
        let trade = TradeLoss {
            trade_id: 42,
            slashed_signal: true,
            stop_loss_amount: 100,
        };

        let event = claim_insurance(&mut pool, "user-1", &trade, 600).unwrap();

        assert_eq!(event.amount_claimed, 300);
        assert_eq!(event.trade_id, 42);
        assert_eq!(pool.balance, 700);
    }

    #[test]
    fn invalid_claim_without_slashed_signal_is_rejected() {
        let mut pool = InsurancePool { balance: 1_000 };
        let trade = TradeLoss {
            trade_id: 43,
            slashed_signal: false,
            stop_loss_amount: 100,
        };

        let result = claim_insurance(&mut pool, "user-1", &trade, 600);

        assert_eq!(result, Err(ContractError::InvalidClaim));
        assert_eq!(pool.balance, 1_000);
    }

    #[test]
    fn pool_insufficient_caps_payout_at_balance() {
        let mut pool = InsurancePool { balance: 75 };
        let trade = TradeLoss {
            trade_id: 44,
            slashed_signal: true,
            stop_loss_amount: 100,
        };

        let event = claim_insurance(&mut pool, "user-1", &trade, 600).unwrap();

        assert_eq!(event.amount_claimed, 75);
        assert_eq!(pool.balance, 0);
    }

    #[test]
    fn collected_fees_fund_pool_at_five_percent() {
        let mut pool = InsurancePool { balance: 0 };

        let allocated = allocate_insurance_fee(&mut pool, 10_000);

        assert_eq!(allocated, 500);
        assert_eq!(pool.balance, 500);
    }
}
