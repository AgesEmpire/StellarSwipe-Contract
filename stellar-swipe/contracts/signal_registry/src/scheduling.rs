use crate::errors::AdminError;
use crate::types::{
    RecurrencePattern, ScheduleStatus, ScheduledSignal, SignalDataV2, VersionedSignalData,
};
use soroban_sdk::{contracttype, Address, Env, Vec};

#[contracttype]
pub enum ScheduleDataKey {
    Schedule(u64),
    ProviderSchedules(Address),
    NextScheduleId,
}

/// Schedule a new signal for future publication. The `signal_data` parameter
/// uses the current V2 shape; it is stored as `VersionedSignalData::V2` so
/// that older V1 records (stored before Issue #568) coexist transparently
/// in persistent storage.
pub fn schedule_signal(
    env: Env,
    provider: Address,
    signal_data: SignalDataV2,
    publish_at: u64,
    recurrence: RecurrencePattern,
) -> Result<u64, AdminError> {
    provider.require_auth();

    let current_time = env.ledger().timestamp();
    if publish_at <= current_time {
        return Err(AdminError::InvalidTimestamp);
    }
    if publish_at > current_time + 2_592_000 {
        return Err(AdminError::ScheduleTooFarFuture);
    }

    let mut provider_schedules: Vec<u64> = env
        .storage()
        .persistent()
        .get(&ScheduleDataKey::ProviderSchedules(provider.clone()))
        .unwrap_or(Vec::new(&env));

    if provider_schedules.len() >= 50 {
        return Err(AdminError::ScheduleLimitReached);
    }

    let schedule_id: u64 = env
        .storage()
        .instance()
        .get(&ScheduleDataKey::NextScheduleId)
        .unwrap_or(0);

    // Always write new records as V2 (Issue #568).
    let scheduled = ScheduledSignal {
        id: schedule_id,
        provider: provider.clone(),
        signal_data: VersionedSignalData::V2(signal_data),
        publish_at,
        recurrence,
        status: ScheduleStatus::Pending,
    };

    env.storage()
        .persistent()
        .set(&ScheduleDataKey::Schedule(schedule_id), &scheduled);
    provider_schedules.push_back(schedule_id);
    env.storage().persistent().set(
        &ScheduleDataKey::ProviderSchedules(provider),
        &provider_schedules,
    );
    env.storage()
        .instance()
        .set(&ScheduleDataKey::NextScheduleId, &(schedule_id + 1));

    Ok(schedule_id)
}

/// Publish all pending schedules whose `publish_at` has elapsed.
/// Reads handle both V1 and V2 signal-data variants transparently; recurring
/// next-occurrences are re-written as V2 so the data migrates forward
/// on first publish without a separate migration pass (Issue #568).
pub fn publish_scheduled_signals(env: Env) -> Vec<u64> {
    let mut published_ids = Vec::new(&env);
    let current_time = env.ledger().timestamp();
    let max_id: u64 = env
        .storage()
        .instance()
        .get(&ScheduleDataKey::NextScheduleId)
        .unwrap_or(0);

    for i in 0..max_id {
        if let Some(mut scheduled) = env
            .storage()
            .persistent()
            .get::<_, ScheduledSignal>(&ScheduleDataKey::Schedule(i))
        {
            if scheduled.status == ScheduleStatus::Pending && current_time >= scheduled.publish_at {
                scheduled.status = ScheduleStatus::Published;
                published_ids.push_back(scheduled.id);

                if scheduled.recurrence.is_recurring && scheduled.recurrence.repeat_count > 0 {
                    // Upgrade V1 → V2 on first publish of a recurring schedule.
                    let resolved = scheduled.signal_data.clone().resolve();
                    schedule_next_occurrence(
                        &env,
                        &scheduled,
                        scheduled.recurrence.clone(),
                        resolved,
                    );
                }

                env.storage()
                    .persistent()
                    .set(&ScheduleDataKey::Schedule(i), &scheduled);
            }
        }
    }
    published_ids
}

/// Clone the recurring schedule forward, re-writing signal data as V2 so any
/// V1 data is upgraded on the fly (Issue #568).
fn schedule_next_occurrence(
    env: &Env,
    current: &ScheduledSignal,
    mut pattern: RecurrencePattern,
    resolved_data: SignalDataV2,
) {
    let next_id: u64 = env
        .storage()
        .instance()
        .get(&ScheduleDataKey::NextScheduleId)
        .unwrap_or(0);

    pattern.repeat_count = pattern.repeat_count.saturating_sub(1);

    let next_scheduled = ScheduledSignal {
        id: next_id,
        provider: current.provider.clone(),
        // Forward-write as V2 so the next occurrence is always current shape.
        signal_data: VersionedSignalData::V2(resolved_data),
        publish_at: current.publish_at + pattern.interval_seconds,
        recurrence: pattern,
        status: ScheduleStatus::Pending,
    };

    env.storage()
        .persistent()
        .set(&ScheduleDataKey::Schedule(next_id), &next_scheduled);
    env.storage()
        .instance()
        .set(&ScheduleDataKey::NextScheduleId, &(next_id + 1));
}

pub fn cancel_scheduled_signal(
    env: Env,
    provider: Address,
    schedule_id: u64,
) -> Result<(), AdminError> {
    provider.require_auth();
    let mut scheduled: ScheduledSignal = env
        .storage()
        .persistent()
        .get(&ScheduleDataKey::Schedule(schedule_id))
        .ok_or(AdminError::ScheduleNotFound)?;

    if scheduled.provider != provider {
        return Err(AdminError::NotScheduleOwner);
    }
    scheduled.status = ScheduleStatus::Cancelled;
    env.storage()
        .persistent()
        .set(&ScheduleDataKey::Schedule(schedule_id), &scheduled);
    Ok(())
}

/// Read a stored scheduled signal and return the resolved V2 signal data
/// alongside the full record. Transparently upgrades any V1 record on read
/// (Issue #568). Returns `None` if no record exists for `schedule_id`.
pub fn get_scheduled_signal_data(
    env: &Env,
    schedule_id: u64,
) -> Option<(ScheduledSignal, SignalDataV2)> {
    let scheduled: ScheduledSignal = env
        .storage()
        .persistent()
        .get(&ScheduleDataKey::Schedule(schedule_id))?;
    let resolved = scheduled.signal_data.clone().resolve();
    Some((scheduled, resolved))
}
