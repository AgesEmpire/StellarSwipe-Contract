//! Custom position tagging for user-defined portfolio grouping (Issue #703).
//!
//! Users can tag their own open positions with arbitrary bounded-length string tags
//! (e.g. "long-term", "experimental") for personal tracking purposes.
//!
//! Storage keys:
//! - `DataKey::PositionTag(user, position_id)` → `String` (the tag)
//! - `DataKey::UserPositionsByTag(user, tag)` → `Vec<u64>` (position IDs with that tag)

use soroban_sdk::{Address, Env, String, Vec};

use crate::storage::DataKey;

/// Maximum length of a user-defined position tag (in bytes).
pub const MAX_TAG_LENGTH: u32 = 32;
/// Maximum number of positions that can be stored per tag index.
pub const MAX_POSITIONS_PER_TAG: u32 = 200;

/// Emit `PositionTagged { user, position_id, tag }` event.
fn emit_position_tagged(env: &Env, user: Address, position_id: u64, tag: Option<String>) {
    let topics = (soroban_sdk::Symbol::new(env, "position_tagged"),);
    env.events().publish(topics, (user, position_id, tag));
}

/// Verify that `position_id` exists and belongs to `user`.
fn require_position_owner(env: &Env, user: &Address, position_id: u64) {
    let key = DataKey::UserPositions(user.clone());
    let list: Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    let mut found = false;
    for i in 0..list.len() {
        if let Some(pid) = list.get(i) {
            if pid == position_id {
                found = true;
                break;
            }
        }
    }
    if !found {
        panic!("position not found for user");
    }
}

/// Validate that a tag string is within the allowed length bounds.
fn validate_tag(env: &Env, tag: &String) {
    if tag.len() == 0 || tag.len() > MAX_TAG_LENGTH {
        panic!("tag must be between 1 and {MAX_TAG_LENGTH} characters");
    }
}

/// Tag a position with a user-defined string.
///
/// - Restricted to the position owner (`user` must sign).
/// - `tag` must be 1–32 characters.
/// - If the position was previously tagged, the old tag index is cleaned up.
/// - Emits `PositionTagged` event.
pub fn tag_position(
    env: &Env,
    user: Address,
    position_id: u64,
    tag: String,
) -> Result<(), ()> {
    user.require_auth();
    require_position_owner(env, &user, position_id);
    validate_tag(env, &tag);

    // Remove existing tag if present (clean up old reverse index).
    let old_tag_key = DataKey::PositionTag(user.clone(), position_id);
    if let Some(old_tag) = env.storage().persistent().get::<_, String>(&old_tag_key) {
        let old_index_key = DataKey::UserPositionsByTag(user.clone(), old_tag);
        let mut old_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&old_index_key)
            .unwrap_or_else(|| Vec::new(env));
        let mut updated = Vec::new(env);
        for i in 0..old_ids.len() {
            if let Some(id) = old_ids.get(i) {
                if id != position_id {
                    updated.push_back(id);
                }
            }
        }
        if updated.len() > 0 {
            env.storage().persistent().set(&old_index_key, &updated);
        } else {
            env.storage().persistent().remove(&old_index_key);
        }
    }

    // Store the new tag.
    env.storage().persistent().set(&old_tag_key, &tag);

    // Update the tag→positions reverse index.
    let new_index_key = DataKey::UserPositionsByTag(user.clone(), tag.clone());
    let mut tag_ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&new_index_key)
        .unwrap_or_else(|| Vec::new(env));

    // Deduplicate.
    let mut already_present = false;
    for i in 0..tag_ids.len() {
        if let Some(id) = tag_ids.get(i) {
            if id == position_id {
                already_present = true;
                break;
            }
        }
    }
    if !already_present {
        if tag_ids.len() >= MAX_POSITIONS_PER_TAG {
            panic!("too many positions with this tag");
        }
        tag_ids.push_back(position_id);
        env.storage().persistent().set(&new_index_key, &tag_ids);
    }

    emit_position_tagged(env, user, position_id, Some(tag));
    Ok(())
}

/// Remove a tag from a position (untag).
pub fn untag_position(env: &Env, user: Address, position_id: u64) {
    user.require_auth();
    require_position_owner(env, &user, position_id);

    let old_tag_key = DataKey::PositionTag(user.clone(), position_id);
    if let Some(old_tag) = env.storage().persistent().get::<_, String>(&old_tag_key) {
        let old_index_key = DataKey::UserPositionsByTag(user.clone(), old_tag);
        let mut old_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&old_index_key)
            .unwrap_or_else(|| Vec::new(env));
        let mut updated = Vec::new(env);
        for i in 0..old_ids.len() {
            if let Some(id) = old_ids.get(i) {
                if id != position_id {
                    updated.push_back(id);
                }
            }
        }
        if updated.len() > 0 {
            env.storage().persistent().set(&old_index_key, &updated);
        } else {
            env.storage().persistent().remove(&old_index_key);
        }
    }

    env.storage().persistent().remove(&old_tag_key);
    emit_position_tagged(env, user, position_id, None);
}

/// Returns all position IDs for a user that have been tagged with `tag`.
pub fn get_positions_by_tag(env: &Env, user: Address, tag: String) -> Vec<u64> {
    let key = DataKey::UserPositionsByTag(user, tag);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

/// Returns the tag (if any) for a specific position owned by `user`.
pub fn get_position_tag(env: &Env, user: Address, position_id: u64) -> Option<String> {
    env.storage()
        .persistent()
        .get(&DataKey::PositionTag(user, position_id))
}


#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Env};

    #[contract]
    pub struct TagHarness;

    #[contractimpl]
    impl TagHarness {
        pub fn simulate_open_position(env: Env, user: Address, position_id: u64) {
            let key = DataKey::UserPositions(user.clone());
            let mut list: Vec<u64> = env
                .storage()
                .persistent()
                .get(&key)
                .unwrap_or_else(|| Vec::new(&env));
            list.push_back(position_id);
            env.storage().persistent().set(&key, &list);
        }
    }

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(TagHarness, ());
        (env, contract_id)
    }

    fn run<F, R>(env: &Env, contract_id: &Address, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        env.as_contract(contract_id, f)
    }

    #[test]
    fn tag_and_get_positions_by_tag() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);
            let tag = String::from_str(&env, "long-term");

            TagHarness::simulate_open_position(env.clone(), user.clone(), 1);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 2);

            tag_position(&env, user.clone(), 1, tag.clone()).unwrap();
            tag_position(&env, user.clone(), 2, tag.clone()).unwrap();

            let ids = get_positions_by_tag(&env, user, tag);
            assert_eq!(ids.len(), 2);
        });
    }

    #[test]
    fn retag_position_removes_old_index() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 1);

            let tag1 = String::from_str(&env, "experimental");
            let tag2 = String::from_str(&env, "long-term");

            tag_position(&env, user.clone(), 1, tag1).unwrap();
            tag_position(&env, user.clone(), 1, tag2.clone()).unwrap();

            let old_ids = get_positions_by_tag(
                &env, user.clone(),
                String::from_str(&env, "experimental"),
            );
            assert_eq!(old_ids.len(), 0);

            let new_ids = get_positions_by_tag(&env, user, tag2);
            assert_eq!(new_ids.len(), 1);
            assert_eq!(new_ids.get(0).unwrap(), 1);
        });
    }

    #[test]
    fn untag_position_removes_tag_and_index() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 1);

            let tag = String::from_str(&env, "watchlist");
            tag_position(&env, user.clone(), 1, tag.clone()).unwrap();

            let ids = get_positions_by_tag(&env, user.clone(), tag.clone());
            assert_eq!(ids.len(), 1);

            untag_position(&env, user.clone(), 1);

            let ids = get_positions_by_tag(&env, user.clone(), tag);
            assert_eq!(ids.len(), 0);

            let stored_tag = get_position_tag(&env, user, 1);
            assert!(stored_tag.is_none());
        });
    }

    #[test]
    fn tag_length_validated() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 1);

            let long_tag = String::from_str(
                &env,
                "this-tag-is-way-too-long-to-be-allowed-123456789",
            );
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tag_position(&env, user.clone(), 1, long_tag).unwrap();
            }));
            assert!(result.is_err());
        });
    }

    #[test]
    fn filter_positions_by_tag() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 10);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 20);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 30);

            let lt = String::from_str(&env, "long-term");
            let exp = String::from_str(&env, "experimental");

            tag_position(&env, user.clone(), 10, lt.clone()).unwrap();
            tag_position(&env, user.clone(), 20, exp.clone()).unwrap();
            tag_position(&env, user.clone(), 30, lt.clone()).unwrap();

            let long_ids = get_positions_by_tag(&env, user.clone(), lt);
            assert_eq!(long_ids.len(), 2);

            let exp_ids = get_positions_by_tag(&env, user.clone(), exp);
            assert_eq!(exp_ids.len(), 1);
            assert_eq!(exp_ids.get(0).unwrap(), 20);
        });
    }

    #[test]
    fn get_position_tag_returns_correct_value() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);
            TagHarness::simulate_open_position(env.clone(), user.clone(), 42);

            let tag = String::from_str(&env, "favorite");
            tag_position(&env, user.clone(), 42, tag.clone()).unwrap();

            let stored = get_position_tag(&env, user, 42);
            assert_eq!(stored, Some(tag));
        });
    }
}
