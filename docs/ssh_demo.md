# demo.hotpath.rs setup

## Overview

This server runs two SSH services:
- **Port 22**: `tuihost` - public demo access, no authentication required using https://github.com/pawurb/tuihost-rs
- **Port 2222**: `sshd-admin` - admin access with key-based authentication

Users connect with `ssh demo.hotpath.rs` (any or no username) and get the hotpath console TUI.

## Services

### tuihost (Port 22)

Custom SSH server that ignores usernames and spawns a forced TUI command.

**Service file**: `/etc/systemd/system/tuihost.service`
```ini
[Unit]
Description=TUI SSH Host for Hotpath Console
After=network.target

[Service]
Type=simple
User=tuihost
Group=tuihost
ExecStart=/usr/local/bin/tuihost -l 0.0.0.0:22 -k /etc/tuihost/host_key -c /usr/local/bin/hotpath-wrapper.sh -e HOTPATH_EXCLUDE_WRAPPER=true -e HOTPATH_FUNCTIONS_LIMIT=15 --max-connections 100 --timeout 600 --max-session-duration 600
Environment=NO_COLOR=1
Restart=always
RestartSec=5

# Resource ceilings (one session is ~35M RSS, ~60 threads, ~35 fds)
MemoryHigh=1536M
MemoryMax=2G
MemorySwapMax=0
OOMPolicy=continue
CPUQuota=200%
TasksMax=4096
LimitNOFILE=65536

# Sandboxing
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes

# Allow binding to privileged port 22
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
```

**Config files**:
- Host key: `/etc/tuihost/host_key` (Ed25519, owned by tuihost:tuihost)
- Binaries: `/usr/local/bin/tuihost`, `/usr/local/bin/hotpath`
- Wrapper: `/usr/local/bin/hotpath-wrapper.sh` (picks random metrics port per connection)

**Wrapper script** (`/usr/local/bin/hotpath-wrapper.sh`):
```bash
#!/bin/bash
# Wrapper for tuihost: picks a random metrics port and runs hotpath console
PORT=$((RANDOM % 16384 + 49152))
export HOTPATH_METRICS_PORT="$PORT"
export HOTPATH_DISABLE_SAMPLY_LOAD=1
export HOTPATH_SAMPLY_WRAPPER_BIN="/root/.cargo/bin/samply"
export HOTPATH_SAMPLY_BIN="/root/.cargo/bin/hotpath-samply"
exec /usr/local/bin/hotpath console --metrics-port "$PORT"
```

### sshd-admin (Port 2222)

Standard OpenSSH for admin access.

**Service file**: `/etc/systemd/system/sshd-admin.service`
```ini
[Unit]
Description=OpenSSH Admin Server (Port 2222)
After=network.target auditd.service
ConditionPathExists=!/etc/ssh/sshd_not_to_be_run

[Service]
EnvironmentFile=-/etc/default/ssh
ExecStartPre=/usr/sbin/sshd -t -f /etc/ssh/sshd_config_admin
ExecStart=/usr/sbin/sshd -D -f /etc/ssh/sshd_config_admin
ExecReload=/bin/kill -HUP $MAINPID
KillMode=process
Restart=on-failure
RestartPreventExitStatus=255
Type=notify
RuntimeDirectory=sshd-admin
RuntimeDirectoryMode=0755

[Install]
WantedBy=multi-user.target
```

**Config file**: `/etc/ssh/sshd_config_admin`

---

## Usage

### Public demo access
```bash
ssh demo.hotpath.rs
# or
ssh user@demo.hotpath.rs
# Both work - username is ignored
```

### Admin access
```bash
ssh -p 2222 root@demo.hotpath.rs
```
---

## Management

```bash
# Status
sudo systemctl status tuihost
sudo systemctl status sshd-admin

# Logs
sudo journalctl -u tuihost -f

# Restart
sudo systemctl restart tuihost

# Verify both services
sudo ss -tlnp | grep -E ':22 |:2222'
```

---

## Boot Configuration

### Enabled services
```bash
sudo systemctl is-enabled tuihost      # enabled
sudo systemctl is-enabled sshd-admin   # enabled
sudo systemctl is-enabled sshd         # disabled
```

### tmpfiles.d for /run/sshd

OpenSSH requires `/run/sshd` for privilege separation.

**File**: `/etc/tmpfiles.d/sshd.conf`
```
d /run/sshd 0755 root root -
```

---

## Security Hardening

### Dedicated service user
```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin tuihost
```

### File ownership
```bash
sudo chown tuihost:tuihost /etc/tuihost
sudo chmod 700 /etc/tuihost
sudo chown tuihost:tuihost /etc/tuihost/host_key
```

### Sandboxing options
| Option | Purpose |
|--------|---------|
| `User=tuihost` | Run as unprivileged user |
| `NoNewPrivileges=yes` | Prevent privilege escalation |
| `ProtectSystem=strict` | Mount /usr, /boot, /efi, /etc as read-only |
| `ProtectHome=yes` | Make /home, /root, /run/user inaccessible |
| `PrivateTmp=yes` | Isolated /tmp and /var/tmp |
| `PrivateDevices=yes` | No access to physical devices |
| `ProtectKernelTunables=yes` | /proc and /sys read-only |
| `ProtectControlGroups=yes` | /sys/fs/cgroup read-only |
| `RestrictNamespaces=yes` | Prevent namespace creation |
| `RestrictRealtime=yes` | Prevent realtime scheduling |
| `RestrictSUIDSGID=yes` | Prevent SUID/SGID file creation |
| `CAP_NET_BIND_SERVICE` | Only capability allowed (for port 22) |

---

## Rollback

To restore original OpenSSH on port 22:
```bash
sudo systemctl stop tuihost
sudo systemctl disable tuihost
sudo systemctl enable ssh
sudo systemctl start ssh
```

---

## Troubleshooting

### tuihost not starting
```bash
sudo journalctl -u tuihost -n 50
# Check if port 22 is in use:
sudo ss -tlnp | grep :22
```

### Can't connect on port 2222
```bash
ls -la /run/sshd
# If missing:
sudo mkdir -p /run/sshd && sudo chmod 755 /run/sshd
```

---

## Deploy Hotpath Binary


```bash
cd /root/hotpath-rs/crates/hotpath/
RUSTFLAGS="--cfg tokio_unstable" cargo install --path . --features='tui,hotpath,hotpath-alloc,demo'
cargo install --path . --bin hotpath-samply --features='hotpath'
```

and run:

```bash
cp /root/.cargo/bin/hotpath /usr/local/bin/hotpath && sudo systemctl restart tuihost
```
