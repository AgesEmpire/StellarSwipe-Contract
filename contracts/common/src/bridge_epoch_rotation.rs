//! Epoch-boundary validator-set rotation for the bridge (#1414).
//!
//! Validator-set changes are made predictable by tying activation to a defined
//! bridge epoch boundary. A pending set records the epoch at which it activates
//! and cannot be promoted early. Expired or superseded pending sets follow an
//! explicit cancellation path so stale proposals never linger silently.
//!
//! ## Epoch model
//!
//! An epoch is a fixed-length window of ledgers. The epoch number for a given
//! ledger is `ledger / epoch_length` (integer division). The epoch boundary is
//! the first ledger of that epoch: `epoch * epoch_length`.
//!
//! Pending sets store their `activation_epoch` (not a raw ledger). They can
//! only be promoted once the network ledger reaches or passes that epoch's
//! opening boundary. This prevents an operator from activating a new set mid-
//! epoch, which would make the transition point ambiguous to off-chain indexers
//! and SDK consumers.
//!
//! ## Cancellation
//!
//! A pending set is cancelled when:
//! - The submitter explicitly calls [`PendingValidatorSet::cancel`].
//! - A newer pending set supersedes it (the old one is considered abandoned).
//!
//! Cancellation is tracked through [`PendingSetState`] so the reason is always
//! auditable. A cancelled set cannot be promoted.
//!
//! ## What done looks like (#1414)
//!
//! * A pending set records its `activation_epoch` and cannot activate early.
//! * Expired or superseded pending sets follow an explicit cancellation path.
//! * Tests cover rotation at and around epoch boundaries.

/// Length of a bridge epoch in ledgers. All epoch boundary calculations use
/// this constant. Must be non-zero; enforced at construction via
/// [`EpochConfig::new`].
pub const DEFAULT_EPOCH_LENGTH: u32 = 17_280; // ≈ 1 day at 5-second ledger close

/// Maximum number of epochs a pending set may remain pending before it is
/// considered expired. A pending set that has not been promoted or cancelled
/// within this window is treated as abandoned.
pub const MAX_PENDING_EPOCHS: u32 = 4;

/// Errors returned by epoch-rotation operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochRotationError {
    /// The epoch length supplied to [`EpochConfig`] is zero.
    InvalidEpochLength,
    /// The pending set's activation epoch has not yet arrived.
    EpochNotReached,
    /// The pending set was already cancelled.
    AlreadyCancelled,
    /// The pending set was already promoted to active.
    AlreadyActivated,
    /// The activation epoch is not strictly after the current epoch, so the
    /// rotation would take effect in the past or in the same epoch.
    NonMonotonicEpoch,
    /// The activation epoch is too far in the future (exceeds
    /// [`MAX_PENDING_EPOCHS`] from the current epoch).
    EpochTooFar,
    /// No pending set exists to promote or cancel.
    NoPendingSet,
    /// A pending set already exists and must be cancelled before a new one
    /// can be queued.
    PendingSetExists,
}

/// Lifecycle state of a pending validator set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingSetState {
    /// Queued and waiting for its activation epoch to arrive.
    Pending,
    /// Promoted to the active set at the activation epoch boundary.
    Activated,
    /// Cancelled explicitly by the submitter.
    CancelledBySubmitter,
    /// Superseded by a newer pending set that was queued afterwards.
    Superseded,
    /// Abandoned because the pending window (MAX_PENDING_EPOCHS) expired
    /// without promotion.
    Expired,
}

/// Epoch configuration, holding the epoch length in ledgers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpochConfig {
    /// Number of ledgers per epoch. Must be ≥ 1.
    pub epoch_length: u32,
}

impl EpochConfig {
    /// Creates an epoch config with the given `epoch_length`.
    ///
    /// Returns [`EpochRotationError::InvalidEpochLength`] when `epoch_length`
    /// is zero because a zero-length epoch makes boundary arithmetic undefined.
    pub fn new(epoch_length: u32) -> Result<Self, EpochRotationError> {
        if epoch_length == 0 {
            return Err(EpochRotationError::InvalidEpochLength);
        }
        Ok(Self { epoch_length })
    }

    /// Returns the epoch number that contains `ledger`.
    ///
    /// ```text
    /// epoch(ledger) = ledger / epoch_length
    /// ```
    pub fn epoch_of(&self, ledger: u32) -> u32 {
        ledger / self.epoch_length
    }

    /// Returns the first ledger of `epoch` (the epoch boundary ledger).
    ///
    /// ```text
    /// boundary(epoch) = epoch * epoch_length
    /// ```
    pub fn epoch_boundary(&self, epoch: u32) -> u32 {
        epoch.saturating_mul(self.epoch_length)
    }

    /// Returns `true` when `ledger` is at or past the opening boundary of
    /// `epoch`.
    pub fn epoch_reached(&self, epoch: u32, ledger: u32) -> bool {
        ledger >= self.epoch_boundary(epoch)
    }
}

/// A pending validator set queued for activation at a future epoch boundary.
///
/// The set carries an opaque `set_id` that the caller supplies (e.g. a hash of
/// the validator public keys). This module only manages lifecycle — the actual
/// key material is managed by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingValidatorSet {
    /// Caller-supplied opaque identifier for this validator set.
    pub set_id: [u8; 32],
    /// The epoch at which this set may be promoted to active.
    ///
    /// Promotion is only permitted when the current ledger is at or past
    /// `epoch_config.epoch_boundary(activation_epoch)`.
    pub activation_epoch: u32,
    /// Ledger at which this pending set was submitted.
    pub submitted_at_ledger: u32,
    /// Current lifecycle state.
    pub state: PendingSetState,
}

impl PendingValidatorSet {
    /// Creates a new pending set targeted at `activation_epoch`.
    ///
    /// `current_ledger` is used to derive the current epoch. The activation
    /// epoch must be strictly after the current epoch so the rotation never
    /// takes effect in the past.
    ///
    /// Returns [`EpochRotationError::NonMonotonicEpoch`] when
    /// `activation_epoch <= current_epoch`, and
    /// [`EpochRotationError::EpochTooFar`] when the activation epoch is more
    /// than [`MAX_PENDING_EPOCHS`] ahead of the current epoch.
    pub fn new(
        set_id: [u8; 32],
        activation_epoch: u32,
        current_ledger: u32,
        config: &EpochConfig,
    ) -> Result<Self, EpochRotationError> {
        let current_epoch = config.epoch_of(current_ledger);
        if activation_epoch <= current_epoch {
            return Err(EpochRotationError::NonMonotonicEpoch);
        }
        if activation_epoch.saturating_sub(current_epoch) > MAX_PENDING_EPOCHS {
            return Err(EpochRotationError::EpochTooFar);
        }
        Ok(Self {
            set_id,
            activation_epoch,
            submitted_at_ledger: current_ledger,
            state: PendingSetState::Pending,
        })
    }

    /// Attempts to promote this pending set to `Activated`.
    ///
    /// Succeeds only when the current ledger is at or past the activation
    /// epoch's opening boundary. Returns:
    /// - [`EpochRotationError::AlreadyCancelled`] when the set has been cancelled.
    /// - [`EpochRotationError::AlreadyActivated`] when already promoted.
    /// - [`EpochRotationError::EpochNotReached`] when the activation epoch
    ///   boundary has not yet arrived.
    pub fn try_activate(
        &mut self,
        current_ledger: u32,
        config: &EpochConfig,
    ) -> Result<(), EpochRotationError> {
        match self.state {
            PendingSetState::CancelledBySubmitter
            | PendingSetState::Superseded
            | PendingSetState::Expired => return Err(EpochRotationError::AlreadyCancelled),
            PendingSetState::Activated => return Err(EpochRotationError::AlreadyActivated),
            PendingSetState::Pending => {}
        }
        if !config.epoch_reached(self.activation_epoch, current_ledger) {
            return Err(EpochRotationError::EpochNotReached);
        }
        self.state = PendingSetState::Activated;
        Ok(())
    }

    /// Cancels this pending set at the submitter's request.
    ///
    /// A set that has already been activated or cancelled cannot be cancelled
    /// again. Cancellation is explicit: calling this method is the only way to
    /// put the set into `CancelledBySubmitter` state; it never happens
    /// implicitly.
    pub fn cancel_by_submitter(&mut self) -> Result<(), EpochRotationError> {
        match self.state {
            PendingSetState::Activated => return Err(EpochRotationError::AlreadyActivated),
            PendingSetState::CancelledBySubmitter
            | PendingSetState::Superseded
            | PendingSetState::Expired => return Err(EpochRotationError::AlreadyCancelled),
            PendingSetState::Pending => {}
        }
        self.state = PendingSetState::CancelledBySubmitter;
        Ok(())
    }

    /// Marks this pending set as `Superseded` by a newer proposal.
    ///
    /// Called by the rotation manager when a new pending set is queued and
    /// this one is still pending. Only a `Pending` set can be superseded.
    pub fn mark_superseded(&mut self) -> Result<(), EpochRotationError> {
        match self.state {
            PendingSetState::Activated => return Err(EpochRotationError::AlreadyActivated),
            PendingSetState::CancelledBySubmitter
            | PendingSetState::Superseded
            | PendingSetState::Expired => return Err(EpochRotationError::AlreadyCancelled),
            PendingSetState::Pending => {}
        }
        self.state = PendingSetState::Superseded;
        Ok(())
    }

    /// Marks this pending set as `Expired` when the pending window has elapsed
    /// without promotion.
    ///
    /// A set is considered expired when the current epoch is more than
    /// [`MAX_PENDING_EPOCHS`] past the activation epoch. Only a `Pending` set
    /// can be expired; an already-cancelled or activated set is left as-is.
    pub fn mark_expired_if_stale(
        &mut self,
        current_ledger: u32,
        config: &EpochConfig,
    ) -> bool {
        if self.state != PendingSetState::Pending {
            return false;
        }
        let current_epoch = config.epoch_of(current_ledger);
        if current_epoch.saturating_sub(self.activation_epoch) > MAX_PENDING_EPOCHS {
            self.state = PendingSetState::Expired;
            return true;
        }
        false
    }

    /// Returns `true` when this pending set is in a terminal (non-pending) state.
    pub fn is_terminal(&self) -> bool {
        !matches!(self.state, PendingSetState::Pending)
    }
}

/// Manages the queue of pending validator-set rotations for a bridge.
///
/// At most one `Pending` set exists at any time. Queueing a new set while one
/// is already pending marks the existing one as `Superseded` automatically,
/// giving it an explicit, auditable cancellation reason. Activation promotes
/// the pending set to the active slot and records the activated epoch for
/// future monotonicity checks.
#[derive(Clone, Debug)]
pub struct EpochRotationManager {
    /// Identifier of the currently active validator set (opaque).
    pub active_set_id: [u8; 32],
    /// Epoch at which the current active set was promoted.
    pub active_since_epoch: u32,
    /// The single in-flight pending set, if any.
    pub pending: Option<PendingValidatorSet>,
    /// Epoch configuration shared across all operations.
    pub config: EpochConfig,
}

impl EpochRotationManager {
    /// Creates a manager with an initial active set that became active at
    /// `active_since_epoch`.
    pub fn new(
        active_set_id: [u8; 32],
        active_since_epoch: u32,
        config: EpochConfig,
    ) -> Self {
        Self {
            active_set_id,
            active_since_epoch,
            pending: None,
            config,
        }
    }

    /// Queues a new pending validator set for activation at `activation_epoch`.
    ///
    /// If a pending set already exists it is marked `Superseded` before the
    /// new one is registered, so there is always an explicit cancellation trail.
    ///
    /// Returns [`EpochRotationError::NonMonotonicEpoch`] when the requested
    /// activation epoch is not strictly after the current epoch, or
    /// [`EpochRotationError::EpochTooFar`] when it exceeds the pending window.
    pub fn queue(
        &mut self,
        set_id: [u8; 32],
        activation_epoch: u32,
        current_ledger: u32,
    ) -> Result<(), EpochRotationError> {
        // Supersede any existing pending set before registering the new one.
        if let Some(ref mut existing) = self.pending {
            // Best-effort: ignore error if already terminal.
            let _ = existing.mark_superseded();
        }
        let new_pending =
            PendingValidatorSet::new(set_id, activation_epoch, current_ledger, &self.config)?;
        self.pending = Some(new_pending);
        Ok(())
    }

    /// Promotes the pending set to active if the activation epoch has arrived.
    ///
    /// On success the pending slot is cleared and the active set is updated to
    /// the promoted set's `set_id`. Returns the epoch at which the set was
    /// activated.
    ///
    /// Returns [`EpochRotationError::NoPendingSet`] when no pending set exists,
    /// or [`EpochRotationError::EpochNotReached`] when the activation epoch
    /// boundary has not yet arrived.
    pub fn try_activate(&mut self, current_ledger: u32) -> Result<u32, EpochRotationError> {
        let pending = self.pending.as_mut().ok_or(EpochRotationError::NoPendingSet)?;
        pending.try_activate(current_ledger, &self.config)?;
        // Promote.
        let activated_epoch = pending.activation_epoch;
        self.active_set_id = pending.set_id;
        self.active_since_epoch = activated_epoch;
        self.pending = None;
        Ok(activated_epoch)
    }

    /// Cancels the pending set at the submitter's request.
    ///
    /// Returns [`EpochRotationError::NoPendingSet`] when no pending set exists.
    pub fn cancel_pending(&mut self) -> Result<(), EpochRotationError> {
        let pending = self.pending.as_mut().ok_or(EpochRotationError::NoPendingSet)?;
        pending.cancel_by_submitter()?;
        // Move the terminal pending set out so callers can inspect it if needed.
        // Here we simply clear it; callers that need an audit log must persist
        // it before calling this method.
        self.pending = None;
        Ok(())
    }

    /// Expires the pending set if it has been pending past the allowed window.
    ///
    /// Returns `true` when a set was expired, `false` otherwise (including
    /// when no pending set exists).
    pub fn expire_stale_pending(&mut self, current_ledger: u32) -> bool {
        if let Some(ref mut pending) = self.pending {
            if pending.mark_expired_if_stale(current_ledger, &self.config) {
                self.pending = None;
                return true;
            }
        }
        false
    }

    /// Returns a reference to the pending set, if any.
    pub fn pending_set(&self) -> Option<&PendingValidatorSet> {
        self.pending.as_ref()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> EpochConfig {
        EpochConfig::new(DEFAULT_EPOCH_LENGTH).unwrap()
    }

    fn set_id(n: u8) -> [u8; 32] {
        [n; 32]
    }

    // ── EpochConfig ──────────────────────────────────────────────────────────

    #[test]
    fn zero_epoch_length_is_rejected() {
        assert_eq!(EpochConfig::new(0), Err(EpochRotationError::InvalidEpochLength));
    }

    #[test]
    fn epoch_of_first_ledger_in_epoch_is_correct() {
        let cfg = config();
        // Ledger 0 is epoch 0.
        assert_eq!(cfg.epoch_of(0), 0);
        // First ledger of epoch 1 is exactly epoch_length.
        assert_eq!(cfg.epoch_of(DEFAULT_EPOCH_LENGTH), 1);
        // Last ledger of epoch 0.
        assert_eq!(cfg.epoch_of(DEFAULT_EPOCH_LENGTH - 1), 0);
    }

    #[test]
    fn epoch_boundary_returns_first_ledger_of_epoch() {
        let cfg = config();
        assert_eq!(cfg.epoch_boundary(0), 0);
        assert_eq!(cfg.epoch_boundary(1), DEFAULT_EPOCH_LENGTH);
        assert_eq!(cfg.epoch_boundary(2), 2 * DEFAULT_EPOCH_LENGTH);
    }

    #[test]
    fn epoch_reached_is_false_before_boundary_and_true_at_boundary() {
        let cfg = config();
        // One ledger before the epoch-2 boundary: not reached.
        assert!(!cfg.epoch_reached(2, 2 * DEFAULT_EPOCH_LENGTH - 1));
        // Exactly at the boundary: reached.
        assert!(cfg.epoch_reached(2, 2 * DEFAULT_EPOCH_LENGTH));
        // One ledger past the boundary: still reached.
        assert!(cfg.epoch_reached(2, 2 * DEFAULT_EPOCH_LENGTH + 1));
    }

    // ── PendingValidatorSet construction ─────────────────────────────────────

    #[test]
    fn pending_set_with_next_epoch_is_accepted() {
        let cfg = config();
        // current_ledger sits in epoch 0; activate at epoch 1.
        let result = PendingValidatorSet::new(set_id(1), 1, 0, &cfg);
        assert!(result.is_ok());
        let p = result.unwrap();
        assert_eq!(p.activation_epoch, 1);
        assert_eq!(p.state, PendingSetState::Pending);
    }

    #[test]
    fn pending_set_with_current_epoch_is_rejected() {
        let cfg = config();
        // current_ledger is in epoch 1; activation_epoch 1 is not strictly after.
        let current_ledger = DEFAULT_EPOCH_LENGTH; // epoch 1
        let result = PendingValidatorSet::new(set_id(1), 1, current_ledger, &cfg);
        assert_eq!(result, Err(EpochRotationError::NonMonotonicEpoch));
    }

    #[test]
    fn pending_set_with_past_epoch_is_rejected() {
        let cfg = config();
        // current_ledger is in epoch 3; activation_epoch 2 is in the past.
        let current_ledger = 3 * DEFAULT_EPOCH_LENGTH;
        let result = PendingValidatorSet::new(set_id(1), 2, current_ledger, &cfg);
        assert_eq!(result, Err(EpochRotationError::NonMonotonicEpoch));
    }

    #[test]
    fn pending_set_too_far_in_future_is_rejected() {
        let cfg = config();
        // current_ledger is in epoch 0; activation_epoch MAX_PENDING_EPOCHS+1 too far.
        let far_epoch = MAX_PENDING_EPOCHS + 2;
        let result = PendingValidatorSet::new(set_id(1), far_epoch, 0, &cfg);
        assert_eq!(result, Err(EpochRotationError::EpochTooFar));
    }

    #[test]
    fn pending_set_at_max_pending_epochs_is_accepted() {
        let cfg = config();
        // MAX_PENDING_EPOCHS ahead of epoch 0 is exactly the limit.
        let result = PendingValidatorSet::new(set_id(1), MAX_PENDING_EPOCHS, 0, &cfg);
        assert!(result.is_ok());
    }

    // ── Activation at epoch boundary ─────────────────────────────────────────

    #[test]
    fn activation_before_epoch_boundary_is_rejected() {
        let cfg = config();
        // Activate at epoch 2; current_ledger is one ledger before the boundary.
        let mut p = PendingValidatorSet::new(set_id(1), 2, 0, &cfg).unwrap();
        let one_before = 2 * DEFAULT_EPOCH_LENGTH - 1;
        assert_eq!(p.try_activate(one_before, &cfg), Err(EpochRotationError::EpochNotReached));
        assert_eq!(p.state, PendingSetState::Pending);
    }

    #[test]
    fn activation_exactly_at_epoch_boundary_succeeds() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 2, 0, &cfg).unwrap();
        let boundary = 2 * DEFAULT_EPOCH_LENGTH;
        assert_eq!(p.try_activate(boundary, &cfg), Ok(()));
        assert_eq!(p.state, PendingSetState::Activated);
    }

    #[test]
    fn activation_after_epoch_boundary_succeeds() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 2, 0, &cfg).unwrap();
        let after_boundary = 2 * DEFAULT_EPOCH_LENGTH + 500;
        assert_eq!(p.try_activate(after_boundary, &cfg), Ok(()));
        assert_eq!(p.state, PendingSetState::Activated);
    }

    #[test]
    fn activating_already_activated_set_is_rejected() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 1, 0, &cfg).unwrap();
        p.try_activate(DEFAULT_EPOCH_LENGTH, &cfg).unwrap();
        assert_eq!(p.try_activate(DEFAULT_EPOCH_LENGTH, &cfg), Err(EpochRotationError::AlreadyActivated));
    }

    // ── Cancellation ─────────────────────────────────────────────────────────

    #[test]
    fn cancel_by_submitter_moves_set_to_cancelled_state() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 1, 0, &cfg).unwrap();
        assert_eq!(p.cancel_by_submitter(), Ok(()));
        assert_eq!(p.state, PendingSetState::CancelledBySubmitter);
    }

    #[test]
    fn activating_cancelled_set_is_rejected() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 1, 0, &cfg).unwrap();
        p.cancel_by_submitter().unwrap();
        assert_eq!(
            p.try_activate(DEFAULT_EPOCH_LENGTH, &cfg),
            Err(EpochRotationError::AlreadyCancelled)
        );
    }

    #[test]
    fn cancelling_activated_set_is_rejected() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 1, 0, &cfg).unwrap();
        p.try_activate(DEFAULT_EPOCH_LENGTH, &cfg).unwrap();
        assert_eq!(p.cancel_by_submitter(), Err(EpochRotationError::AlreadyActivated));
    }

    #[test]
    fn superseded_set_cannot_be_activated() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 1, 0, &cfg).unwrap();
        p.mark_superseded().unwrap();
        assert_eq!(
            p.try_activate(DEFAULT_EPOCH_LENGTH, &cfg),
            Err(EpochRotationError::AlreadyCancelled)
        );
    }

    // ── Expiry ───────────────────────────────────────────────────────────────

    #[test]
    fn stale_pending_set_is_marked_expired() {
        let cfg = config();
        // Activation epoch 1; skip forward MAX_PENDING_EPOCHS+1 past that.
        let mut p = PendingValidatorSet::new(set_id(1), 1, 0, &cfg).unwrap();
        let stale_ledger = (1 + MAX_PENDING_EPOCHS + 1) * DEFAULT_EPOCH_LENGTH;
        assert!(p.mark_expired_if_stale(stale_ledger, &cfg));
        assert_eq!(p.state, PendingSetState::Expired);
    }

    #[test]
    fn non_stale_pending_set_is_not_expired() {
        let cfg = config();
        let mut p = PendingValidatorSet::new(set_id(1), 2, 0, &cfg).unwrap();
        // Current epoch 3 is only 1 epoch past activation_epoch 2: not stale.
        let not_stale = 3 * DEFAULT_EPOCH_LENGTH;
        assert!(!p.mark_expired_if_stale(not_stale, &cfg));
        assert_eq!(p.state, PendingSetState::Pending);
    }

    // ── EpochRotationManager ─────────────────────────────────────────────────

    #[test]
    fn queue_then_activate_at_boundary_promotes_set() {
        let cfg = config();
        let mut mgr = EpochRotationManager::new(set_id(0), 0, cfg);
        // Queue for epoch 1.
        mgr.queue(set_id(1), 1, 0).unwrap();
        // Cannot activate before the boundary.
        assert_eq!(
            mgr.try_activate(DEFAULT_EPOCH_LENGTH - 1),
            Err(EpochRotationError::EpochNotReached)
        );
        // Activate exactly at the boundary.
        let activated = mgr.try_activate(DEFAULT_EPOCH_LENGTH).unwrap();
        assert_eq!(activated, 1);
        assert_eq!(mgr.active_set_id, set_id(1));
        assert_eq!(mgr.active_since_epoch, 1);
        assert!(mgr.pending.is_none());
    }

    #[test]
    fn queuing_new_set_supersedes_existing_pending_set() {
        let cfg = config();
        let mut mgr = EpochRotationManager::new(set_id(0), 0, cfg);
        // Queue first pending set for epoch 1.
        mgr.queue(set_id(1), 1, 0).unwrap();
        // Queue second pending set for epoch 2 (supersedes the first).
        mgr.queue(set_id(2), 2, 0).unwrap();
        // The pending slot now holds set_id(2).
        assert_eq!(mgr.pending_set().unwrap().set_id, set_id(2));
    }

    #[test]
    fn cancel_pending_removes_pending_slot() {
        let cfg = config();
        let mut mgr = EpochRotationManager::new(set_id(0), 0, cfg);
        mgr.queue(set_id(1), 1, 0).unwrap();
        mgr.cancel_pending().unwrap();
        assert!(mgr.pending_set().is_none());
    }

    #[test]
    fn cancel_pending_when_no_set_queued_returns_error() {
        let cfg = config();
        let mut mgr = EpochRotationManager::new(set_id(0), 0, cfg);
        assert_eq!(mgr.cancel_pending(), Err(EpochRotationError::NoPendingSet));
    }

    #[test]
    fn activate_when_no_pending_set_returns_error() {
        let cfg = config();
        let mut mgr = EpochRotationManager::new(set_id(0), 0, cfg);
        assert_eq!(
            mgr.try_activate(DEFAULT_EPOCH_LENGTH),
            Err(EpochRotationError::NoPendingSet)
        );
    }

    #[test]
    fn expire_stale_clears_pending_slot() {
        let cfg = config();
        let mut mgr = EpochRotationManager::new(set_id(0), 0, cfg);
        mgr.queue(set_id(1), 1, 0).unwrap();
        // Skip far enough to make epoch 1 stale.
        let stale = (1 + MAX_PENDING_EPOCHS + 2) * DEFAULT_EPOCH_LENGTH;
        assert!(mgr.expire_stale_pending(stale));
        assert!(mgr.pending_set().is_none());
    }

    #[test]
    fn expire_stale_does_not_affect_non_stale_pending_set() {
        let cfg = config();
        let mut mgr = EpochRotationManager::new(set_id(0), 0, cfg);
        mgr.queue(set_id(1), 2, 0).unwrap();
        // Only 1 epoch past activation: not stale.
        assert!(!mgr.expire_stale_pending(3 * DEFAULT_EPOCH_LENGTH));
        assert!(mgr.pending_set().is_some());
    }
}
