# Harbor Server on the LG K11+ — Runbook

The Harbor Server (control plane only) is deployed on the LG K11+ and serves
paired Harbor devices on the local network. This runbook records the facts of
that deployment and how to operate it. It deliberately contains **no
credentials**: SSH access to the device is key-based and out of scope here, and
the certificate fingerprint below is public pinning material, not a secret.

## Device facts (observed, not assumed)

The K11+ gets a **dynamic LAN lease (DHCP)**: its `192.168.1.x` address
changes whenever the Wi-Fi drops and returns (`.6` at deploy time, `.7`
since 2026-09-05). Never hardcode the LAN address into clients or
scripts — the Tailnet endpoint below is the stable one, and
[`../server/k11/find-k11.sh`](../server/k11/find-k11.sh) re-discovers the
current LAN address by TLS pin when SSH access is needed.

| Fact | Value |
| --- | --- |
| Hardware | LG K11+ |
| Architecture | `armv7l` (32-bit ARM) |
| OS | Android 7.1.2 (API 25), kernel 3.18.35 |
| Runtime | Termux (`HOME=/data/data/com.termux/files/home`) |
| CPU / RAM | 3 cores, ~2.8 GiB total |
| Storage at deploy | ~20 GB free |
| Listening ports | `8022` (Termux sshd), `9091` (harbor-server) |
| Supervisor | none — Termux has no systemd; `nohup` + log + manual restart |

## Deployment layout (on the device)

```text
$HOME/harbor/
  harbor-server          # binary, mode 700, cross-compiled, stripped (~1.1 MB)
  supervise.sh           # supervisor loop (from server/k11/, 0700)
  run-server.sh          # idempotent start, bind [::]:9091 (from server/k11/)
  supervisor.log         # timestamped restarts/rotations (append, rotated)
  server.log             # stdout+stderr of the running server (append, rotated)
  state/
    tls/cert.pem         # self-signed ECDSA P-256 identity, 0600
    tls/key.pem          # TLS private key — never leaves the device, 0600
    state/control-state-v1.json   # durable control-plane metadata only, 0600
$HOME/.termux/boot/
  harbor                 # Termux:Boot entry relaunching supervision (from server/k11/boot-harbor)
```

The server persists **only** registered identities and accepted pairing
relationships (the deployment snapshot is a few hundred bytes). Pairing codes,
presence leases and logical sessions are transient by design: a restart never
resurrects them.

## Build and upgrade (from the development machine)

There is no Rust toolchain on the device; the binary is cross-compiled with the
Android NDK (bionic, API 24 — the device is API 25) and pushed over SSH:

```sh
export NDK="$HOME/Android/Sdk/ndk/28.2.13676358"
export TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC_armv7_linux_androideabi="$TOOLCHAIN/armv7a-linux-androideabi24-clang"
export AR_armv7_linux_androideabi="$TOOLCHAIN/llvm-ar"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$TOOLCHAIN/armv7a-linux-androideabi24-clang"

CARGO_TARGET_DIR=<repo>/build/cargo-target \
  cargo build --release -p harbor-server --target armv7-linux-androideabi

"$TOOLCHAIN/llvm-strip" \
  <repo>/build/cargo-target/armv7-linux-androideabi/release/harbor-server
```

Push and restart (the exact SSH address/port is a deployment decision recorded
by the operator; this runbook does not carry it):

```sh
scp -P <ssh-port> <binary> <user>@<host>:harbor/harbor-server.new
ssh -p <ssh-port> <user>@<host> '
  mv $HOME/harbor/harbor-server.new $HOME/harbor/harbor-server
  chmod 700 $HOME/harbor/harbor-server
  pkill -x harbor-server
  HARBOR_SERVER_BIND="[::]:9091" \
  HARBOR_SERVER_STATE_DIR=$HOME/harbor/state \
    nohup $HOME/harbor/harbor-server >> $HOME/harbor/server.log 2>&1 &
'
```

After an upgrade, verify the startup fingerprint (below) — the persisted TLS
identity means an upgrade keeps the same fingerprint and pinned clients keep
trusting the server.

### Killing the process on this device: use /proc, not pgrep/pkill

`pgrep`/`pkill` (including `-x`) are **unreliable on this Termux/Android 7
build** and were observed both missing a live `harbor-server` process
("tudo morto" while it still held the port) and matching the caller's own
shell with `-f`. The trustworthy procedure is procfs directly:

```sh
ssh -p <ssh-port> <user>@<host> '
  # find the pid by exact binary path
  pid=""; for p in /proc/[0-9]*; do
    case "$(cat $p/cmdline 2>/dev/null | tr "\0" " ")" in
      "$HOME/harbor/harbor-server"*) pid="${p#/proc/}"; break;;
    esac
  done
  # stop it, then wait for the port (9091 = hex 2383) to actually free.
  # Check both tables: the dual-stack socket lives in tcp6.
  [ -n "$pid" ] && kill "$pid"
  for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
    grep -q ":2383" /proc/net/tcp /proc/net/tcp6 2>/dev/null || break; sleep 1
  done
  # only then start the new instance — an early start dies with EADDRINUSE
  HARBOR_SERVER_BIND="[::]:9091" \
  HARBOR_SERVER_STATE_DIR=$HOME/harbor/state \
    nohup $HOME/harbor/harbor-server >> $HOME/harbor/server.log 2>&1 &
'
```

Two failure modes observed first-hand during deployment: `pkill -f` matching
the remote shell's own command line (killing the session mid-restart), and a
restart racing the old listener's teardown — the new instance hit
`Address already in use` and exited, leaving nothing running. Always wait for
`:2383` to leave `/proc/net/tcp` before starting.

## Start / stop / health

```sh
# start (dual-stack: serves LAN IPv4 and global IPv6 from one socket)
ssh -p <ssh-port> <user>@<host> '
  HARBOR_SERVER_BIND="[::]:9091" \
  HARBOR_SERVER_STATE_DIR=$HOME/harbor/state \
    nohup $HOME/harbor/harbor-server >> $HOME/harbor/server.log 2>&1 &'

# health: process alive, port listening, startup line in the log
ssh -p <ssh-port> <user>@<host> '
  for p in /proc/[0-9]*; do
    case "$(cat $p/cmdline 2>/dev/null | tr "\0" " ")" in
      "$HOME/harbor/harbor-server"*) echo "pid ${p#/proc/}";;
    esac
  done
  grep -q ":2383" /proc/net/tcp /proc/net/tcp6 2>/dev/null && echo "port 9091: LISTEN"
  tail -2 $HOME/harbor/server.log'
```

The startup line is the authoritative fingerprint source:

```text
harbor-server: control-plane listener on 0.0.0.0:9091 (protocol v1, certificate sha256:b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f)
```

This fingerprint survived a kill+restart unchanged during deployment
validation (the TLS identity is durable by design). Config is environment-only
(`HARBOR_SERVER_BIND`, `HARBOR_SERVER_STATE_DIR`, optional cert overrides — see
[`control-protocol-v1.md`](control-protocol-v1.md)); the deployment pins the
first two explicitly.

## Post-deploy validation checklist

1. **TLS handshake from a LAN machine** — TLS 1.3 must negotiate and the
   SHA-256 of the served certificate DER must equal the startup fingerprint:
   ```sh
   echo | openssl s_client -connect <k11-ip>:9091 -showcerts 2>/dev/null \
     | openssl x509 -outform der | sha256sum
   ```
   (`Verification error: self-signed certificate` from `openssl` is expected —
   trust is by fingerprint pinning, not a CA chain.)
2. **Pinning is enforced** — a core configured with a wrong fingerprint must be
   refused with `server certificate does not match the pinned fingerprint`
   (`error.server.unavailable`, retryable).
3. **Real pairing through the device** — two `harbor-core` processes (host and
   peer) pin the K11+ fingerprint, run `pairing.create` → `pairing.submit` →
   `pairing.incoming`/`pairing.accept` → `pairing.status` and both observe
   `ACCEPTED`. The traffic on the wire is signed control-plane envelopes only.
4. **Durable state stays minimal** — `state/control-state-v1.json` holds only
   registered identities and the accepted relationship, mode 0600.

## Public endpoint (IPv6 primary; IPv4 forwarding closed)

The K11+ serves external clients over its native IPv6 address. The server
binds dual-stack (`HARBOR_SERVER_BIND=[::]:9091` in production) and the
pinned fingerprint is transport-agnostic, so LAN IPv4, IPv6, and any future
hostname all share one pin and one pairing flow.

- Internal: `<current-DHCP>:9091` on the LAN (was `192.168.1.6:9091` at
  deploy, `192.168.1.7:9091` since 2026-09-05 — served by the same
  dual-stack socket; re-discover with `server/k11/find-k11.sh`)
- Public IPv6 (stable, EUI-64-derived — re-check with `ip -o addr show
  wlan0` if clients stop connecting, and prefer it over the temporary
  privacy address sharing the `/64`):
  `[2804:d59:8777:ad00:3a30:f9ff:fe3e:de81]:9091`
- Pinned fingerprint (unchanged, public pinning material):
  `b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f`

Rules that stay in force on the public surface:

- **No IPv4 port forwarding is expected to work.** The ISP NAT sits at
  `100.85.222.175` (CGNAT range): a WAN→LAN forward on the local router
  can never deliver traffic, which an independent external TCP check
  confirmed (`179.252.119.55:9091` times out from the real internet while
  the identical E2E passes in under a second on any working path). Do not
  chase the IPv4 rule further.
- **SSH (`8022`) is LAN-only and is never forwarded.** Verified: the public
  IPv4 refuses `8022` while the LAN SSH session that operates this device
  is unaffected.
- **The server remains control plane only.** Voice, chat, files, and the
  public-profile sync all travel peer-to-peer over WebRTC/DataChannels once
  signaling succeeds; reaching the server over IPv6 changes nothing about
  that architecture.
- **No Cloudflare Tunnel in front.** A `https://…cfargotunnel.com` URL
  cannot serve this protocol: the listener speaks raw TLS frames
  (length-prefixed JSON), not HTTP, so an HTTP-ingress tunnel has nothing
  to translate. TCP-mode tunneling would additionally require a client-side
  daemon on every Harbor device, which the architecture does not assume.
  Native IPv6 is the relay-free path; if a network has neither IPv6 nor a
  working forward, that network cannot reach this control plane by design.
- Clients point at the endpoint through the `server.configure` channel
  (`{"address": "<host>:9091", "fingerprint": "<64 hex>"}` — a bracketed
  IPv6 literal such as `[2804:…]:9091` is accepted); the product UI
  deliberately never shows addresses, fingerprints, or TLS details.
  Pairing itself is unchanged: six-digit code, five-minute
  expiry, explicit accept. A hostname with both AAAA and A records gives
  clients automatic family fallback through normal resolution.

Validation status (2026-09-03, after rebinding `[::]:9091` with zero
downtime beyond the restart and an unchanged fingerprint):

1. **Dual-stack socket**: `/proc/net/tcp6` shows `:::9091` in LISTEN;
   `/proc/net/tcp` has no separate entry (one v6 socket serves both).
2. **IPv6 handshake from a LAN machine** — the SHA-256 of the served
   certificate DER equals the startup fingerprint:
   ```sh
   echo | openssl s_client \
     -connect "[2804:d59:8777:ad00:3a30:f9ff:fe3e:de81]:9091" \
     -showcerts 2>/dev/null \
     | openssl x509 -outform der | sha256sum
   # b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f
   ```
3. **Full pairing E2E over IPv6** (0.88 s): wrong-pin refusal first, then
   `pairing.create` → `pairing.submit` → `pairing.incoming`/`pairing.accept`
   → `pairing.status` = `ACCEPTED` on both sides plus `contacts.list`,
   all through the bracketed IPv6 endpoint:
   ```sh
   HARBOR_E2E_ADDRESS="[2804:d59:8777:ad00:3a30:f9ff:fe3e:de81]:9091" \
   HARBOR_E2E_FINGERPRINT="b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f" \
     cargo test -p harbor-core --lib \
       pairing_and_contacts_succeed_against_the_deployed_control_plane
   ```
4. **IPv4 regression**: the same E2E against the then-current LAN address
   still passes — IPv4-mapped clients on a dual-stack Linux socket keep
   working. (The address itself moves with DHCP; re-discover, don't pin it.)
5. Note: from inside the LAN, the public IPv4 was never expected to work
   unless the router does NAT loopback — validate externally (mobile data
   with the openssl command above, substituting nothing: the address is
   already global), not from the LAN.

## Tailnet endpoint (works behind CGNAT, no router rules)

Since the carrier NAT makes IPv4 forwarding impossible, the production
client path is Tailscale: a private WireGuard mesh where every device gets
a stable `100.x.y.z` address reachable from any network. No protocol
change was needed — the Tailnet address goes in the same server field with
the same pinned fingerprint, and pairing, signaling, voice, chat, files,
and profile sync behave identically.

- K11+ Tailnet address: `100.114.220.46` (hostname `localhost-0`;
  re-check with `~/tailscale_1.102.3_arm/tailscale
  --socket=$HOME/tailscale.sock status` if clients stop connecting)
- Client endpoint: `100.114.220.46:9091`
- Fingerprint: unchanged,
  `b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f`

### K11+ setup (Termux, Android 7 — official app needs newer Android)

The static `tailscaled` runs in userspace-networking mode (no TUN needed):

```sh
ssh -p 8022 joshy@<k11-lan-ip> '
  ~/tailscale_1.102.3_arm/tailscaled --tun=userspace-networking \
    --statedir=$HOME/tailscale-state --socket=$HOME/tailscale.sock \
    >> $HOME/tailscale.log 2>&1 &
  ~/tailscale_1.102.3_arm/tailscale --socket=$HOME/tailscale.sock up'
# approve the printed login.tailscale.com URL in a browser, then:
ssh -p 8022 joshy@<k11-lan-ip> '
  ~/tailscale_1.102.3_arm/tailscale --socket=$HOME/tailscale.sock status'
```

Then, in the admin console (`login.tailscale.com/admin/machines`):
disable **key expiry** for the K11+ node, or it silently deauthenticates
one day. Exempt Termux from battery optimization like `harbor-server`
itself; after a reboot both daemons need the same start commands.

### Validation (2026-09-03, all through the Tailnet)

1. **Mesh**: `tailscale ping` K11+ answers direct (129 ms, IPv6 underlay).
2. **TLS + pin**:
   ```sh
   echo | openssl s_client -connect 100.114.220.46:9091 -showcerts 2>/dev/null \
     | openssl x509 -outform der | sha256sum
   # b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f
   ```
3. **Full pairing E2E** (1.04 s — wrong-pin refusal, pairing both sides
   `ACCEPTED`, `contacts.list` with no keys):
   ```sh
   HARBOR_E2E_ADDRESS="100.114.220.46:9091" \
   HARBOR_E2E_FINGERPRINT="b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f" \
     cargo test -p harbor-core --lib \
       pairing_and_contacts_succeed_against_the_deployed_control_plane
   ```
4. **Client switch**: the desktop app's pin was moved from
   `192.168.1.6:9091` to `100.114.220.46:9091` via `server.configure`
   (backup at `/tmp/opencode/harbor-state-backup`); same server process,
   so identities and relationships persist untouched.

Security posture improves over a public forward: nothing is exposed to
the internet at all (Tailnet-only), SSH stays LAN-only, and the pin still
authenticates the server independently of the transport.

### Desktop auto-join (zero-config installs)

Since Harbor 2.1.x the desktop joins the Tailnet by itself on startup
(`native/HarborTailnet.*`) and pins the server endpoint automatically
(`HarborFacade::ensureDefaultServer`, same defaults the mobile client
ships). A fresh install pairs with no manual setup; an existing pin or an
already-connected personal Tailscale login is never touched.

Admin prerequisites (one time, then §rotation only):

1. Tag the K11+ machine `tag:harbor-server` (Machines → `localhost-0`).
2. Minimal ACL — the containment for the extractable client key
   (grants syntax, as the console saves it). Tagged nodes are NOT members,
   so owner devices need an explicit grant to reach the tagged server —
   without the member→server grant both netmaps come back empty:
   ```json
   "grants": [
   	{
   		"src": ["tag:harbor-client"],
   		"dst": ["tag:harbor-server"],
   		"ip":  ["tcp:9091"],
   	},
   	{
   		"src": ["autogroup:member"],
   		"dst": ["tag:harbor-server"],
   		"ip":  ["tcp:9091"],
   	},
   	{
   		"src": ["autogroup:member"],
   		"dst": ["autogroup:member"],
   		"ip":  ["*"],
   	},
   	{
   		"src": ["tag:harbor-client"],
   		"dst": ["tag:harbor-client"],
   		"ip":  ["*"],
   	},
   	{
   		"src": ["tag:harbor-client"],
   		"dst": ["autogroup:member"],
   		"ip":  ["*"],
   	},
   	{
   		"src": ["autogroup:member"],
   		"dst": ["tag:harbor-client"],
   		"ip":  ["*"],
   	},
   ],
   ```
   The last three exist for the P2P media plane (voice/chat/files travel
   device-to-device over Tailnet IPs): without them pairing completes but
   calls never connect audio. Owner-only installs work with the first two
   grants; all six are required once keyed (ephemeral-client) builds ship.
3. Settings → Keys → Generate auth key: Reusable ✓, Ephemeral ✓ (the node
   vanishes when Harbor closes), Tags: `harbor-client`, shortest expiry for
   the first test, 90 days for production.
4. GitHub → repo → Settings → Secrets → Actions → `HARBOR_TAILSCALE_AUTHKEY`.
   The key is compiled into Linux/Windows release binaries only; PR/fork
   builds stay inert, and it never appears in logs, QML, or argv (staged in
   a 0600 temp file as `--auth-key=file:PATH`, removed right after use;
   outputs scrubbed to `tskey-<redacted>`).

Per-machine one-time notes (desktop joins by itself after these):

- Linux: Harbor performs the one-time setup itself — distro client install
  (native package on Arch, vendor script elsewhere), daemon start, first
  join and the operator grant — behind a single system-password dialog
  (`native/HarborTailnet::installClient`). If it reports `needs-admin`
  instead, run once: `sudo tailscale up --operator=$USER` and relaunch.
- Windows: install with `harbor-windows-setup.exe` (ships the VC++ 2022
  runtime and the Tailscale 1.102.3 client, both hash-pinned in
  `packaging/windows/`). Harbor joins on first launch with its embedded
  key; the Tailscale tray app stays as the manual fallback.
- The K11+ server itself is unaffected (manual `up` in Termux, as above).

Rotation (before key expiry — set a reminder, installs stop joining after):

```sh
# 1. Generate the replacement key (same flags/tags as above).
# 2. Update the HARBOR_TAILSCALE_AUTHKEY secret.
# 3. Tag + release; the mandatory updater retires old builds with the dead key.
```

Kill-switch (suspected key abuse):

```sh
# 1. Admin console → Settings → Keys → revoke the key (joins stop at once;
#    existing ephemeral nodes vanish as they go offline).
# 2. Machines → remove unknown harbor-* ephemeral nodes.
# 3. Rotate per §rotation. Pairing crypto is unaffected: no code + accept,
#    no relationship — the key alone never grants a pairing.
```

## Supervision: automatic restart (2026-09-05)

Android kills background processes and the Wi-Fi lease moves, so the
previous fire-and-forget `nohup` (plus a watchdog that itself died)
left the server down for ~12 h (log stopped 2026-09-04 22:51, found dead
2026-09-05). The repo now carries the supervision that fixed it, in
[`../server/k11/`](../server/k11/):

- `supervise.sh` — the supervisor loop (30 s): keeps `tailscaled` and
  `harbor-server` alive, rotates logs past 5 MiB, holds a wake lock when
  Termux:API is present. Idempotent: healthy listeners are never touched.
- `run-server.sh` — canonical idempotent start. Binds **`[::]:9091`**
  (the old on-device copy bound `0.0.0.0:9091`, silently dropping the
  public IPv6 endpoint); a live-but-portless wedged process is stopped
  before rebinding.
- `boot-harbor` — Termux:Boot entry (`~/.termux/boot/harbor`, needs the
  Termux:Boot app) so supervision returns after a device reboot.
- `find-k11.sh` — operator-side LAN discovery: probes ARP neighbors for
  the Termux sshd banner and the pinned 9091 certificate, printing the
  K11+'s current DHCP address. No nmap, no root.

Health checks match **LISTEN state `0A`** in `/proc/net/tcp*`, never a
bare `:2383` match: a dying process leaves CLOSE/TIME_WAIT rows that a
naive grep mistakes for a live listener (this exact false-positive hid
an outage during the 2026-09-05 kill-test and is fixed in both scripts).

Recovery validation (2026-09-05, after the outage above):

1. `find-k11.sh` reported `192.168.1.7  K11+ (pin match)` (identity also
   confirmed by MAC `38:30:f9:3e:de:81` matching the documented IPv6 EUI-64).
2. Supervisor deployed and started: `tailscaled` + `harbor-server`
   restarted, LAN + Tailnet pins verified `b984...7271f`, mesh pong 18 ms.
3. Kill-test: SIGTERM to `harbor-server` → supervisor logged
   `(re)starting` on the next cycle → port back to `0A`, pin correct.
4. Full pairing E2E through the Tailnet passed in 0.92 s.

Remaining honest limits: a supervisor that Android itself kills cannot
restart anything until the next boot entry (Termux:Boot) or manual start
— the wake lock only lowers the odds. After any reboot, confirm with the
health commands above; after any reconnect, clients on the Tailnet need
nothing (the address is stable), LAN-only users re-discover.

## Operational limits observed on this device

- **Supervision, not invincibility**: `supervise.sh` + the Termux:Boot
  entry restart both daemons after kills and drops (see above), but if
  Android kills the supervisor itself between boot entries, nothing
  restarts until the next boot or manual start. If the port stops
  answering, SSH to the current LAN address (`find-k11.sh`) and check
  `supervisor.log`; durable state and the TLS identity survive every
  restart path.
- **Clock discipline**: the server rejects requests with timestamps more than
  ±300 s from its own clock (`stale_timestamp`, retryable). If clients see
  retryable failures across the board, check the device clock first —
  deployment validation measured the device and desktop clocks in sync to the
  second.
- **Bind address**: `[::]:9091` is intentional — one dual-stack socket serves
  LAN IPv4 (mapped) and global IPv6 together, which is what makes the
  public endpoint work without any relay. The surface is control-plane only (allowlisted signed
  envelopes, 256 KiB frame cap, 8 connections, 30 s idle timeout); there is no
  media, chat, file or DataChannel path through this listener.
