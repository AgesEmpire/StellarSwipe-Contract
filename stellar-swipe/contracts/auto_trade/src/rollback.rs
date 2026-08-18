//! Atomic rollback semantics for multi-step contract workflows.
//!
//! Soroban panics automatically roll back all storage writes for the current
//! invocation.  This module provides a lightweight **workflow execution
//! harness** that:
//!
//! 1. Runs each step inside an explicit *check* phase before committing state.
//! 2. Returns a typed `WorkflowResult` so callers know which step failed.
//! 3. Documents the rollback guarantee so consumers can rely on it.
//!
//! ## Rollback guarantee
//!
//! In Soroban, any `panic!` (including `require` failures and integer
//! overflow) discards **all** storage mutations made during the current
//! invocation.  This is enforced by the host environment and cannot be
//! bypassed by contract code.
//!
//! The helpers in this module surface that guarantee explicitly:
//!
//! - `run_workflow` executes a sequence of `WorkflowStep`s one by one.
//! - The first step that returns `Err(...)` causes the function to return
//!   `WorkflowResult::Failed` **without** executing subsequent steps.
//! - Because no `panic!` is issued, callers can observe *which* step failed.
//!   If the caller then wishes to abort the entire transaction it should
//!   `panic!` after inspecting the result.
//!
//! ## Usage
//!
//! ```ignore
//! let steps: Vec<WorkflowStep> = Vec::new(&env);
//! // push step descriptors …
//! let result = rollback::run_workflow(&env, &steps, my_executor);
//! if let WorkflowResult::Failed { step_index, .. } = result {
//!     panic!("workflow step {} failed – transaction rolled back", step_index);
//! }
//! ```

#![allow(dead_code)]

use soroban_sdk::{contracttype, Env, String, Vec};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Identifies a workflow step by name and sequential index.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowStep {
    /// Zero-based position within the workflow.
    pub index: u32,
    /// Human-readable step name (stored on-chain for auditability).
    pub name: String,
}

/// Outcome of a complete workflow execution.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowResult {
    /// All steps completed successfully.
    Success,
    /// A step returned an error; `step_index` identifies which one.
    Failed { step_index: u32, step_name: String },
}

// ── Core harness ──────────────────────────────────────────────────────────────

/// Execute `steps` in order using `executor`.
///
/// `executor` receives the `Env` reference and a reference to the current
/// `WorkflowStep`.  It should return `Ok(())` on success or `Err(())` to
/// signal that the step failed.
///
/// The harness **stops at the first failure** and returns
/// `WorkflowResult::Failed`.  It never panics itself; panicking is left to
/// the caller so that partial state is only rolled back when the caller
/// explicitly chooses to abort.
pub fn run_workflow<F>(env: &Env, steps: &Vec<WorkflowStep>, mut executor: F) -> WorkflowResult
where
    F: FnMut(&Env, &WorkflowStep) -> Result<(), ()>,
{
    for i in 0..steps.len() {
        let step = steps.get_unchecked(i);
        if executor(env, &step).is_err() {
            return WorkflowResult::Failed {
                step_index: step.index,
                step_name: step.name.clone(),
            };
        }
    }
    WorkflowResult::Success
}

/// Convenience wrapper: runs the workflow and **panics** on failure, ensuring
/// Soroban rolls back all mutations made during this invocation.
pub fn run_workflow_or_panic<F>(env: &Env, steps: &Vec<WorkflowStep>, executor: F)
where
    F: FnMut(&Env, &WorkflowStep) -> Result<(), ()>,
{
    match run_workflow(env, steps, executor) {
        WorkflowResult::Success => {}
        WorkflowResult::Failed { .. } => {
            panic!("workflow failed – rolling back");
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn env() -> Env {
        Env::default()
    }

    fn step(env: &Env, idx: u32, name: &str) -> WorkflowStep {
        WorkflowStep {
            index: idx,
            name: soroban_sdk::String::from_str(env, name),
        }
    }

    fn make_steps(env: &Env, names: &[&str]) -> Vec<WorkflowStep> {
        let mut v: Vec<WorkflowStep> = Vec::new(env);
        for (i, n) in names.iter().enumerate() {
            v.push_back(step(env, i as u32, n));
        }
        v
    }

    // All steps succeed → Success.
    #[test]
    fn all_steps_succeed_returns_success() {
        let e = env();
        let steps = make_steps(&e, &["init", "validate", "settle"]);
        let result = run_workflow(&e, &steps, |_, _| Ok(()));
        assert_eq!(result, WorkflowResult::Success);
    }

    // First step fails → Failed at index 0.
    #[test]
    fn first_step_failure_returns_failed_index_0() {
        let e = env();
        let steps = make_steps(&e, &["init", "validate", "settle"]);
        let result = run_workflow(
            &e,
            &steps,
            |_, step| {
                if step.index == 0 {
                    Err(())
                } else {
                    Ok(())
                }
            },
        );
        match result {
            WorkflowResult::Failed { step_index, .. } => assert_eq!(step_index, 0),
            _ => panic!("expected Failed"),
        }
    }

    // Middle step fails → Failed at that index; subsequent steps not executed.
    #[test]
    fn middle_step_failure_stops_execution() {
        let e = env();
        let steps = make_steps(&e, &["init", "validate", "settle"]);
        let mut executed = soroban_sdk::Vec::<u32>::new(&e);
        let result = run_workflow(&e, &steps, |_, step| {
            executed.push_back(step.index);
            if step.index == 1 {
                Err(())
            } else {
                Ok(())
            }
        });
        // Steps 0 and 1 ran; step 2 must not have executed.
        assert_eq!(executed.len(), 2);
        assert_eq!(executed.get_unchecked(0), 0);
        assert_eq!(executed.get_unchecked(1), 1);
        match result {
            WorkflowResult::Failed { step_index, .. } => assert_eq!(step_index, 1),
            _ => panic!("expected Failed"),
        }
    }

    // Last step fails → Failed at that index.
    #[test]
    fn last_step_failure_returns_correct_index() {
        let e = env();
        let steps = make_steps(&e, &["a", "b", "c"]);
        let result = run_workflow(
            &e,
            &steps,
            |_, step| {
                if step.index == 2 {
                    Err(())
                } else {
                    Ok(())
                }
            },
        );
        match result {
            WorkflowResult::Failed { step_index, .. } => assert_eq!(step_index, 2),
            _ => panic!("expected Failed"),
        }
    }

    // Empty workflow succeeds immediately.
    #[test]
    fn empty_workflow_returns_success() {
        let e = env();
        let steps: Vec<WorkflowStep> = Vec::new(&e);
        let result = run_workflow(&e, &steps, |_, _| Ok(()));
        assert_eq!(result, WorkflowResult::Success);
    }

    // run_workflow_or_panic succeeds without panicking.
    #[test]
    fn workflow_or_panic_succeeds_silently() {
        let e = env();
        let steps = make_steps(&e, &["step1"]);
        run_workflow_or_panic(&e, &steps, |_, _| Ok(()));
    }

    // run_workflow_or_panic panics on failure.
    #[test]
    #[should_panic(expected = "workflow failed – rolling back")]
    fn workflow_or_panic_panics_on_failure() {
        let e = env();
        let steps = make_steps(&e, &["step1"]);
        run_workflow_or_panic(&e, &steps, |_, _| Err(()));
    }

    // Failure step name is preserved in the result.
    #[test]
    fn failure_step_name_preserved() {
        let e = env();
        let steps = make_steps(&e, &["init", "bridge_transfer", "finalize"]);
        let result = run_workflow(
            &e,
            &steps,
            |_, step| {
                if step.index == 1 {
                    Err(())
                } else {
                    Ok(())
                }
            },
        );
        match result {
            WorkflowResult::Failed { step_name, .. } => {
                assert_eq!(
                    step_name,
                    soroban_sdk::String::from_str(&e, "bridge_transfer")
                )
            }
            _ => panic!("expected Failed"),
        }
    }
}
