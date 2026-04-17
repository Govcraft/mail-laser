// Example Cedar policies for mail-laser.
//
// Two actions are evaluated per incoming message:
//   Action::"SendMail" — checked at the SMTP MAIL FROM command. The principal
//     is `User::"<mail-from-address>"` (lowercased). Deny here rejects the
//     entire transaction with 550 5.7.1 Sender not authorized.
//
//   Action::"Attach"   — checked per attachment after parsing. The resource
//     is a singleton `Attachment::"inbound"` and policy context exposes:
//       - context.content_type : String  (e.g. "application/pdf")
//       - context.size_bytes   : Long
//       - context.filename     : String  (empty when MIME has no filename)
//     Any denied attachment rejects the whole message with
//     550 5.7.1 Attachment not permitted by policy.
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
