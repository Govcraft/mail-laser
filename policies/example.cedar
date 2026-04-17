// Example Cedar policies for mail-laser.
//
// Two actions are evaluated per incoming message, both at end-of-DATA (after
// DMARC has run):
//
//   Action::"SendMail" — principal is `User::"<address>"` (lowercased). When
//     DMARC is in `enforce` mode and the message passed alignment, the
//     principal is the DMARC-aligned `From:`; otherwise the envelope
//     `MAIL FROM`. Resource is `Recipient::"<recipient-address>"`.
//     Deny here rejects the transaction with 550 5.7.1 Sender not authorized.
//
//   Action::"Attach"   — checked per attachment. Resource is the singleton
//     `Attachment::"inbound"`. Attachment-specific context:
//       - context.content_type : String  (e.g. "application/pdf")
//       - context.size_bytes   : Long
//       - context.filename     : String  (empty when MIME has no filename)
//     Any denied attachment rejects the whole message with
//     550 5.7.1 Attachment not permitted by policy.
//
// Shared DMARC context surfaced on BOTH actions:
//       - context.dmarc_result       : String ("pass"|"fail"|"none"|"temperror"|"off")
//       - context.dmarc_aligned      : Bool   (true iff dmarc_result == "pass")
//       - context.authenticated_from : String (aligned From, "" when absent)
//       - context.envelope_from      : String (envelope MAIL FROM always)
//       - context.helo               : String
//       - context.peer_ip            : String
//
// Default is DENY. Replace the example entities in `entities.json` with the
// real allowlist for this deployment. Document every per-user carve-out; this
// file is the audit trail.

// ---------- SendMail ----------

// Members of the ApprovedSenders group may submit mail through mail-laser.
permit(
  principal in Group::"ApprovedSenders",
  action == Action::"SendMail",
  resource
);

// ---------- Attach ----------

// Alice may attach PDFs and Word documents up to 10 MiB each.
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

// Bob is restricted to PDFs only, up to 2 MiB.
permit(
  principal == User::"bob@agency.gov",
  action == Action::"Attach",
  resource
) when {
  context.content_type == "application/pdf" &&
  context.size_bytes <= 2097152
};

// Note: Bob is in ApprovedSenders (may send), but has no `Attach` permit that
// allows text files; any attachment other than PDFs under 2 MiB is rejected.

// ---------- Example: DMARC-aware hardening ----------
//
// Uncomment to require DMARC alignment for every inbound message. With DMARC
// mode `off`, `dmarc_result` is "off" and this forbid always fires, so enable
// DMARC validation before turning this on.
//
// forbid(principal, action == Action::"SendMail", resource)
//   unless { context.dmarc_result == "pass" };

// Uncomment to block envelope-spoofing: when DMARC passed, the envelope must
// match the authenticated From identity.
//
// forbid(principal, action == Action::"SendMail", resource)
//   when {
//     context.dmarc_aligned == true &&
//     context.envelope_from != context.authenticated_from
//   };
