use crate::policy::{AttachmentCheck, PolicyEngine};

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

#[test]
fn can_send_allows_listed_user() {
    assert!(engine().can_send("alice@agency.gov"));
}

#[test]
fn can_send_is_case_insensitive() {
    assert!(engine().can_send("Alice@AGENCY.gov"));
}

#[test]
fn can_send_denies_unknown_user() {
    assert!(!engine().can_send("mallory@evil.example"));
}

#[test]
fn can_attach_allows_pdf_within_size() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("brief.pdf"),
        content_type: "application/pdf",
        size_bytes: 2_000_000,
    };
    assert!(e.can_attach("alice@agency.gov", &att));
}

#[test]
fn can_attach_denies_disallowed_content_type() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("payload.exe"),
        content_type: "application/x-msdownload",
        size_bytes: 1_000,
    };
    assert!(!e.can_attach("alice@agency.gov", &att));
}

#[test]
fn can_attach_denies_when_over_size() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("giant.pdf"),
        content_type: "application/pdf",
        size_bytes: 10_485_761,
    };
    assert!(!e.can_attach("alice@agency.gov", &att));
}

#[test]
fn can_attach_denies_unauthorized_principal() {
    let e = engine();
    let att = AttachmentCheck {
        filename: Some("brief.pdf"),
        content_type: "application/pdf",
        size_bytes: 1_000,
    };
    assert!(!e.can_attach("bob@agency.gov", &att));
}

#[test]
fn default_is_deny_when_no_policy_matches() {
    let e = PolicyEngine::from_strings("", None).expect("empty policy set");
    assert!(!e.can_send("alice@agency.gov"));
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
    assert!(engine.can_send("alice@agency.gov"));
    assert!(engine.can_attach(
        "alice@agency.gov",
        &AttachmentCheck {
            filename: Some("brief.pdf"),
            content_type: "application/pdf",
            size_bytes: 1_000_000,
        }
    ));

    // Bob can send but can't attach docx (he's PDF-only, under 2 MiB).
    assert!(engine.can_send("bob@agency.gov"));
    assert!(!engine.can_attach(
        "bob@agency.gov",
        &AttachmentCheck {
            filename: Some("doc.docx"),
            content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            size_bytes: 1_000,
        }
    ));
    assert!(engine.can_attach(
        "bob@agency.gov",
        &AttachmentCheck {
            filename: Some("small.pdf"),
            content_type: "application/pdf",
            size_bytes: 500_000,
        }
    ));

    // Unlisted user is denied.
    assert!(!engine.can_send("eve@evil.example"));
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
    assert!(e.can_send("carol@agency.gov"));
    assert!(!e.can_send("dan@agency.gov"));
}
