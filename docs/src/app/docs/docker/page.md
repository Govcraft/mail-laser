---
title: Docker
nextjs:
  metadata:
    title: Docker deployment
    description: Deploy MailLaser with Docker, including Docker Compose examples and production configuration.
---

Docker is the recommended way to deploy MailLaser in production. The official image is minimal, secure, and available from GitHub Container Registry.

---

## Image details

| Property | Value |
|----------|-------|
| Registry | `ghcr.io/govcraft/mail-laser` |
| Base image | `scratch` (empty) |
| Binary | Statically linked with musl |
| Included extras | CA certificates for HTTPS |
| Architecture | `x86_64` |

The multi-stage Dockerfile produces an image that contains only the compiled binary and root CA certificates. No shell, no package manager, no runtime dependencies.

---

## Basic usage

```shell
docker pull ghcr.io/govcraft/mail-laser:latest

docker run -d \
  --name mail-laser \
  -p 2525:2525 \
  -p 8080:8080 \
  -e MAIL_LASER_TARGET_EMAILS="alerts@example.com" \
  -e MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
  --restart unless-stopped \
  ghcr.io/govcraft/mail-laser:latest
```

---

## Docker Compose

For more complex setups, use Docker Compose. This example includes log configuration and resilience tuning:

```yaml
# docker-compose.yml
services:
  mail-laser:
    image: ghcr.io/govcraft/mail-laser:latest
    container_name: mail-laser
    restart: unless-stopped
    ports:
      - "2525:2525"   # SMTP
      - "8080:8080"   # Health check
    environment:
      MAIL_LASER_TARGET_EMAILS: "alerts@example.com,support@example.com"
      MAIL_LASER_WEBHOOK_URL: "https://your-api.com/webhook"
      MAIL_LASER_WEBHOOK_TIMEOUT: "15"
      MAIL_LASER_WEBHOOK_MAX_RETRIES: "5"
      MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD: "10"
      MAIL_LASER_CIRCUIT_BREAKER_RESET: "120"
      RUST_LOG: "info"
```

Start with:

```shell
docker compose up -d
```

### Using an env file

Keep configuration separate from the Compose file:

```yaml
# docker-compose.yml
services:
  mail-laser:
    image: ghcr.io/govcraft/mail-laser:latest
    container_name: mail-laser
    restart: unless-stopped
    ports:
      - "2525:2525"
      - "8080:8080"
    env_file:
      - mail-laser.env
```

```shell
# mail-laser.env
MAIL_LASER_TARGET_EMAILS=alerts@example.com,support@example.com
MAIL_LASER_WEBHOOK_URL=https://your-api.com/webhook
MAIL_LASER_HEADER_PREFIX=X-Custom,X-My-App
RUST_LOG=info
```

---

## Kubernetes

Deploy MailLaser as a Kubernetes Deployment with a Service for SMTP access and health probes:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: mail-laser
spec:
  replicas: 1
  selector:
    matchLabels:
      app: mail-laser
  template:
    metadata:
      labels:
        app: mail-laser
    spec:
      containers:
        - name: mail-laser
          image: ghcr.io/govcraft/mail-laser:latest
          ports:
            - containerPort: 2525
              name: smtp
            - containerPort: 8080
              name: health
          env:
            - name: MAIL_LASER_TARGET_EMAILS
              value: "alerts@example.com"
            - name: MAIL_LASER_WEBHOOK_URL
              value: "https://your-api.com/webhook"
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
          resources:
            requests:
              memory: "16Mi"
              cpu: "50m"
            limits:
              memory: "64Mi"
              cpu: "200m"
---
apiVersion: v1
kind: Service
metadata:
  name: mail-laser
spec:
  selector:
    app: mail-laser
  ports:
    - name: smtp
      port: 2525
      targetPort: smtp
    - name: health
      port: 8080
      targetPort: health
```

{% callout title="Resource requests" %}
MailLaser is lightweight. The memory and CPU requests shown above are conservative starting points. Monitor actual usage and adjust based on your email volume.
{% /callout %}

---

## Viewing logs

```shell
# Follow logs in real time
docker logs -f mail-laser

# Show last 100 lines
docker logs --tail 100 mail-laser
```

Set `RUST_LOG=debug` for detailed output during troubleshooting, or `RUST_LOG=mail_laser::webhook=debug` to focus on webhook delivery diagnostics.

---

## Building the image locally

To build the Docker image from source:

```shell
git clone https://github.com/Govcraft/mail-laser.git
cd mail-laser
docker build -t mail-laser:local .
```

The Dockerfile uses a multi-stage build:

1. **Builder stage**: Uses `rust:slim`, adds the `x86_64-unknown-linux-musl` target, and compiles a statically-linked binary.
2. **Final stage**: Copies only the binary and CA certificates into a `scratch` image.

The build caches Cargo dependencies in a separate layer, so rebuilds after source changes are fast.
