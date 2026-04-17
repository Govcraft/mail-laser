---
title: Upgrading to v3
nextjs:
  metadata:
    title: Upgrading to v3
    description: Migrate an existing MailLaser v2.0 deployment to v3.0 without changing inbound behavior.
---

This guide walks you through upgrading an existing v2.0 deployment to v3.0 without changing how mail is accepted. The only required change is pointing MailLaser at a Cedar policy file. Once the binary is running, you can opt into v3's new capabilities one at a time.

---

## What changed

v3.0 adds four capability surfaces and one required config variable.

| Change | Impact | Required action |
|--------|--------|-----------------|
| **Cedar authorization** | Every message is now evaluated against a policy file. | **Create a policy file** and set `MAIL_LASER_CEDAR_POLICIES`. |
| **Attachments** | Emails with attachments are now parsed and forwarded (inline or to S3). | None. Default `inline` mode keeps payloads self-contained. |
| **DMARC validation** | Optional gate that can reject spoofed `From:` at the SMTP layer. | None — defaults to `off`. |
| **Webhook signing** | Optional HMAC-SHA256 header on every outbound request. | None — off unless `MAIL_LASER_WEBHOOK_SIGNING_SECRET` is set. |
| **EHLO `SIZE`** | MailLaser now advertises `SIZE` and rejects oversized messages with `552`. | None — default cap is 25 MiB; increase `MAIL_LASER_MAX_MESSAGE_SIZE` if needed. |

The payload schema gains three optional fields (`attachments`, `dmarc_result`, `authenticated_from`) that are absent from v2-style payloads and appear only when the corresponding feature is enabled. Existing consumers that ignore unknown fields are unaffected.

---

## Step 1: Write a v2-compatible Cedar policy

Create a file at `/etc/mail-laser/policies.cedar` (or wherever you prefer) containing:

```cedar
permit(principal, action == Action::"SendMail", resource);
permit(principal, action == Action::"Attach", resource);
```

This accepts every sender and every attachment, matching v2.0's behavior. You can tighten it later — see [Authorization](/docs/authorization) for how.

---

## Step 2: Point MailLaser at the policy

Add one environment variable:

```shell
MAIL_LASER_CEDAR_POLICIES=/etc/mail-laser/policies.cedar
```

If you run MailLaser in Docker, also mount the policy file into the container. See [Docker](/docs/docker) for the full compose snippet.

At this point your deployment behaves identically to v2.0 — accept the same mail, forward the same payload shape — with attachments now passing through as well.

---

## Step 3 (optional): Opt into DMARC

If your MailLaser instance sits behind a public MX, enable DMARC to stop spoofed `From:` headers:

```shell
MAIL_LASER_DMARC_MODE=monitor   # start in monitor to observe outcomes
```

Monitor mode annotates each payload with a `dmarc_result` field without rejecting anything. After watching a few days of real traffic, switch to `enforce` to actually reject failures. See [DMARC validation](/docs/dmarc).

---

## Step 4 (optional): Opt into webhook signing

If you want your receiver to verify each delivery came from MailLaser:

```shell
MAIL_LASER_WEBHOOK_SIGNING_SECRET=<a-high-entropy-secret>
```

Every request will then carry `X-MailLaser-Timestamp` and `X-MailLaser-Signature-256` headers. See [Webhook signing](/docs/webhook-signing) for the verification recipe.

---

## Step 5 (optional): Move large attachments to S3

Inline delivery base64-encodes attachment bytes into the JSON payload, which inflates payload size by roughly 33% and caps out at the message-size limit. For large files or many attachments, upload to an S3-compatible bucket instead:

```shell
MAIL_LASER_ATTACHMENT_DELIVERY=s3
MAIL_LASER_S3_BUCKET=mail-laser-inbound
MAIL_LASER_S3_REGION=us-east-1
```

See [Attachments](/docs/attachments) for the full set of S3 options and presigned-URL behavior.

---

## Verifying the upgrade

After restarting, check that:

- **Startup logs** show `Config: Using cedar_policies_path: ...` and no "missing variable" errors.
- **A test email** sent via `swaks` is accepted and forwarded.
- **The webhook payload** contains the same fields as before. New optional fields (`attachments`, `dmarc_result`, `authenticated_from`) appear only if you enabled the corresponding feature.

If you enabled DMARC in `monitor` mode, every payload should carry a `dmarc_result` field — `pass`, `fail`, `none`, or `temperror`. If you enabled signing, every webhook request should carry both `X-MailLaser-Timestamp` and `X-MailLaser-Signature-256` headers.
