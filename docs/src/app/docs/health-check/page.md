---
title: Health check
nextjs:
  metadata:
    title: Health check
    description: MailLaser's /health endpoint for monitoring integration.
---

MailLaser runs a lightweight HTTP server alongside the SMTP server to provide a health check endpoint for monitoring and orchestration systems.

---

## Endpoint

| Property | Value |
|----------|-------|
| Path | `/health` |
| Method | Any (GET, POST, HEAD, PUT, etc.) |
| Response status | `200 OK` |
| Response body | Empty |

Any request to a path other than `/health` returns `404 Not Found` with a body of `Not Found`.

```shell
# Check health
curl -i http://localhost:8080/health
# HTTP/1.1 200 OK

# Any other path returns 404
curl -i http://localhost:8080/status
# HTTP/1.1 404 Not Found
```

---

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_HEALTH_BIND_ADDRESS` | `0.0.0.0` | IP address the health check server binds to. |
| `MAIL_LASER_HEALTH_PORT` | `8080` | Port the health check server listens on. |

The health check server runs on a separate port from the SMTP server, allowing you to expose it independently in your network configuration.

---

## Monitoring integration

### Docker health check

Add a health check to your Docker run command:

```shell
docker run -d \
  --name mail-laser \
  --health-cmd="wget -q --spider http://localhost:8080/health || exit 1" \
  --health-interval=30s \
  --health-timeout=5s \
  --health-retries=3 \
  -p 2525:2525 \
  -p 8080:8080 \
  -e MAIL_LASER_TARGET_EMAILS="alerts@example.com" \
  -e MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
  ghcr.io/govcraft/mail-laser:latest
```

{% callout title="Why wget instead of curl?" %}
The MailLaser Docker image is built from `scratch` and does not include `curl`. The `wget` shown above is also unavailable in the scratch image. For Docker health checks, use a multi-stage build that includes a static health check binary, or monitor the health endpoint externally.
{% /callout %}

### Kubernetes liveness probe

```yaml
apiVersion: v1
kind: Pod
spec:
  containers:
    - name: mail-laser
      image: ghcr.io/govcraft/mail-laser:latest
      ports:
        - containerPort: 2525
          name: smtp
        - containerPort: 8080
          name: health
      livenessProbe:
        httpGet:
          path: /health
          port: health
        initialDelaySeconds: 5
        periodSeconds: 30
      readinessProbe:
        httpGet:
          path: /health
          port: health
        initialDelaySeconds: 2
        periodSeconds: 10
```

### External monitoring

Point any HTTP-based monitoring tool (Uptime Robot, Pingdom, Datadog, etc.) at:

```text
http://your-server:8080/health
```

A `200` response confirms the service is running. Any other status or a connection failure indicates a problem.

---

## Architecture

The health check server runs as its own actor (`HealthState`) in the `acton-reactive` framework with a `Permanent` restart policy. It operates independently from the SMTP server: if the health check server encounters an error, it can restart without affecting email processing.

The server uses `hyper` for HTTP handling and `tokio::select!` for graceful shutdown via a cancellation token.
