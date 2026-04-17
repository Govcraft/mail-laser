---
title: Authorization
nextjs:
  metadata:
    title: Authorization
    description: How MailLaser uses Cedar policies to decide which senders may send mail and which attachments are allowed.
---

Every message and every attachment passes through a Cedar policy engine before delivery. The policy file you configure decides who is allowed to send mail to whom and which attachments are permitted, giving you a single declarative place to express authorization rules without writing code.

---

## What Cedar is

Cedar is a declarative authorization language developed by AWS. A Cedar policy is a set of `permit` and `forbid` rules evaluated against a request of the form *"can this **principal** perform this **action** on this **resource**?"* MailLaser loads your policy file at startup, evaluates every inbound message against it, and rejects any request that isn't permitted. You don't need prior Cedar experience to use MailLaser — the rest of this page covers everything you need.

The canonical language reference is at [cedarpolicy.com](https://www.cedarpolicy.com/).

---

## Actions MailLaser evaluates

MailLaser evaluates two actions against your policy.

| Action | When it fires | Principal | Resource |
|--------|---------------|-----------|----------|
| `Action::"SendMail"` | After `RCPT TO` succeeds, before `DATA` is accepted. | The envelope sender from `MAIL FROM` (or, in DMARC `enforce` mode with `pass`, the aligned `From:` header). | The recipient address from `RCPT TO`. |
| `Action::"Attach"` | For each attachment parsed from the email body, before it is forwarded or uploaded. | Same principal as the message's `SendMail`. | The attachment (filename, content type, size). |

A denial on `SendMail` causes MailLaser to reject the `RCPT TO` with `550 5.7.1 Sender not authorized`. A denial on `Attach` causes the entire message to be rejected at end-of-DATA with `550 5.7.1 Attachment not permitted by policy`.

---

## Entity model

MailLaser passes structured entities into each Cedar request so your policies can reason about them.

**Principal (`User`)**:
- `email`: the sender address (lowercased)
- `domain`: the sender's domain part

**Resource for `SendMail` (`Recipient`)**:
- `email`: the recipient address (lowercased)

**Resource for `Attach` (`Attachment`)**:
- `filename`: optional filename from the Content-Disposition header
- `content_type`: MIME type (e.g. `application/pdf`)
- `size_bytes`: decoded byte length

Optional entities (users, groups, attributes) can be supplied via `MAIL_LASER_CEDAR_ENTITIES` pointing to a Cedar entities JSON file.

---

## Minimal policy

The smallest policy that matches v2.0's "accept everything that passed recipient validation" behavior is two lines:

```cedar
permit(principal, action == Action::"SendMail", resource);
permit(principal, action == Action::"Attach", resource);
```

Store this at the path referenced by `MAIL_LASER_CEDAR_POLICIES` and MailLaser starts accepting mail immediately.

---

## Tightening the policy

Cedar `when` clauses let you express the rules you'd otherwise write in application code.

**Accept mail only from a specific domain**:

```cedar
permit(principal, action == Action::"SendMail", resource)
when { principal.domain == "partner.com" };
```

**Block executable attachments regardless of sender**:

```cedar
forbid(principal, action == Action::"Attach", resource)
when {
  resource.content_type == "application/x-msdownload" ||
  resource.filename like "*.exe"
};
```

**Cap per-recipient attachment size**:

```cedar
forbid(principal, action == Action::"Attach", resource)
when { resource.size_bytes > 2097152 }; // 2 MiB
```

`forbid` takes precedence over `permit`, so you can layer restrictive rules on top of a permissive baseline.

---

## DMARC and the principal

When `MAIL_LASER_DMARC_MODE=enforce` and a message passes DMARC alignment, the principal handed to `SendMail` is the DMARC-aligned `From:` address rather than the envelope `MAIL FROM`. This lets you write policies that trust the *authenticated* sender identity. When DMARC is `off` or the message did not pass, the envelope sender is used — which an attacker can forge. See [DMARC validation](/docs/dmarc).

---

## Troubleshooting denials

A policy denial surfaces in two places.

- **SMTP reply**: `550 5.7.1 Sender not authorized` for a `SendMail` denial at `RCPT TO`, or `550 5.7.1 Attachment not permitted by policy` at end-of-DATA for an `Attach` denial.
- **Logs**: every denial is logged at `warn` with the sender and the denied resource. Run with `RUST_LOG=mail_laser::policy=debug` during policy development to see the full evaluation trace (principal, action, resource, decision).

If MailLaser fails to start with `cedar_policy::…`, the policy file has a syntax error. Cedar errors name the rule and line; fix and restart.
