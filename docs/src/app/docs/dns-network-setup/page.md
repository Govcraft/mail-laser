---
title: DNS and network setup
nextjs:
  metadata:
    title: DNS and network setup
    description: Configure DNS MX records, firewalls, and port forwarding to receive email from the internet with MailLaser.
---

To receive emails from external senders (not just local testing), you need to configure DNS records and network rules so that mail destined for your domain reaches the MailLaser server.

---

## DNS configuration

### MX record

Create an MX (Mail Exchanger) record for the domain or subdomain that receives email. This tells other mail servers where to deliver messages addressed to that domain.

```text
example.com.    IN    MX    10    mail.example.com.
mail.example.com.    IN    A    203.0.113.50
```

In this example:

- Emails sent to `*@example.com` are directed to `mail.example.com`
- `mail.example.com` resolves to the public IP of your MailLaser server
- The priority `10` is the standard value when you have a single mail server

If MailLaser handles a subdomain only:

```text
hooks.example.com.    IN    MX    10    mail.example.com.
```

Emails to `*@hooks.example.com` are directed to MailLaser, while `*@example.com` continues to go to your primary mail provider.

---

## Port configuration

MailLaser listens on port 2525 by default. Internet SMTP traffic uses port 25. You have two options:

### Option A: Run on port 25

Set `MAIL_LASER_PORT=25` and run with appropriate privileges. On Linux, binding to ports below 1024 requires root or the `CAP_NET_BIND_SERVICE` capability.

```shell
docker run -d \
  --name mail-laser \
  -p 25:25 \
  -e MAIL_LASER_PORT=25 \
  -e MAIL_LASER_TARGET_EMAILS="alerts@example.com" \
  -e MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
  ghcr.io/govcraft/mail-laser:latest
```

With Docker, the container process does not need special privileges because Docker handles the port mapping at the host level.

### Option B: Port forwarding

Keep MailLaser on port 2525 and forward external port 25 traffic to it:

```shell
# Using iptables
iptables -t nat -A PREROUTING -p tcp --dport 25 -j REDIRECT --to-port 2525

# Using Docker port mapping
docker run -d -p 25:2525 ...
```

The Docker port mapping approach (`-p 25:2525`) maps host port 25 to container port 2525, which is the simplest option for Docker deployments.

---

## Firewall rules

Ensure the SMTP port is open for inbound TCP connections on your server.

### UFW (Ubuntu/Debian)

```shell
# If using port 25
sudo ufw allow 25/tcp

# If using port 2525
sudo ufw allow 2525/tcp
```

### Cloud provider security groups

On AWS, GCP, or Azure, add an inbound rule to your security group:

| Type | Protocol | Port | Source |
|------|----------|------|--------|
| Custom TCP | TCP | 25 (or 2525) | 0.0.0.0/0 |

{% callout type="warning" title="Port 25 restrictions" %}
Many cloud providers block outbound port 25 by default to prevent spam. However, inbound port 25 is usually available. Check your provider's documentation. AWS, for example, requires a specific request to unblock outbound port 25 but allows inbound freely.
{% /callout %}

---

## NAT and port forwarding

If your MailLaser server is behind a NAT router (common in home networks or some hosting setups), configure port forwarding on the router:

| External port | Internal IP | Internal port |
|---------------|-------------|---------------|
| 25 | 192.168.1.100 | 2525 |

Replace `192.168.1.100` with the private IP of your MailLaser server.

---

## Verifying your setup

After configuring DNS and network rules, verify the complete chain:

### 1. Check DNS resolution

```shell
dig MX example.com
# Should show your mail server's hostname

dig A mail.example.com
# Should show your server's public IP
```

### 2. Test SMTP connectivity

From an external machine:

```shell
swaks \
  --to alerts@example.com \
  --from test@sender.com \
  --server mail.example.com:25 \
  --header "Subject: External test" \
  --body "Testing from the internet"
```

### 3. Verify webhook delivery

Check your webhook endpoint's logs to confirm it received the JSON payload.

---

## Security considerations

MailLaser does not implement SMTP authentication. When exposed to the internet, consider these measures:

- **Recipient validation** is your first line of defense. Only emails addressed to your configured `MAIL_LASER_TARGET_EMAILS` are processed. All others are rejected with a 550 response.
- **Rate limiting** at the network level (e.g., iptables, cloud provider rules) can mitigate abuse.
- **Reverse proxy** such as HAProxy or nginx can add connection limits, IP allowlists, or TLS termination in front of MailLaser.
- **Monitoring** the webhook actor's failure metrics helps detect abuse patterns. Watch for unusual spikes in dropped or failed deliveries.

For internal-only deployments, bind MailLaser to a private interface (`MAIL_LASER_BIND_ADDRESS=10.0.0.5`) and use firewall rules to restrict access to trusted networks.
