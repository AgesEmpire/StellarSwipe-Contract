pub struct SignalBundle {
    pub signal_ids: Vec<u32>,
    pub allocations: Vec<u32>,
}

pub fn create_signal_bundle(signal_ids: Vec<u32>, allocations: Vec<u32>) -> SignalBundle {
    let total: u32 = allocations.iter().sum();
    assert!(total == 10000, "allocations must sum to 10000");

    SignalBundle {
        signal_ids,
        allocations,
    }
}

pub fn copy_signal_bundle(total_amount: i128, bundle: SignalBundle) -> Vec<i128> {
    bundle
        .allocations
        .iter()
        .map(|a| total_amount * (*a as i128) / 10000)
        .collect()
}
