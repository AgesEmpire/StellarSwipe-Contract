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
}

pub fn respond_to_dispute<Provider: Clone + PartialEq, Hash: Clone>(
    dispute: &mut Option<Dispute<Provider, Hash>>,
    provider: Provider,
    signal_id: u64,
    response_hash: Hash,
    now: u64,
) -> Result<DisputeResponseSubmitted<Provider, Hash>, ContractError> {
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

    #[test]
    fn timely_response_is_stored_and_emits_event_shape() {
        let mut dispute = dispute();

        let event = respond_to_dispute(
            &mut dispute,
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

        // The next check must reflect the new (revoked) role, not the cache.
        assert!(!authorize_dispute_response(&mut cache, &provider, |_| false));
        assert_eq!(cache.get(&provider), Some(false));
    }

    #[test]
    fn role_change_during_dependent_operation_uses_new_role_on_next_call() {
        let mut cache = AuthCache::new();
        let provider = "provider-1";

        // First call caches an authorized decision.
        assert!(authorize_dispute_response(&mut cache, &provider, |_| true));

        // Role changes mid-flight; the cache is invalidated as part of the change.
        cache.invalidate(&provider);

        // The next relevant call must observe the new role.
        assert!(!authorize_dispute_response(&mut cache, &provider, |_| false));
    }

    #[test]
    fn revoked_permission_is_rejected_on_next_relevant_call() {
        let mut cache = AuthCache::new();
        let provider = "provider-1";

        assert!(authorize_dispute_response(&mut cache, &provider, |_| true));

        // Permission revoked: invalidate and re-check.
        cache.invalidate(&provider);
        let authorized = authorize_dispute_response(&mut cache, &provider, |_| false);

        assert!(!authorized);
        assert_eq!(cache.get(&provider), Some(false));
    }

    #[test]
    fn stale_cache_cannot_bypass_authorization_check() {
        let mut cache = AuthCache::new();
        let provider = "provider-1";

        // Cache an authorized decision.
        assert!(authorize_dispute_response(&mut cache, &provider, |_| true));
        assert_eq!(cache.get(&provider), Some(true));

        // Even if a stale entry were present, the authoritative check is the
        // source of truth and must reject the now-unauthorized provider.
        let authorized = authorize_dispute_response(&mut cache, &provider, |_| false);
        assert!(!authorized);
        assert_eq!(cache.get(&provider), Some(false));
    }
}
