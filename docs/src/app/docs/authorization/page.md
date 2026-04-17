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
| `Action::"SendMail"` | At end-of-DATA, after DMARC has run. | The envelope sender from `MAIL FROM` (or, in DMARC `enforce` mode with `pass`, the DMARC-aligned `From:` header). | The recipient address from `RCPT TO`, as `Recipient::"<email>"`. |
| `Action::"Attach"` | For each attachment parsed from the email body, before it is forwarded or uploaded. | Same principal as the message's `SendMail`. | The attachment (filename, content type, size). |

A denial on either action causes MailLaser to reject the transaction at end-of-DATA with `550 5.7.1 Sender not authorized` (`SendMail`) or `550 5.7.1 Attachment not permitted by policy` (`Attach`). Deferring `SendMail` until end-of-DATA is what makes DMARC authentication facts available in policy context; see *DMARC and the principal* below.

---

## Entity model

MailLaser passes structured entities into each Cedar request so your policies can reason about them.

**Principal (`User`)**:
- `email`: the sender address (lowercased)
- `domain`: the sender's domain part

**Resource for `SendMail` (`Recipient`)**: the UID is `Recipient::"<recipient-address>"` (lowercased). Match a specific address with `resource == Recipient::"alerts@mydomain.com"`. Entity attributes can be attached via the Cedar entities JSON file if your policies need to reason about the recipient beyond its UID.

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

When `MAIL_LASER_DMARC_MODE=enforce` and a message passes DMARC alignment, the principal handed to `SendMail` is the DMARC-aligned `From:` address rather than the envelope `MAIL FROM`. This lets you write policies that trust the *authenticated* sender identity. When DMARC is `off`, `monitor`, or the message did not pass, the envelope sender is used — which an attacker can forge. See [DMARC validation](/docs/dmarc).

MailLaser also surfaces the full DMARC outcome to every `SendMail` and `Attach` evaluation as Cedar context. The same fields are mirrored onto both actions so policies can gate attachments on authentication too.

| Context field | Type | Values |
|---------------|------|--------|
| `context.dmarc_result` | String | `"pass"`, `"fail"`, `"none"`, `"temperror"`, or `"off"` (when DMARC validation is disabled). |
| `context.dmarc_aligned` | Bool | `true` only when `dmarc_result == "pass"`. |
| `context.authenticated_from` | String | The aligned `From:` address when `dmarc_aligned`, otherwise the empty string. Guard with `context.authenticated_from != ""`. |
| `context.envelope_from` | String | The envelope `MAIL FROM`, regardless of which identity became principal. Lets policies cross-check claimed vs. authenticated identity. |
| `context.helo` | String | The HELO/EHLO domain the client announced. |
| `context.peer_ip` | String | The peer IP the connection came from. |

**Require DMARC pass before accepting mail**:

```cedar
forbid(principal, action == Action::"SendMail", resource)
unless { context.dmarc_result == "pass" };
```

**Block envelope spoofing — envelope must match the authenticated identity**:

```cedar
forbid(principal, action == Action::"SendMail", resource)
when {
  context.dmarc_aligned == true &&
  context.envelope_from != context.authenticated_from
};
```

**Only allow executables when DMARC-aligned and from a trusted domain**:

```cedar
permit(principal, action == Action::"Attach", resource)
when {
  context.dmarc_aligned == true &&
  principal.domain == "partner.com" &&
  context.content_type == "application/x-msdownload"
};
```

---

## Troubleshooting denials

A policy denial surfaces in two places.

- **SMTP reply**: `550 5.7.1 Sender not authorized` for a `SendMail` denial, or `550 5.7.1 Attachment not permitted by policy` for an `Attach` denial. Both are issued at end-of-DATA, after DMARC evaluation.
- **Logs**: every denial is logged at `warn` with the sender and the denied resource. Run with `RUST_LOG=mail_laser::policy=debug` during policy development to see the full evaluation trace (principal, action, resource, decision).

If MailLaser fails to start with `cedar_policy::…`, the policy file has a syntax error. Cedar errors name the rule and line; fix and restart.
