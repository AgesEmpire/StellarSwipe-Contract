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
}
