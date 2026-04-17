---
title: Attachments
nextjs:
  metadata:
    title: Attachments
    description: How MailLaser parses email attachments and forwards them to your webhook inline or via S3-compatible storage.
---

MailLaser parses every MIME attachment out of incoming email and delivers it to your webhook in one of two shapes: base64-encoded inline in the JSON payload, or uploaded to an S3-compatible bucket with a URL in the payload. This page covers both delivery modes, the size caps that bound them, and the schema your consumer sees.

---

## Choosing a delivery mode

The `MAIL_LASER_ATTACHMENT_DELIVERY` setting picks between the two modes.

| Mode | When to use | Trade-off |
|------|-------------|-----------|
| `inline` (default) | Small attachments, simple consumer, no object storage available. | Base64 inflates payload ~33%. Counts against the message size cap. |
| `s3` | Large files, many attachments per message, or consumers that prefer to fetch lazily. | Requires credentials for AWS, MinIO, R2, or Wasabi. Adds upload latency to each delivery. |

Both modes enforce the same size limits and the same Cedar `Attach` policy check — the only difference is where the bytes end up.

---

## Size caps

Two limits bound attachment handling.

| Variable | Default | What it controls |
|----------|---------|------------------|
| `MAIL_LASER_MAX_MESSAGE_SIZE` | 25 MiB | Total SMTP message size. Advertised to clients via the EHLO `SIZE` extension. Oversize messages are rejected with `552 5.3.4`. |
| `MAIL_LASER_MAX_ATTACHMENT_SIZE` | 10 MiB | Maximum size for any single attachment after MIME decoding. Attachments larger than this trigger a `552 5.3.4 Attachment exceeds size limit` at end-of-DATA. |

The message cap stops abuse at the SMTP layer before MailLaser allocates memory. The per-attachment cap stops a single huge attachment from filling memory even when the total message fits.

---

## Cedar authorization

Every attachment is evaluated against your Cedar policy with `Action::"Attach"` before it is delivered. The policy sees the filename, content type, and decoded size; a denial rejects the entire message with `550 5.7.1 Attachment not permitted by policy`. See [Authorization](/docs/authorization) for policy examples.

---

## Inline mode

With `MAIL_LASER_ATTACHMENT_DELIVERY=inline` (the default), each attachment appears in the payload as:

```json
{
  "filename": "report.pdf",
  "content_type": "application/pdf",
  "size_bytes": 482193,
  "content_id": null,
  "delivery": "inline",
  "data_base64": "JVBERi0xLjQKJeLjz9MK..."
}
```

The `data_base64` field is the raw MIME-decoded bytes re-encoded as base64. Your consumer decodes it and writes it to disk, passes it along, or does whatever your workflow needs.

---

## S3 mode

With `MAIL_LASER_ATTACHMENT_DELIVERY=s3`, MailLaser uploads each attachment to the configured bucket and emits a URL in the payload:

```json
{
  "filename": "report.pdf",
  "content_type": "application/pdf",
  "size_bytes": 482193,
  "content_id": null,
  "delivery": "s3",
  "url": "s3://mail-laser-inbound/2026/04/17/2f8c...pdf",
  "presigned_url": "https://mail-laser-inbound.s3.us-east-1.amazonaws.com/2026/04/17/2f8c...pdf?X-Amz-Algorithm=..."
}
```

Object keys are unique (UUID-suffixed) so uploads never collide. The `presigned_url` field is present only when `MAIL_LASER_S3_PRESIGN_TTL` is set — it's a time-limited GET URL your consumer can hand to a browser or untrusted client without sharing bucket credentials.

### Configuration

| Variable | Required | Notes |
|----------|----------|-------|
| `MAIL_LASER_S3_BUCKET` | Yes | Target bucket name. |
| `MAIL_LASER_S3_REGION` | Yes | Region (e.g. `us-east-1`, `auto` for R2). |
| `MAIL_LASER_S3_ENDPOINT` | No | Custom endpoint URL for S3-compatible stores. Omit to use AWS. Examples: `https://<accountid>.r2.cloudflarestorage.com`, `http://minio.local:9000`. |
| `MAIL_LASER_S3_KEY_PREFIX` | No | Prepended to every object key. Useful when sharing a bucket, e.g. `inbound/`. |
| `MAIL_LASER_S3_PRESIGN_TTL` | No | Seconds a presigned GET URL stays valid. Omit to skip presigning. |

### Credentials

MailLaser uses the standard AWS credential chain — environment variables, IAM instance role, shared credentials file, or container credentials. Credentials are not part of MailLaser's own configuration.

---

## Payload field reference

The `attachments` array on the webhook payload appears only when at least one attachment passed policy and was successfully prepared. It is omitted entirely on messages without attachments. Each entry has:

| Field | Type | Description |
|-------|------|-------------|
| `filename` | string (optional) | From the Content-Disposition header. Absent when the email provides none. |
| `content_type` | string | MIME type as declared by the sender. |
| `size_bytes` | integer | Decoded byte length. |
| `content_id` | string (optional) | The `Content-ID` header value for inline images referenced from the HTML body. |
| `delivery` | `"inline"` or `"s3"` | Mode tag. Tells the consumer which other fields to expect. |
| `data_base64` | string | Present only when `delivery == "inline"`. |
| `url` | string | Present only when `delivery == "s3"`. An `s3://` URI to the stored object. |
| `presigned_url` | string (optional) | Present only when `delivery == "s3"` and `MAIL_LASER_S3_PRESIGN_TTL` is configured. |

---

## Verifying

Send an email with an attached file and check the webhook payload. In inline mode the `attachments[0].data_base64` field decodes to the original file bytes. In S3 mode the `url` should match an object in your bucket, and the presigned URL (if enabled) should return the file body on a `curl -L` with no auth.

If the message is rejected at end-of-DATA, check the SMTP reply code — `552` indicates a size-cap hit; `550 5.7.1 Attachment not permitted by policy` indicates a Cedar denial. Both are logged at `warn`.
