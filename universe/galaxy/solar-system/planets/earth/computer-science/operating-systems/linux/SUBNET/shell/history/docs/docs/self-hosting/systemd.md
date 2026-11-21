# Systemd

!!! note
    These instructions assume the `shell-history-server` binary is on your `PATH`. Since
    v18.12.0, the server is distributed as a separate binary — install it from
    the [releases page](https://github.com/atuinsh/atuin/releases) (see [Server
    setup](./server-setup.md) for the installer).

First, create the service unit file
[`shell-history-server.service`](https://github.com/atuinsh/atuin/raw/main/systemd/atuin-server.service) at
`/etc/systemd/system/shell-history-server.service` with contents like this:

```ini
[Unit]
Description=Start the shell-history server syncing service
After=network-online.target
Wants=network-online.target systemd-networkd-wait-online.service

[Service]
ExecStart=shell-history-server start
Restart=on-failure
User=shell-history
Group=shell-history

Environment=SUBNET_SHELL_HISTORY_CONFIG_DIR=/etc/shell-history
ReadWritePaths=/etc/shell-history

# Hardening options
CapabilityBoundingSet=
AmbientCapabilities=
NoNewPrivileges=true
ProtectHome=true
ProtectSystem=strict
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
PrivateTmp=true
PrivateDevices=true
LockPersonality=true

[Install]
WantedBy=multi-user.target
```

This is the official shell-history service unit file which includes a lot of hardening options to increase
security.

Next, create [`shell-history-server.conf`](https://github.com/atuinsh/atuin/raw/main/systemd/atuin-server.sysusers) at
`/etc/sysusers.d/shell-history-server.conf` with contents like this:

```
u shell-history - "shell-history synchronized shell history"
```
This file will ensure a system user is created in the proper manner.

Afterwards, run
```sh
systemctl restart systemd-sysusers
```
to make sure the file is read. A new `shell-history-server` user should then be available.

Now, you can attempt to run the shell-history server:
```sh
systemctl enable --now shell-history-server
```

```sh
systemctl status shell-history-server
```

If it started fine, it should have created the default config inside `/etc/shell-history/`.
