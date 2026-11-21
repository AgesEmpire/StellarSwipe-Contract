use soroban_sdk::{contracttype, Address, Env, String, Symbol};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OnboardingStatus {
    NotStarted = 0,
    InProgress = 1,
    Completed = 2,
}

pub fn emit_onboarding_status_updated(
    env: &Env,
    user: Address,
    status: OnboardingStatus,
    milestone: Option<String>,
) {
    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "UserOnboardingStatusUpdated"),),
        (user, status, milestone),
    );
}
