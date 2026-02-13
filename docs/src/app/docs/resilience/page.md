---
title: Resilience
nextjs:
  metadata:
    title: Resilience
    description: MailLaser's circuit breaker and retry with exponential backoff protect your webhook from cascading failures.
---

MailLaser includes two resilience patterns that protect both your webhook endpoint and the MailLaser process itself from cascading failures: retry with exponential backoff and a circuit breaker.

---

## Retry with exponential backoff

When a webhook delivery fails (HTTP error or timeout), MailLaser retries the request with increasing delays between attempts.

### Backoff schedule

| Attempt | Delay before attempt |
|---------|---------------------|
| 1 (initial) | None |
| 2 (first retry) | 100ms |
| 3 (second retry) | 200ms |
| 4 (third retry) | 400ms |

The delay doubles with each retry: `100ms * 2^(attempt - 1)`. With the default `MAIL_LASER_WEBHOOK_MAX_RETRIES=3`, MailLaser makes up to 4 total attempts (1 initial + 3 retries).

Each attempt is subject to the webhook timeout (`MAIL_LASER_WEBHOOK_TIMEOUT`, default 30 seconds). If the timeout expires, the attempt counts as a failure.

### Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_WEBHOOK_MAX_RETRIES` | `3` | Number of retry attempts after the initial failure. Set to `0` to disable retries. |
| `MAIL_LASER_WEBHOOK_TIMEOUT` | `30` | Seconds before each delivery attempt times out. |

---

## Circuit breaker

The circuit breaker prevents MailLaser from repeatedly hammering an unresponsive webhook endpoint. When consecutive failures exceed a threshold, the circuit "opens" and subsequent emails are dropped without attempting delivery.

### States

The circuit breaker has three states:

**Closed** (normal operation)
: All webhook deliveries are attempted normally. Each failure increments a consecutive failure counter. Each success resets the counter to zero.

**Open** (protection mode)
: Triggered when consecutive failures reach `MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD` (default 5). In this state, all incoming emails are dropped immediately without attempting webhook delivery. The webhook actor logs a warning for each dropped email.

**Half-open** (recovery probe)
: After `MAIL_LASER_CIRCUIT_BREAKER_RESET` seconds (default 60) have elapsed since the circuit opened, the next email triggers a probe. If the delivery succeeds, the circuit closes and the failure counter resets. If it fails, the circuit re-opens.

### State transitions

```text
           success
    +----[resets counter]----+
    |                        |
    v                        |
 CLOSED ---[threshold failures]--> OPEN ---[reset timer expires]--> HALF-OPEN
    ^                                                                  |
    |                                                                  |
    +----------------------[success]-----------------------------------+
    |                                                                  |
    |                         OPEN <-----------[failure]---------------+
```

### Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD` | `5` | Consecutive failures required to open the circuit. |
| `MAIL_LASER_CIRCUIT_BREAKER_RESET` | `60` | Seconds before an open circuit transitions to half-open. |

{% callout type="warning" title="Emails are dropped, not queued" %}
When the circuit breaker is open, incoming emails are discarded. MailLaser does not queue emails for later delivery. If your webhook is down for an extended period, emails received during that window are lost. For use cases where email loss is unacceptable, place a message queue between MailLaser and your final destination.
{% /callout %}

---

## How the patterns work together

When an email arrives, the webhook actor applies both patterns in sequence:

1. **Circuit breaker check**: If the circuit is open and the reset period has not elapsed, the email is dropped immediately. No retries are attempted.
2. **Delivery with retries**: If the circuit is closed (or half-open), the email is delivered with the full retry sequence.
3. **Result feedback**: After all attempts complete, the success or failure feeds back into the circuit breaker:
   - Success: Consecutive failure counter resets to zero. If the circuit was half-open, it closes.
   - Failure: Consecutive failure counter increments. If it reaches the threshold, the circuit opens.

### Example scenario

With default settings (`max_retries=3`, `threshold=5`, `reset=60`):

1. Webhook goes down
2. Email 1: 4 attempts fail (initial + 3 retries) -- consecutive failures = 1
3. Email 2: 4 attempts fail -- consecutive failures = 2
4. Emails 3, 4, 5: Same pattern -- consecutive failures reach 5
5. Circuit opens
6. Emails 6 through N: Dropped immediately for the next 60 seconds
7. After 60 seconds: Circuit transitions to half-open
8. Next email: One probe attempt is made
9. If webhook is back: Circuit closes, normal operation resumes
10. If webhook still down: Circuit re-opens for another 60 seconds

---

## Monitoring resilience

The webhook actor logs key events at appropriate levels:

- `info`: Successful deliveries and retry attempts
- `warn`: Circuit breaker open (dropping emails), individual retry failures
- `error`: All retries exhausted, circuit breaker opened

Use `RUST_LOG=mail_laser::webhook=debug` for detailed resilience diagnostics.

When the webhook actor stops (during shutdown), it logs the total count of successfully forwarded emails and total failures, providing a session summary.
