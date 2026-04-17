---
title: DMARC validation
nextjs:
  metadata:
    title: DMARC validation
    description: Enable SPF, DKIM, and DMARC checks on inbound mail to reject spoofed senders at the SMTP layer.
---

DMARC validation is an opt-in gate that authenticates the `From:` header on every inbound message before MailLaser delivers it. When enabled, MailLaser can either annotate the payload with the validation outcome or reject spoofed mail outright at SMTP — essential for any deployment behind a public MX record.

---

## Why you want it

Without DMARC, MailLaser accepts whatever `From:` address an SMTP client chooses to send. An attacker can trivially spoof mail as `ceo@yourcompany.com` to a MailLaser instance you run publicly. That matters because downstream Cedar policies, audit logs, and webhook consumers otherwise treat a forged sender as authentic. DMARC closes this gap by chaining SPF and DKIM results through DNS-published policy to verify the aligned `From:` identity.

If your MailLaser instance sits in a private network and only accepts mail from trusted internal senders, DMARC is optional. For any public-MX deployment, run at least `monitor` mode.

---

## Three modes

`MAIL_LASER_DMARC_MODE` picks how strict to be.

| Mode | Behavior | When to use |
|------|----------|-------------|
| `off` (default) | No validation. Zero DNS traffic. | Private networks; trusted internal senders only. |
| `monitor` | Full SPF + DKIM + DMARC check. Annotates the payload with the outcome. Always returns `250 OK`. | First rollout; watch live traffic without blocking anything. |
| `enforce` | Same check. Returns `550 5.7.1` on `fail`. Optionally `451 4.7.0` on temperror. | Production on a public MX, after `monitor` confirms legitimate senders pass. |

---

## Payload annotations

In `monitor` and `enforce` modes, every delivered payload carries a `dmarc_result` field.

| Value | Meaning |
|-------|---------|
| `pass` | DMARC alignment succeeded. The payload also carries `authenticated_from` with the verified `From:` address. Cedar receives this address as the principal. |
| `fail` | DMARC explicitly failed. In `enforce` mode this never reaches the webhook — the message is rejected at SMTP. |
| `none` | No DMARC record published for the `From:` domain. Message is accepted. |
| `temperror` | DNS lookup failed (timeout, SERVFAIL). Handling depends on `MAIL_LASER_DMARC_TEMPERROR_ACTION`. |

The `authenticated_from` field is present only when `dmarc_result == "pass"`. Use it in downstream consumers when you need to trust the identity rather than the envelope.

---

## DNS requirements

DMARC needs working outbound DNS. MailLaser wraps the SPF, DKIM, and DMARC lookups in a single timeout, governed by `MAIL_LASER_DMARC_DNS_TIMEOUT` (default 5 seconds).

| Variable | Default | Purpose |
|----------|---------|---------|
| `MAIL_LASER_DMARC_DNS_TIMEOUT` | `5` (seconds) | Overall timeout across all DNS lookups for one message. |
| `MAIL_LASER_DMARC_DNS_SERVERS` | *(system)* | Explicit comma-separated resolvers as `ip:port`. Empty uses the OS resolver. Useful in locked-down networks. |

If outbound DNS is blocked, every message times out and becomes a `temperror`. Configure `MAIL_LASER_DMARC_TEMPERROR_ACTION=accept` in `enforce` mode only if you prefer fail-open; the default is `reject`, which returns `451 4.7.0` so the sending MTA retries later.

---

## Rolling out

Run three stages in sequence:

1. **`off` → `monitor`**: turn on monitor mode and watch logs for a few days of real traffic. Every message is processed; `dmarc_result` appears in each payload. Note which legitimate senders return `fail` or `none` and fix their SPF/DKIM records before advancing.
2. **`monitor` → `enforce` with `MAIL_LASER_DMARC_TEMPERROR_ACTION=accept`**: this rejects explicit failures but tolerates DNS hiccups. You now block spoofed mail without risking legitimate mail during resolver outages.
3. **Tighten temperror**: once you're confident in your resolver's reliability, switch the temperror action to `reject` for the strongest stance.

You can skip stage 2 if your resolver is reliable, but stage 1 (monitor) is never optional — it's the only way to catch legitimate senders whose DMARC you'd otherwise break.

---

## Interaction with Cedar

In `enforce` mode, when a message passes DMARC alignment, the principal MailLaser hands to the Cedar `SendMail` evaluation is the DMARC-aligned `From:` address rather than the envelope `MAIL FROM`. This lets authorization policies trust the authenticated identity. In `monitor` and `off` modes — and whenever DMARC returns `fail`, `none`, or `temperror` — the envelope sender is used, which a spoofer controls. Monitor mode stays strictly observational so flipping it on never changes authorization outcomes.

Regardless of mode, the full DMARC outcome is also exposed to every `SendMail` and `Attach` evaluation as Cedar context (`context.dmarc_result`, `context.dmarc_aligned`, `context.authenticated_from`, `context.envelope_from`, `context.helo`, `context.peer_ip`). See [Authorization](/docs/authorization) for the full list and example policies.

---

## Verifying

In `monitor` mode, send a message from an unaligned source and confirm the payload shows `dmarc_result: "fail"` (or `"none"` if the sender domain publishes no DMARC record). In `enforce` mode, the same message should be rejected at DATA with `550 5.7.1` and never reach your webhook.

Run with `RUST_LOG=mail_laser::dmarc=debug` to see the full SPF/DKIM/DMARC trace per message while you validate the configuration.
