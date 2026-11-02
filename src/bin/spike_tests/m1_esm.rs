//! Unit tests for the M1.1 spike's evaluation check (the subprocess runs live in tests/spikes.rs).

use super::*;
use v8::{ModuleStatus, PromiseState};

#[test]
fn evaluation_passes_unless_failed_errored_or_rejected() {
    assert_eq!(
        check_evaluation(true, ModuleStatus::Evaluated, None),
        Ok(())
    );
    assert_eq!(
        check_evaluation(true, ModuleStatus::Evaluated, Some(PromiseState::Fulfilled)),
        Ok(())
    );
    assert_eq!(
        check_evaluation(true, ModuleStatus::Evaluated, Some(PromiseState::Pending)),
        Ok(()),
        "a pending top-level await is not a failure"
    );
    assert_eq!(
        check_evaluation(true, ModuleStatus::Evaluated, Some(PromiseState::Rejected)),
        Err("module promise rejected".to_string())
    );
}

#[test]
fn evaluation_failure_reports_the_status_first() {
    assert_eq!(
        check_evaluation(false, ModuleStatus::Evaluated, None),
        Err("evaluate failed: status=Evaluated".to_string())
    );
    assert_eq!(
        check_evaluation(true, ModuleStatus::Errored, Some(PromiseState::Rejected)),
        Err("evaluate failed: status=Errored".to_string())
    );
}
