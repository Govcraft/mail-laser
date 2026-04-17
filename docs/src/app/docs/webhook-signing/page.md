---
title: Webhook signing
nextjs:
  metadata:
    title: Webhook signing
    description: Verify that each MailLaser webhook delivery is authentic and unmodified using HMAC-SHA256 request signing.
---

Setting `MAIL_LASER_WEBHOOK_SIGNING_SECRET` enables HMAC-SHA256 request signing so your webhook endpoint can verify that each delivery originated from MailLaser and has not been tampered with in transit. This page covers the header format, the signed-string format, and verification recipes in Node.js and Python. When the secret is unset, no signing headers are emitted and the request shape is unchanged.

---

## Headers

When signing is enabled, every outbound POST carries two additional headers:

| Header | Value |
|--------|-------|
| `X-MailLaser-Timestamp` | Unix time in seconds when the request was signed (e.g. `1700000000`). |
| `X-MailLaser-Signature-256` | `sha256=<hex>`, where `<hex>` is the lowercase HMAC-SHA256 of `<timestamp>.<body>` using the configured secret as the key. |

The timestamp lives inside the MAC (not just as a separate header), so an attacker cannot replay an old body under a fresh clock. Your verifier should both recompute the MAC and reject requests whose timestamp is outside a reasonable tolerance — five minutes is a good default.

---

## Node.js

```js
import crypto from "node:crypto";

function verify(req, secret, toleranceSecs = 300) {
  const ts = req.headers["x-maillaser-timestamp"];
  const sig = req.headers["x-maillaser-signature-256"];
  if (!ts || !sig?.startsWith("sha256=")) return false;

  const age = Math.abs(Math.floor(Date.now() / 1000) - Number(ts));
  if (age > toleranceSecs) return false;

  const expected = crypto
    .createHmac("sha256", secret)
    .update(`${ts}.${req.rawBody}`) // rawBody must be the exact bytes received
    .digest("hex");

  return crypto.timingSafeEqual(
    Buffer.from(sig.slice("sha256=".length), "hex"),
    Buffer.from(expected, "hex"),
  );
}
```

---

## Python

```python
import hmac, hashlib, time

def verify(headers, raw_body, secret, tolerance=300):
    ts = headers.get("X-MailLaser-Timestamp")
    sig = headers.get("X-MailLaser-Signature-256", "")
    if not ts or not sig.startswith("sha256="):
        return False
    if abs(int(time.time()) - int(ts)) > tolerance:
        return False
    expected = hmac.new(
        secret.encode(),
        f"{ts}.".encode() + raw_body,
        hashlib.sha256,
    ).hexdigest()
    return hmac.compare_digest(sig[len("sha256=") :], expected)
```

---

## Implementation notes

{% callout type="warning" title="Sign the raw body" %}
You must HMAC the exact bytes your framework received, before any parsing or re-serialization. A JSON round-trip will usually reorder keys or alter whitespace and break verification. In Express use `express.raw()` or preserve the buffer on the request; in FastAPI use `await request.body()`; in Flask use `request.get_data()`.
{% /callout %}

- **Use a constant-time compare.** `crypto.timingSafeEqual` in Node, `hmac.compare_digest` in Python. A `==` compare leaks timing information that can be used to recover a signature byte-by-byte.
- **Pick your tolerance deliberately.** Five minutes tolerates normal clock skew; ten minutes is fine for slow queues. Anything over an hour defeats the replay protection.
- **Rotate the secret through your proxy or secrets manager**, not by restarting MailLaser. MailLaser reads the secret at startup — a rolling deployment (two instances, drain-drain) avoids dropping in-flight SMTP sessions during rotation.
- **Secret never appears in logs.** MailLaser redacts the secret at startup (logs show `<set>` or `<not set>` only). Verifiers should do the same.

---

## What this does not do

Signing proves that the payload came from a process holding the secret and has not been modified in transit. It does not:

- **Authenticate the original email sender.** Use [DMARC validation](/docs/dmarc) for that.
- **Authorize the sender.** Use [Cedar policies](/docs/authorization) for that.
- **Encrypt the payload.** TLS to the webhook URL does that; MailLaser enforces HTTPS-only connections in release builds.
