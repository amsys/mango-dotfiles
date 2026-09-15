# ufw firewall — inbound, host-wide

## Scope

This installer closes inbound access to this host, on every interface,
except the paths VAULT.md names as needed: the Hetzner WireGuard tunnel
(`wg_hetzner`), the mango hotspot (`p2p0`), and the docker bridges. Those
paths keep their own ufw rules, added by `system/remote/install.sh` and
`system/libvirt-net/install.sh`, plus the `172.16.0.0/12` rules for
postgres and redis already in `/etc/ufw/user.rules`.

## Why ufw owns inbound, and vpnguard owns egress

Two problems, two tools, one rule set each. ufw filters packets that
arrive at this host. `system/vpnguard/` controls packets that leave it,
through a WireGuard fail-closed design. A single tool that tried to do
both would need to track direction on every rule. Splitting the job
keeps each rule set small and easy to read. Do not move an inbound rule
into vpnguard's config, and do not move an egress rule into ufw's.

## Docker publishes bypass ufw

A container that publishes a port with `-p 0.0.0.0:PORT:PORT` (or a
compose file that does the same) writes into the `nat` table directly,
ahead of ufw's own `INPUT` chain. `ufw status` does not show that port,
and a `ufw deny` rule does not block it.

`docker-daemon.json` sets the daemon's default publish IP to
`127.0.0.1`. A container that publishes a port with no host IP named
then lands on localhost, not on every interface. A container (or
compose file) that names `0.0.0.0` on purpose still bypasses ufw — fix
that at the container, not here.

## What this installer does

Read `install.sh`, or run it with `--print`, for the exact commands. In
short: it sets the default input policy to deny, the output and forward
policies to allow, opens the LM Studio proxy port (`1234/tcp`) to the
docker bridge range only, installs `docker-daemon.json`, restarts
docker, and turns ufw on. It then checks that the `wg_hetzner` and
`p2p0` VNC rules and the docker postgres/redis rule are still in `ufw
status verbose`, and that no `docker-proxy` process listens on
`0.0.0.0` or `[::]`.

## Refuses to run when

`/lib/modules/$(uname -r)` is missing. A `pacman` kernel upgrade can
remove the old kernel's module directory without a reboot. `ufw enable`
then fails to load the netfilter modules it needs — see
`firewall.md`'s Incident log for the exact errors. Reboot into the
kernel that matches `/lib/modules/`, then run this installer.

## Rollback

Printed after every run. See `install.sh`'s Rollback block.
