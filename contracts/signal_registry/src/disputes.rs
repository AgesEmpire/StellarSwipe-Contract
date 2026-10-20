pub const RESPONSE_WINDOW_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Dispute<Provider, Hash> {
    pub signal_id: u64,
    pub provider: Provider,
    pub dispute_hash: Hash,
    pub filed_at: u64,
    pub response: Option<DisputeResponse<Provider, Hash>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisputeResponse<Provider, Hash> {
    pub provider: Provider,
    pub response_hash: Hash,
    pub submitted_at: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisputeResponseSubmitted<Provider, Hash> {
    pub signal_id: u64,
    pub provider: Provider,
    pub response_hash: Hash,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContractError {
    DisputeNotFound,
    UnauthorizedProvider,
    ResponseWindowClosed,
    /// The authenticated caller does not match the address bound to the
    /// operation. Returned when authorization is derived from the
    /// authenticated invocation context rather than caller-supplied metadata.
    UnauthorizedCaller,
    /// Aggregated delegated voting power overflowed checked arithmetic.
    VotingPowerOverflow,
    /// Aggregated delegated voting power exceeds the proposal-specific cap.
    VotingPowerCapExceeded,
}

/// Authorization context for a dispute operation.
///
/// `authenticated_caller` MUST be derived from the Soroban invocation context
/// (i.e. the address that passed `require_auth`), never from
/// caller-controlled metadata such as a function argument or event payload.
/// Binding the check to this value keeps authorization valid even when the
/// authorization tree changes (nested invocations, intermediary contracts),
/// because the authenticated address is invariant across the tree.
pub struct AuthContext<Provider> {
    pub authenticated_caller: Provider,
}

pub fn respond_to_dispute<Provider: Clone + PartialEq, Hash: Clone>(
    dispute: &mut Option<Dispute<Provider, Hash>>,
    auth: &AuthContext<Provider>,
    provider: Provider,
    signal_id: u64,
    response_hash: Hash,
    now: u64,
) -> Result<DisputeResponseSubmitted<Provider, Hash>, ContractError> {
    // Bind authorization to the authenticated caller, not to the
    // caller-supplied `provider` argument. This prevents an intermediary
    // contract from spoofing the provider via metadata while the
    // authorization tree is being traversed.
    if auth.authenticated_caller != provider {
        return Err(ContractError::UnauthorizedCaller);
    }

    let dispute = dispute.as_mut().ok_or(ContractError::DisputeNotFound)?;

    if dispute.signal_id != signal_id {
        return Err(ContractError::DisputeNotFound);
    }
    if dispute.provider != provider {
        return Err(ContractError::UnauthorizedProvider);
    }
    if now > dispute.filed_at.saturating_add(RESPONSE_WINDOW_SECONDS) {
        return Err(ContractError::ResponseWindowClosed);
    }

    dispute.response = Some(DisputeResponse {
        provider: provider.clone(),
        response_hash: response_hash.clone(),
        submitted_at: now,
    });

    Ok(DisputeResponseSubmitted {
        signal_id,
        provider,
        response_hash,
    })
}

pub fn get_dispute_for_admin<Provider: Clone, Hash: Clone>(
    dispute: &Option<Dispute<Provider, Hash>>,
) -> Option<Dispute<Provider, Hash>> {
    dispute.clone()
}

/// A single delegation of voting power from a delegator to a delegatee for a
/// specific governance proposal.
///
/// `delegator` identifies the source of the power. It is used to reject
/// duplicate or overlapping delegations so the same delegator cannot be
/// counted more than once when aggregating a proposal's delegated power.
#[derive(Clone, Debug, PartialEq)]
pub struct Delegation<Provider> {
    pub proposal_id: u64,
    pub delegator: Provider,
    pub delegatee: Provider,
    pub power: u64,
}

/// Aggregates delegated voting power for a single governance proposal and
/// validates it against the proposal-specific cap.
///
/// Guarantees:
/// - Power is summed with checked arithmetic; overflow yields
///   `VotingPowerOverflow` instead of wrapping.
/// - Duplicate or overlapping delegations (same delegator counted twice) are
///   ignored, so they cannot inflate the total.
/// - Delegations for other proposals are skipped.
/// - The aggregated total is validated against `cap` before being returned;
///   exceeding the cap yields `VotingPowerCapExceeded`.
pub fn aggregate_delegated_power<Provider: Clone + PartialEq>(
    delegations: &[Delegation<Provider>],
    proposal_id: u64,
    cap: u64,
) -> Result<u64, ContractError> {
    let mut seen: Vec<Provider> = Vec::new();
    let mut total: u64 = 0;

    for delegation in delegations {
        if delegation.proposal_id != proposal_id {
            continue;
        }

        // Reject duplicate/overlapping delegations: a delegator may only
        // contribute once per proposal, regardless of how many entries exist.
        if seen.iter().any(|d| *d == delegation.delegator) {
            continue;
        }
        seen.push(delegation.delegator.clone());

        total = total
            .checked_add(delegation.power)
            .ok_or(ContractError::VotingPowerOverflow)?;
    }

    if total > cap {
        return Err(ContractError::VotingPowerCapExceeded);
    }

    Ok(total)
}

/// Authorization cache used to gate dispute operations.
///
/// The cache is intentionally *not* authoritative: every authorization check
/// re-reads the current role/permission from the source of truth so that a
/// cached entry can never outlive a state change that revokes or replaces it.
/// `invalidate` is called whenever a role or permission changes so that no
/// stale entry can bypass a subsequent authorization check.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuthCache<Provider> {
    entries: Vec<(Provider, bool)>,
}

impl<Provider: Clone + PartialEq> AuthCache<Provider> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Records the last observed authorization decision for a provider.
    pub fn record(&mut self, provider: Provider, authorized: bool) {
        if let Some(entry) = self.entries.iter_mut().find(|(p, _)| *p == provider) {
            entry.1 = authorized;
        } else {
            self.entries.push((provider, authorized));
        }
    }

    /// Drops any cached decision for a provider. Called on role/permission
    /// changes so a revoked permission cannot be served from cache.
    pub fn invalidate(&mut self, provider: &Provider) {
        self.entries.retain(|(p, _)| p != provider);
    }

    /// Returns the cached decision, if any. Callers must treat a `None` as a
    /// cache miss and fall back to the authoritative source of truth.
    pub fn get(&self, provider: &Provider) -> Option<bool> {
        self.entries
            .iter()
            .find(|(p, _)| p == provider)
            .map(|(_, authorized)| *authorized)
    }
}

/// Authoritative authorization check for dispute responses.
///
/// `is_authorized` is the source of truth (e.g. current role/permission
/// lookup). The cache is consulted only as a fast path and is always
/// invalidated on state changes, so a stale entry can never bypass this check.
pub fn authorize_dispute_response<Provider: Clone + PartialEq>(
    cache: &mut AuthCache<Provider>,
    provider: &Provider,
    is_authorized: impl Fn(&Provider) -> bool,
) -> bool {
    let authorized = is_authorized(provider);
    cache.record(provider.clone(), authorized);
    authorized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispute() -> Option<Dispute<&'static str, &'static str>> {
        Some(Dispute {
            signal_id: 7,
            provider: "provider-1",
            dispute_hash: "ipfs://dispute",
            filed_at: 1_700_000_000,
            response: None,
        })
    }

    fn auth(caller: &'static str) -> AuthContext<&'static str> {
        AuthContext {
            authenticated_caller: caller,
        }
    }

    #[test]
    fn timely_response_is_stored_and_emits_event_shape() {
        let mut dispute = dispute();

        let event = respond_to_dispute(
            &mut dispute,
            &auth("provider-1"),
            "provider-1",
            7,
            "ipfs://response",
            1_700_000_000 + 60,
        )
        .unwrap();

        assert_eq!(
            event,
            DisputeResponseSubmitted {
                signal_id: 7,
                provider: "provider-1",
                response_hash: "ipfs://response",
            }
        );
        let admin_view = get_dispute_for_admin(&dispute).unwrap();
        assert_eq!(admin_view.dispute_hash, "ipfs://dispute");
        assert_eq!(admin_view.response.unwrap().response_hash, "ipfs://response");
    }

    #[test]
    fn late_response_is_rejected() {
        let mut dispute = dispute();

        let result = respond_to_dispute(
            &mut dispute,
            &auth("provider-1"),
            "provider-1",
            7,
            "ipfs://response",
            1_700_000_000 + RESPONSE_WINDOW_SECONDS + 1,
        );

        assert_eq!(result, Err(ContractError::ResponseWindowClosed));
        assert!(dispute.unwrap().response.is_none());
    }

    #[test]
    fn no_response_remains_visible_to_admin() {
        let dispute = dispute();

        let admin_view = get_dispute_for_admin(&dispute).unwrap();

        assert_eq!(admin_view.signal_id, 7);
        assert_eq!(admin_view.dispute_hash, "ipfs://dispute");
        assert!(admin_view.response.is_none());
    }

    // --- Delegated voting power cap tests (issue #1208) ---

    fn delegation(
        proposal_id: u64,
        delegator: &'static str,
        delegatee: &'static str,
        power: u64,
    ) -> Delegation<&'static str> {
        Delegation {
            proposal_id,
            delegator,
            delegatee,
            power,
        }
    }

    #[test]
    fn aggregated_power_below_cap_is_returned() {
        let delegations = [
            delegation(1, "a", "delegate", 40),
            delegation(1, "b", "delegate", 50),
        ];

        assert_eq!(aggregate_delegated_power(&delegations, 1, 100), Ok(90));
    }

    #[test]
    fn aggregated_power_exactly_at_cap_is_allowed() {
        let delegations = [
            delegation(1, "a", "delegate", 40),
            delegation(1, "b", "delegate", 60),
        ];

        assert_eq!(aggregate_delegated_power(&delegations, 1, 100), Ok(100));
    }

    #[test]
    fn aggregated_power_above_cap_is_rejected() {
        let delegations = [
            delegation(1, "a", "delegate", 60),
            delegation(1, "b", "delegate", 60),
        ];

        assert_eq!(
            aggregate_delegated_power(&delegations, 1, 100),
            Err(ContractError::VotingPowerCapExceeded)
        );
    }

    #[test]
    fn duplicate_delegations_do_not_inflate_total() {
        let delegations = [
            delegation(1, "a", "delegate", 60),
            delegation(1, "a", "delegate", 60),
        ];

        // The overlapping delegator is counted once, so the total stays at 60.
        assert_eq!(aggregate_delegated_power(&delegations, 1, 100), Ok(60));
    }

    #[test]
    fn delegations_for_other_proposals_are_ignored() {
        let delegations = [
            delegation(1, "a", "delegate", 60),
            delegation(2, "b", "delegate", 60),
        ];

        assert_eq!(aggregate_delegated_power(&delegations, 1, 100), Ok(60));
    }

    #[test]
    fn overflow_is_reported_not_wrapped() {
        let delegations = [
            delegation(1, "a", "delegate", u64::MAX),
            delegation(1, "b", "delegate", 1),
        ];

        assert_eq!(
            aggregate_delegated_power(&delegations, 1, u64::MAX),
            Err(ContractError::VotingPowerOverflow)
        );
    }

    #[test]
    fn delegation_change_during_voting_is_reflected() {
        // A delegation added mid-vote is included in the next aggregation.
        let before = [delegation(1, "a", "delegate", 40)];
        assert_eq!(aggregate_delegated_power(&before, 1, 100), Ok(40));

        let after = [
            delegation(1, "a", "delegate", 40),
            delegation(1, "b", "delegate", 70),
        ];
        assert_eq!(
            aggregate_delegated_power(&after, 1, 100),
            Err(ContractError::VotingPowerCapExceeded)
        );
    }

    // --- Authorization cache invalidation tests (issue #1082) ---

    #[test]
    fn role_change_before_dependent_operation_is_honored() {
        let mut cache = AuthCache::new();
        let provider = "provider-1";

        // Provider is authorized and the decision is cached.
        assert!(authorize_dispute_response(&mut cache, &provider, |_| true));
        assert_eq!(cache.get(&provider), Some(true));

        // Role is revoked before the dependent operation; cache is invalidated.
        cache.invalidate(&provider);
        assert_eq!(cache.get(&provider), None);

        // The next check must reflect the revoked role, not the stale cache.
        assert!(!authorize_dispute_response(&mut cache, &provider, |_| false));
        assert_eq!(cache.get(&provider), Some(false));
    }

    #[test]
    fn stale_cache_entry_cannot_bypass_authorization() {
        let mut cache = AuthCache::new();
        let provider = "provider-1";

        // Cache an authorized decision.
        assert!(authorize_dispute_response(&mut cache, &provider, |_| true));

        // Permission is revoked and the cache is invalidated.
        cache.invalidate(&provider);

        // A subsequent check consults the source of truth and denies access.
        assert!(!authorize_dispute_response(&mut cache, &provider, |_| false));
    }
}
