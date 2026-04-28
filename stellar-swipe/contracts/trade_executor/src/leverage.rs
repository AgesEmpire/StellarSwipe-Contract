pub fn execute_copy_trade_with_leverage(amount: i128, leverage_multiplier: u32) -> (i128, i128) {
    assert!(
        leverage_multiplier >= 1 && leverage_multiplier <= 3,
        "invalid leverage"
    );

    let borrowed = amount * (leverage_multiplier as i128 - 1);
    let total_position = amount + borrowed;

    (total_position, borrowed)
}

pub fn should_liquidate(position_value: i128, borrowed: i128) -> bool {
    position_value < borrowed * 11 / 10
}
