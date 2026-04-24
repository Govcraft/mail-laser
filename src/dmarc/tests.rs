use super::*;

// Open-policy fixture that reaches every arm of `decide`.
const AUTHED_FROM: &str = "alice@agency.gov";

fn pass() -> DmarcOutcome {
    DmarcOutcome::Pass {
        authenticated_from: AUTHED_FROM.to_string(),
    }
}

#[test]
fn decide_off_accepts_regardless_of_outcome() {
    for outcome in [
        pass(),
        DmarcOutcome::Fail,
        DmarcOutcome::TempError,
        DmarcOutcome::NoPolicy,
    ] {
        let decision = decide(&outcome, DmarcMode::Off, DmarcTempErrorAction::Reject);
        assert!(
            matches!(
                decision,
                DmarcDecision::Accept {
                    dmarc_result: "off",
                    authenticated_from: None
                }
            ),
            "Off should accept with no stamping; outcome={:?} decision={:?}",
            outcome,
            decision
        );
    }
}

#[test]
fn decide_monitor_accepts_and_stamps_every_outcome() {
    let cases = [
        (pass(), "pass", Some(AUTHED_FROM.to_string())),
        (DmarcOutcome::Fail, "fail", None),
        (DmarcOutcome::TempError, "temperror", None),
        (DmarcOutcome::NoPolicy, "none", None),
    ];

    for (outcome, expected_result, expected_from) in cases {
        let decision = decide(&outcome, DmarcMode::Monitor, DmarcTempErrorAction::Reject);
        match decision {
            DmarcDecision::Accept {
                dmarc_result,
                authenticated_from,
            } => {
                assert_eq!(dmarc_result, expected_result);
                assert_eq!(authenticated_from, expected_from);
            }
            other => panic!("monitor must always Accept, got {:?}", other),
        }
    }
}

#[test]
fn decide_enforce_passes_accept_with_authenticated_from() {
    let decision = decide(&pass(), DmarcMode::Enforce, DmarcTempErrorAction::Reject);
    assert_eq!(
        decision,
        DmarcDecision::Accept {
            dmarc_result: "pass",
            authenticated_from: Some(AUTHED_FROM.to_string()),
        }
    );
}

#[test]
fn decide_enforce_no_policy_accepts_without_from() {
    let decision = decide(
        &DmarcOutcome::NoPolicy,
        DmarcMode::Enforce,
        DmarcTempErrorAction::Reject,
    );
    assert_eq!(
        decision,
        DmarcDecision::Accept {
            dmarc_result: "none",
            authenticated_from: None,
        }
    );
}

#[test]
fn decide_enforce_fail_rejects_550() {
    let decision = decide(
        &DmarcOutcome::Fail,
        DmarcMode::Enforce,
        DmarcTempErrorAction::Reject,
    );
    assert_eq!(
        decision,
        DmarcDecision::Reject {
            code: 550,
            status: "5.7.1 DMARC policy violation",
        }
    );
}

#[test]
fn decide_enforce_temperror_reject_returns_451() {
    let decision = decide(
        &DmarcOutcome::TempError,
        DmarcMode::Enforce,
        DmarcTempErrorAction::Reject,
    );
    assert_eq!(
        decision,
        DmarcDecision::Reject {
            code: 451,
            status: "4.7.0 DMARC temporary error",
        }
    );
}

#[test]
fn decide_enforce_temperror_accept_forwards_with_stamp() {
    let decision = decide(
        &DmarcOutcome::TempError,
        DmarcMode::Enforce,
        DmarcTempErrorAction::Accept,
    );
    assert_eq!(
        decision,
        DmarcDecision::Accept {
            dmarc_result: "temperror",
            authenticated_from: None,
        }
    );
}

#[test]
fn as_payload_str_covers_every_variant() {
    assert_eq!(
        DmarcOutcome::Pass {
            authenticated_from: "x@y.com".to_string()
        }
        .as_payload_str(),
        "pass"
    );
    assert_eq!(DmarcOutcome::Fail.as_payload_str(), "fail");
    assert_eq!(DmarcOutcome::TempError.as_payload_str(), "temperror");
    assert_eq!(DmarcOutcome::NoPolicy.as_payload_str(), "none");
}

#[test]
fn organizational_domain_strips_subdomain() {
    assert_eq!(organizational_domain("mail.agency.gov"), "agency.gov");
    assert_eq!(organizational_domain("agency.gov"), "agency.gov");
    // psl handles uk's nested TLDs correctly
    assert_eq!(organizational_domain("mail.example.co.uk"), "example.co.uk");
}
