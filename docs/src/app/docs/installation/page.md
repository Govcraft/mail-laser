---
title: Installation
nextjs:
  metadata:
    title: Installation
    description: Install MailLaser using Docker, pre-compiled binaries, Nix, or by building from source.
---

MailLaser provides multiple installation methods. Docker is recommended for production deployments. Pre-compiled binaries work well for quick evaluation, and building from source gives you full control.

---

## Docker (recommended)

Pull the official image from GitHub Container Registry:

```shell
docker pull ghcr.io/govcraft/mail-laser:latest
```

To pin a specific version:

```shell
docker pull ghcr.io/govcraft/mail-laser:v2.0.0
```

Run the container with your configuration:

```shell
docker run -d \
  --name mail-laser \
  -p 2525:2525 \
  -p 8080:8080 \
  -e MAIL_LASER_TARGET_EMAILS="alerts@example.com,support@example.com" \
  -e MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
  --restart unless-stopped \
  ghcr.io/govcraft/mail-laser:latest
```

The Docker image is built from `scratch` with a statically-linked musl binary, resulting in a minimal image that contains only the binary and CA certificates for HTTPS.

{% callout title="Bind addresses in Docker" %}
When running in Docker, leave `MAIL_LASER_BIND_ADDRESS` and `MAIL_LASER_HEALTH_BIND_ADDRESS` at their default value of `0.0.0.0`. Port mapping (`-p`) handles external access. Changing the bind address inside the container will prevent Docker port forwarding from working.
{% /callout %}

---

## Pre-compiled binaries

Download the binary for your platform from the [GitHub Releases page](https://github.com/Govcraft/mail-laser/releases).

Available platforms:

- `mail_laser-linux-x86_64` -- Linux (x86_64)
- `mail_laser-macos-x86_64` -- macOS (Intel)
- `mail_laser-macos-aarch64` -- macOS (Apple Silicon)
- `mail_laser-windows-x86_64.exe` -- Windows (x86_64)

### Linux and macOS

```shell
chmod +x ./mail_laser-linux-x86_64

MAIL_LASER_TARGET_EMAILS="alerts@example.com" \
MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
./mail_laser-linux-x86_64
```

### Windows (PowerShell)

```powershell
$env:MAIL_LASER_TARGET_EMAILS = "alerts@example.com"
$env:MAIL_LASER_WEBHOOK_URL = "https://your-api.com/webhook"
.\mail_laser-windows-x86_64.exe
```

You can also place configuration in a `.env` file in the same directory as the binary. See [Configuration](/docs/configuration) for details.

---

## Nix

MailLaser includes a `flake.nix` for reproducible development environments. Enter the development shell with all necessary tools:

```shell
git clone https://github.com/Govcraft/mail-laser.git
cd mail-laser
nix develop
```

The Nix shell provides the Rust stable toolchain, OpenSSL, and `swaks` for testing.

---

## Build from source

Building from source requires the Rust toolchain (1.70.0 or later). Install it from [rustup.rs](https://rustup.rs/).

```shell
git clone https://github.com/Govcraft/mail-laser.git
cd mail-laser
cargo build --release
```

The compiled binary is at `target/release/mail_laser`. Run it directly:

```shell
MAIL_LASER_TARGET_EMAILS="alerts@example.com" \
MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
./target/release/mail-laser
```

### Release profile

The release build is optimized for binary size:

- `opt-level = "z"` -- Aggressive size optimization
- `lto = true` -- Link-time optimization across crates
- `codegen-units = 1` -- Single codegen unit for better optimization
- `strip = true` -- Debug symbols removed

---

## Verify the installation

Regardless of installation method, verify that MailLaser is running by checking the health endpoint:

```shell
curl http://localhost:8080/health
```

A `200 OK` response with an empty body confirms the service is operational. Any other path returns `404 Not Found`.
