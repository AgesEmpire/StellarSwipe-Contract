# Committee Elections

This document describes the requirements, quorum rules, invalid-vote handling,
and failure modes for governance committee elections in the StellarSwipe
governance contract.

---

## Overview

A committee election replaces the current membership of a governance committee
with addresses chosen by stake-weighted community voting.  Elections are
managed through four on-chain operations:

| Step | Function | Caller |
|------|----------|--------|
| 1 | `start_committee_election` | Admin |
| 2 | `nominate_for_committee` | Any staked address |
| 3 | `vote_in_committee_election` | Any staked address |
| 4 | `finalize_committee_election` | Admin (after deadline) |

---

## Starting an Election

```
start_committee_election(
    admin,
    committee_id,
    positions_available,   // u32  — must be ≥ 3 and ≤ committee.max_members
    duration_days,         // u32  — must be ≥ 1
    min_participation,     // u32  — minimum unique valid voters required
    quorum_stake_threshold // i128 — minimum total staked weight required
)
```

`min_participation` and `quorum_stake_threshold` are independent thresholds.
Set either to `0` to disable that check (not recommended for production).

**Constraints:**
- `positions_available` must be ≥ 3 and ≤ the committee's `max_members`.
- `duration_days` must be ≥ 1.
- `quorum_stake_threshold` must be ≥ 0 (negative values are rejected).
- A new election cannot be started while a previous one is still active
  (i.e. before `election_end`).  A failed election is automatically cleared,
  so a new one can be started immediately after finalization.

**Errors:**
- `InvalidCommitteeConfig` — any constraint above is violated, or an active
  election already exists for the committee.
- `CommitteeNotFound` — `committee_id` does not exist.
- `CommitteeInactive` / `CommitteeTermEnded` — committee cannot run elections.

---

## Nomination

Any address with a positive staked balance may nominate a candidate.
Self-nomination is allowed (nominator and nominee can be the same address).
Duplicate nominations for the same candidate are silently de-duplicated.

**Errors:**
- `CommitteeElectionNotFound` — no active election for this committee.
- `CommitteeElectionNotActive` — nomination window has closed.
- `Unauthorized` — nominator has no staked balance.

---

## Casting a Vote

```
vote_in_committee_election(committee_id, voter, candidate)
```

A ballot is **accepted** when:
1. The election is currently active (`election_start ≤ now < election_end`).
2. `candidate` appears in `election.candidates` (was nominated).
3. `voter` has a staked balance > 0 at the time of the vote.
4. `voter` has not already cast a ballot in this election.

A ballot is **rejected** (returns `GovernanceError::InvalidElectionVote`) when:
- `candidate` is not on the ballot.
- `voter` has zero staked balance.

A duplicate ballot returns `GovernanceError::AlreadyVoted`.

**In all rejection cases the election state is not modified** — no phantom
votes are written, and previously recorded valid votes are preserved intact.

---

## Finalizing an Election

```
finalize_committee_election(admin, committee_id)
```

Can only be called after `election_end`.  Calling before the deadline returns
`CommitteeElectionNotActive`.

Returns an `ElectionResult`:

```rust
pub struct ElectionResult {
    pub status: ElectionStatus,       // outcome variant (see below)
    pub winners: Vec<Address>,        // elected addresses; empty on failure
    pub valid_votes: u32,             // count of accepted ballots
    pub total_stake_weight: i128,     // sum of staked balances behind valid votes
    pub rejected_votes: u32,          // count of dropped invalid ballots
}
```

### ElectionStatus Variants

| Variant | Meaning | Committee updated? |
|---------|---------|-------------------|
| `Succeeded` | All checks passed; winners installed | **Yes** |
| `FailedQuorumParticipation` | `valid_votes < min_participation` | No |
| `FailedQuorumStake` | `total_stake_weight < quorum_stake_threshold` | No |
| `FailedInsufficientWinners` | Fewer than 3 candidates received any votes | No |
| `Pending` | Election not yet finalised (informational only) | No |

On any failure variant:
- The **existing committee membership is unchanged**.
- The failed election record is **removed** from state so a new election can
  be started immediately.
- No error is returned — the caller inspects `result.status`.

---

## Invalid Vote Handling

Invalid ballots operate on a **two-layer** rejection model:

### Layer 1 — Rejected at cast time

These ballots never enter `election.votes`:

| Reason | Error returned |
|--------|---------------|
| Candidate not on ballot | `InvalidElectionVote` |
| Voter has no staked balance | `InvalidElectionVote` |
| Voter already voted | `AlreadyVoted` |
| Election not active (timing) | `CommitteeElectionNotActive` |

### Layer 2 — Excluded at finalization time

Ballots that were valid when cast but became invalid before finalization
(e.g. voter fully unstaked after voting) are silently excluded from tallying
and quorum math.  They are counted in `result.rejected_votes` for auditability.

This two-layer approach guarantees:
- `election.votes` never contains a zero-weight or phantom ballot.
- `election.votes.len()` counts only accepted cast-time ballots.
- `result.valid_votes` counts only ballots that were still valid at finalization.
- `result.rejected_votes` accounts for every ballot that was dropped at
  finalization time.
- The quorum check runs on `valid_votes` / `total_stake_weight`, not on the
  raw ballot count — so unstaking attacks cannot inflate participation numbers.

---

## Quorum Checks

Quorum is evaluated **after** invalid-ballot exclusion, in this order:

1. **Participation quorum** (`min_participation > 0`)  
   If `valid_votes < min_participation` → `FailedQuorumParticipation`.

2. **Stake-weight quorum** (`quorum_stake_threshold > 0`)  
   If `total_stake_weight < quorum_stake_threshold` → `FailedQuorumStake`.

3. **Minimum-winners check**  
   If fewer than 3 distinct candidates received at least one valid vote →
   `FailedInsufficientWinners`.

Setting either threshold to `0` disables that specific check.

---

## Winner Selection

Winners are selected by **stake-weighted plurality**: the top
`positions_available` candidates ranked by accumulated stake weight of their
voters.  Ties are broken by iteration order (first-nominated wins).

The first winner (`winners[0]`) becomes the new committee chair.

---

## Example: Quorum Failure and Recovery

```
// Election 1 — strict quorum, not met
start_committee_election(admin, id, 3, 7, min_participation=10, quorum_stake=0)
// ... only 3 voters participate ...
finalize_committee_election(admin, id)
// → ElectionResult { status: FailedQuorumParticipation, winners: [], valid_votes: 3 }
// Committee is unchanged; election record is cleared.

// Election 2 — relaxed quorum, succeeds
start_committee_election(admin, id, 3, 7, min_participation=1, quorum_stake=0)
// ... 3+ voters participate ...
finalize_committee_election(admin, id)
// → ElectionResult { status: Succeeded, winners: [...], valid_votes: N }
```

## Example: Stake-Weight Quorum Failure

```
// Small-stake voters — total weight below threshold
start_committee_election(admin, id, 3, 7, min_participation=1, quorum_stake=1_000_000)
// ... 3 voters with 1_000 stake each (total = 3_000) ...
finalize_committee_election(admin, id)
// → ElectionResult {
//     status: FailedQuorumStake,
//     valid_votes: 3,
//     total_stake_weight: 3_000,   // returned for diagnostics
//     winners: [],
//   }
```

## Example: Voter Unstakes After Voting

```
// voter_b votes then unstakes before finalization
vote_in_committee_election(id, voter_b, candidate_x)
unstake(voter_b, full_balance)
finalize_committee_election(admin, id)
// → ElectionResult {
//     valid_votes: N-1,    // voter_b excluded
//     rejected_votes: 1,   // voter_b counted here
//     total_stake_weight: weight_without_voter_b,
//     ...
//   }
```

---

## Error Reference

| Error | Trigger |
|-------|---------|
| `CommitteeElectionNotFound` | No election exists for `committee_id` |
| `CommitteeElectionNotActive` | Voting outside `[election_start, election_end)`, or finalizing before deadline |
| `InvalidElectionVote` | Vote for un-nominated candidate, or voter has no staked balance |
| `AlreadyVoted` | Voter has already cast a ballot in this election |
| `InvalidCommitteeConfig` | `positions_available < 3`, `duration_days == 0`, negative `quorum_stake_threshold`, or election already running |
| `ElectionQuorumNotMet` | Reserved for direct call paths that need a hard error; prefer checking `ElectionResult.status` after `finalize_committee_election` |

---

## Test Coverage

The following test scenarios are implemented in
`contracts/governance/src/test_committee_elections.rs`:

| Test | What it verifies |
|------|-----------------|
| `reject_election_with_too_few_positions` | `positions_available < 3` → `InvalidCommitteeConfig` |
| `reject_election_with_zero_duration` | `duration_days == 0` → `InvalidCommitteeConfig` |
| `reject_election_with_negative_quorum_stake_threshold` | Negative threshold → `InvalidCommitteeConfig` |
| `reject_vote_for_unnominated_candidate` | Unknown candidate → `InvalidElectionVote`; `election.votes` unchanged |
| `reject_vote_from_unstaked_voter` | Zero-stake voter → `InvalidElectionVote`; `election.votes` unchanged |
| `reject_duplicate_vote_in_election` | Second ballot → `AlreadyVoted`; first ballot preserved |
| `reject_finalize_before_deadline` | Early finalization → `CommitteeElectionNotActive` |
| `election_fails_when_participation_quorum_not_met` | `valid_votes < min_participation` → `FailedQuorumParticipation`; committee unchanged; new election startable |
| `election_fails_when_stake_quorum_not_met` | `total_stake_weight < threshold` → `FailedQuorumStake`; `total_stake_weight` in result |
| `finalization_excludes_votes_from_voters_who_unstaked` | Post-vote unstake → excluded from `valid_votes`; counted in `rejected_votes` |
| `successful_election_with_quorum_checks_installs_winners` | Both quorum params set and met → `Succeeded`; winners installed; chair updated |
