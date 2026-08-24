# Implement Referral Fee-Share Mechanism

## Summary
This PR implements an on-chain referral fee-share feature in the `fee_collector` smart contract, complementing off-chain tracking mechanisms. When a user (referee) with a registered referrer completes a trade, a configured percentage of the gross trade fee is automatically routed directly to the referrer's address.

## Changes
- **Storage:** Added `Referral(Address)` mapping to store referee-to-referrer relationships and `ReferralFeeShareBps` to store the admin-configurable split percentage.
- **Entrypoints:** 
  - `register_referral`: Allows a user to register a referrer. Rejects self-referrals and enforces immutability (cannot re-attribute once set).
  - `admin_override_referral`: Admin-only function to forcibly override a referral attribution.
  - `set_referral_fee_share`: Admin-only function to configure the percentage (in basis points) of the fee allocated to the referrer.
- **Fee Distribution (`collect_fee_with_recovery`):** Integrated the referral fee-share split. When calculating fees, the referral amount is safely subtracted from the distributable pool to ensure the total fee charged to the user remains exactly the same. The burn amount remains based on the total fee, while the referral payout limits the maximum treasury / revenue share distributions.
- **Events:** Added `ReferralRegistered`, `ReferralFeeShareUpdated`, and `ReferralFeePaid` events to ensure off-chain trackers can seamlessly index referral payouts.
- **Tests:** Added a comprehensive test suite `referral_tests.rs` covering all core scenarios, error conditions, and invariant enforcement.

## Acceptance Criteria Checklist
- [x] **Data model added**: Storage mapping for relationships and fee-share percentage.
- [x] **Registration entrypoint**: Rejects self-referral, enforces immutability (without admin override).
- [x] **Admin configuration entrypoints**: `set_referral_fee_share` and `admin_override_referral` successfully implemented.
- [x] **Fee distribution integration**: Referrer receives configured split; total trader fee remains unchanged.
- [x] **Event emission**: Events track registration and payouts.
- [x] **Unit tests**: Full coverage for the aforementioned acceptance criteria.

## Testing Performed
- Ran the existing test suite via `make test` / `cargo test`, ensuring backward compatibility.
- Unit tests written verifying: baseline execution without a referrer, proper splits with a referrer, self-referral rejection, re-attribution rejection, admin override success, and correct event emission.

## Security & Edge-Case Notes
- **Total Fee Invariance**: The referral split is subtracted from the `distributable` pool (after burn). The amount the trader transfers remains exactly the same (`fee_amount_floor`).
- **Precision/Rounding & Underflow Prevention**: Safely bounded the referral amount and revenue share by the current `remaining_distributable` balance via `saturating_sub` and limit-checks to guarantee that the contract does not attempt to distribute more funds than it holds, averting arithmetic underflow panics or treasury deficits.
- **Access Control**: Configuration and overrides are heavily gated using the standard admin check (`get_admin().require_auth()`).
- **Self-Referral**: Handled by rejecting `referee == referrer`.

## Open Questions / Follow-ups
- Is the 100% (10,000 bps) max boundary for `ReferralFeeShareBps` appropriate, or should it be capped lower (e.g. 50%) to ensure a guaranteed cut for the treasury?
- When both referral share and revenue share are high (e.g. combined > 100% of the distributable balance), the current logic favors the referrer and caps the revenue share at whatever is left. Should this priority be adjusted?
