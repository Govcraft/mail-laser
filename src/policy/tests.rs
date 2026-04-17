use crate::policy::{AttachmentCheck, DmarcContext, PolicyEngine};
use std::net::{IpAddr, Ipv4Addr};

const POLICIES: &str = r#"
    permit(
      principal == User::"alice@agency.gov",
      action == Action::"SendMail",
      resource
    );

    permit(
      principal == User::"bob@agency.gov",
      action == Action::"SendMail",
      resource
    );

    permit(
      principal == User::"alice@agency.gov",
      action == Action::"Attach",
      resource
    ) when {
      [
        "application/pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
      ].contains(context.content_type) &&
      context.size_bytes <= 10485760
    };

    // Bob can only send text; he is not granted Attach.
"#;

fn engine() -> PolicyEngine {
    PolicyEngine::from_strings(POLICIES, None).expect("policies should parse")
}

/// DMARC context with all-off defaults for tests that don't care about
/// authentication facts. Use `dmarc_pass()` when a test explicitly wants a
/// `pass` outcome in context.
fn dmarc_off(envelope_from: &str) -> DmarcContext {
    DmarcContext {
        result: "off",
        aligned: false,
        authenticated_from: None,
        envelope_from: envelope_from.to_string(),
        helo: "test.example".to_string(),
        peer_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    }
}

fn dmarc_pass(envelope_from: &str, authenticated_from: &str) -> DmarcContext {
    DmarcContext {
        result: "pass",
        aligned: true,
        authenticated_from: Some(authenticated_from.to_string()),
        envelope_from: envelope_from.to_string(),
        helo: "test.example".to_string(),
        peer_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    }
}

fn recipient() -> &'static str {
    "dest@agency.gov"
}

#[test]
fn can_send_allows_listed_user() {
    assert!(engine().can_send("alice@agency.gov", recipient(), &dmarc_off("alice@agency.gov")));
}

#[test]
fn can_send_is_case_insensitive() {
    assert!(engine().can_send("Alice@AGENCY.gov", recipient(), &dmarc_off("Alice@AGENCY.gov")));
}

#[test]
fn can_send_denies_unknown_user() {
    assert!(!engine().can_send("mallory@evil.example", recipient(), &dmarc_off("mallory@evil.example")));
}

#[test]
fn can_attach_allows_pdf_within_size() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("brief.pdf"),
        content_type: "application/pdf",
        size_bytes: 2_000_000,
    };
    assert!(e.can_attach("alice@agency.gov", &att, &dmarc_off("alice@agency.gov")));
}

#[test]
fn can_attach_denies_disallowed_content_type() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("payload.exe"),
        content_type: "application/x-msdownload",
        size_bytes: 1_000,
    };
    assert!(!e.can_attach("alice@agency.gov", &att, &dmarc_off("alice@agency.gov")));
}

#[test]
fn can_attach_denies_when_over_size() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("giant.pdf"),
        content_type: "application/pdf",
        size_bytes: 10_485_761,
    };
    assert!(!e.can_attach("alice@agency.gov", &att, &dmarc_off("alice@agency.gov")));
}

#[test]
fn can_attach_denies_unauthorized_principal() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("brief.pdf"),
        content_type: "application/pdf",
        size_bytes: 1_000,
    };
    assert!(!e.can_attach("bob@agency.gov", &att, &dmarc_off("bob@agency.gov")));
}

#[test]
fn default_is_deny_when_no_policy_matches() {
    let e = PolicyEngine::from_strings("", None).expect("empty policy set");
    assert!(!e.can_send("alice@agency.gov", recipient(), &dmarc_off("alice@agency.gov")));
}

#[test]
fn bundled_example_policies_and_entities_load_and_allow_members() {
    use std::path::Path;
    let engine = PolicyEngine::load(
        Path::new("policies/example.cedar"),
        Some(Path::new("policies/entities.json")),
    )
    .expect("bundled policy + entities should load cleanly");

    // Alice is in ApprovedSenders and has a PDF carve-out.
    assert!(engine.can_send(
        "alice@agency.gov",
        recipient(),
        &dmarc_off("alice@agency.gov"),
    ));
    assert!(engine.can_attach(
        "alice@agency.gov",
        &AttachmentCheck {
            filename: Some("brief.pdf"),
            content_type: "application/pdf",
            size_bytes: 1_000_000,
        },
        &dmarc_off("alice@agency.gov"),
    ));

    // Bob can send but can't attach docx (he's PDF-only, under 2 MiB).
    assert!(engine.can_send(
        "bob@agency.gov",
        recipient(),
        &dmarc_off("bob@agency.gov"),
    ));
    assert!(!engine.can_attach(
        "bob@agency.gov",
        &AttachmentCheck {
            filename: Some("doc.docx"),
            content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            size_bytes: 1_000,
        },
        &dmarc_off("bob@agency.gov"),
    ));
    assert!(engine.can_attach(
        "bob@agency.gov",
        &AttachmentCheck {
            filename: Some("small.pdf"),
            content_type: "application/pdf",
            size_bytes: 500_000,
        },
        &dmarc_off("bob@agency.gov"),
    ));

    // Unlisted user is denied.
    assert!(!engine.can_send(
        "eve@evil.example",
        recipient(),
        &dmarc_off("eve@evil.example"),
    ));
}

#[test]
fn entities_file_enables_group_membership() {
    let policies = r#"
        permit(
          principal in Group::"Approved",
          action == Action::"SendMail",
          resource
        );
    "#;
    let entities = r#"
        [
          {
            "uid": { "type": "User", "id": "carol@agency.gov" },
            "parents": [ { "type": "Group", "id": "Approved" } ],
            "attrs": {}
          },
          {
            "uid": { "type": "Group", "id": "Approved" },
            "parents": [],
            "attrs": {}
          }
        ]
    "#;
    let e = PolicyEngine::from_strings(policies, Some(entities)).expect("engine loads");
    assert!(e.can_send("carol@agency.gov", recipient(), &dmarc_off("carol@agency.gov")));
    assert!(!e.can_send("dan@agency.gov", recipient(), &dmarc_off("dan@agency.gov")));
}

// --- DMARC context tests ---

#[test]
fn can_send_allows_when_policy_requires_dmarc_pass_and_outcome_is_pass() {
    let policies = r#"
        permit(principal, action == Action::"SendMail", resource)
          when { context.dmarc_result == "pass" };
    "#;
    let e = PolicyEngine::from_strings(policies, None).expect("policies parse");
    assert!(e.can_send(
        "alice@agency.gov",
        recipient(),
        &dmarc_pass("alice@agency.gov", "alice@agency.gov"),
    ));
}

#[test]
fn can_send_denies_when_policy_requires_dmarc_pass_and_outcome_is_not_pass() {
    let policies = r#"
        permit(principal, action == Action::"SendMail", resource)
          when { context.dmarc_result == "pass" };
    "#;
    let e = PolicyEngine::from_strings(policies, None).expect("policies parse");
    // DMARC off -> no permit matches -> default deny.
    assert!(!e.can_send(
        "alice@agency.gov",
        recipient(),
        &dmarc_off("alice@agency.gov"),
    ));
}

#[test]
fn can_send_can_forbid_unless_dmarc_aligned() {
    let policies = r#"
        permit(principal, action == Action::"SendMail", resource);
        forbid(principal, action == Action::"SendMail", resource)
          unless { context.dmarc_aligned == true };
    "#;
    let e = PolicyEngine::from_strings(policies, None).expect("policies parse");
    assert!(e.can_send(
        "alice@agency.gov",
        recipient(),
        &dmarc_pass("alice@agency.gov", "alice@agency.gov"),
    ));
    assert!(!e.can_send(
        "alice@agency.gov",
        recipient(),
        &dmarc_off("alice@agency.gov"),
    ));
}

#[test]
fn can_send_sees_envelope_from_and_authenticated_from_separately() {
    // Policy trusts the authenticated identity but also requires the envelope
    // to match — blocks envelope spoofing even when DMARC passed on another
    // identity.
    let policies = r#"
        permit(principal, action == Action::"SendMail", resource)
          when {
            context.dmarc_aligned == true &&
            context.envelope_from == context.authenticated_from
          };
    "#;
    let e = PolicyEngine::from_strings(policies, None).expect("policies parse");
    assert!(e.can_send(
        "alice@agency.gov",
        recipient(),
        &dmarc_pass("alice@agency.gov", "alice@agency.gov"),
    ));
    assert!(!e.can_send(
        "alice@agency.gov",
        recipient(),
        &dmarc_pass("alice@agency.gov", "bob@agency.gov"),
    ));
}

#[test]
fn can_send_resource_matches_recipient_literal() {
    let policies = r#"
        permit(
          principal,
          action == Action::"SendMail",
          resource == Recipient::"dest@agency.gov"
        );
    "#;
    let e = PolicyEngine::from_strings(policies, None).expect("policies parse");
    assert!(e.can_send(
        "alice@agency.gov",
        "dest@agency.gov",
        &dmarc_off("alice@agency.gov"),
    ));
    assert!(!e.can_send(
        "alice@agency.gov",
        "someone-else@agency.gov",
        &dmarc_off("alice@agency.gov"),
    ));
}

#[test]
fn can_attach_carries_dmarc_context_alongside_attachment_fields() {
    let policies = r#"
        permit(principal, action == Action::"Attach", resource)
          when {
            context.content_type == "application/pdf" &&
            context.dmarc_result == "pass"
          };
    "#;
    let e = PolicyEngine::from_strings(policies, None).expect("policies parse");
    let att = AttachmentCheck {
        filename: Some("brief.pdf"),
        content_type: "application/pdf",
        size_bytes: 1_000,
    };
    assert!(e.can_attach(
        "alice@agency.gov",
        &att,
        &dmarc_pass("alice@agency.gov", "alice@agency.gov"),
    ));
    assert!(!e.can_attach(
        "alice@agency.gov",
        &att,
        &dmarc_off("alice@agency.gov"),
    ));
}
