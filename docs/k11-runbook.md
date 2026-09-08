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
scripts — the public IPv6 endpoint below is the stable path (kept stable
across prefix rotations by DDNS, Phase 4.2), and
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
| Listening ports | `8022` (Termux sshd), `9091` TCP (harbor-server control plane) + `9091` UDP (STUN rendezvous **and** TURN relay, same port and same socket) |
| Supervisor | `supervise.sh` loop (30 s) via Termux:Boot; see the supervision section |

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

## Public endpoint stability: DDNS AAAA (Phase 4.2)

The carrier prefix rotates, so the literal IPv6 address eventually moves.
[`../server/k11/ddns-update.sh`](../server/k11/ddns-update.sh) keeps a
hostname's AAAA record pointed at the device's current global address; the
supervisor refreshes it every pass while the server is actually serving.

- Pick a provider that accepts a plain URL update (DuckDNS, deSEC, most
  dynamic-DNS services). The token lives in the URL you set — the script
  and the logs never carry it anywhere else.
- Configure in the supervisor's environment (e.g. a wrapper around
  `supervise.sh` in `~/.termux/boot`):
  ```sh
  HARBOR_DDNS_URL='https://example.duckdns.org/update?domains=example&token=SECRET&ipv6=@AAA@'
  # optional: pin the interface whose address should win
  # HARBOR_DDNS_IFACE=rmnet_data0
  ```
- Verify by hand first (`sh ~/harbor/ddns-update.sh` prints the outcome);
  afterwards `supervisor.log` records only ok/unchanged/failed lines.
- Until a hostname is configured, clients pin the literal address above; a
  later `server.configure` moves them with no protocol change.

Address-choice notes: the script prefers `HARBOR_DDNS_IFACE` when set,
else the kernel's default global source address. Android rotates privacy
extension addresses; if the record chases temporary addresses and clients
stale out between updates, set `HARBOR_DDNS_IFACE` to the interface
carrying the stable SLAAC address (compare `ip -6 addr` lifetimes).

## Post-reboot smoke test (Phase 4.2)

[`../server/k11/smoke.sh`](../server/k11/smoke.sh) probes, from the device
itself, the three legs a client needs:

```sh
sh ~/harbor/smoke.sh                       # localhost, pin from server.log
sh ~/harbor/smoke.sh <host> <sha256-hex>   # any reachable address
```

1. **TCP 9091** — the control plane answers TLS with the pinned certificate.
2. **UDP 9091** — STUN echoes the probe nonce with the observed source.
3. **UDP 9091** — TURN answers a bare Allocate with a `401` challenge,
   proving the relay state machine answers before any client credential
   exists.

Needs `openssl` and `nc` (`pkg install openssh openbsd-netcat`); a missing
tool SKIPs its leg loudly instead of passing silently. Run it after every
reboot, upgrade, or DDNS change — it is the fastest honest "the server is
really serving" check available without a second device.

## Supervision: automatic restart (2026-09-05)

Android kills background processes and the Wi-Fi lease moves, so the
previous fire-and-forget `nohup` (plus a watchdog that itself died)
left the server down for ~12 h (log stopped 2026-09-04 22:51, found dead
2026-09-05). The repo now carries the supervision that fixed it, in
[`../server/k11/`](../server/k11/):

- `supervise.sh` — the supervisor loop (30 s): keeps `harbor-server` alive
  (TCP **and** the UDP 9091 side — the datagram leg dying with the process
  up is treated as a restart), rotates logs past 5 MiB, holds a wake lock
  when Termux:API is present, and refreshes the DDNS AAAA each pass when
  `HARBOR_DDNS_URL` is set. Idempotent: healthy listeners are never touched.
- `run-server.sh` — canonical idempotent start. Binds **`[::]:9091`**
  (the old on-device copy bound `0.0.0.0:9091`, silently dropping the
  public IPv6 endpoint); a live-but-portless wedged process is stopped
  before rebinding.
- `boot-harbor` — Termux:Boot entry (`~/.termux/boot/harbor`, needs the
  Termux:Boot app) so supervision returns after a device reboot.
- `find-k11.sh` — operator-side LAN discovery: probes ARP neighbors for
  the Termux sshd banner and the pinned 9091 certificate, printing the
  K11+'s current DHCP address. No nmap, no root.
- `smoke.sh` — the post-reboot probe above; also useful after upgrades.

Health checks match **LISTEN state `0A`** in `/proc/net/tcp*`, never a
bare `:2383` match: a dying process leaves CLOSE/TIME_WAIT rows that a
naive grep mistakes for a live listener (this exact false-positive hid
an outage during the 2026-09-05 kill-test and is fixed in both scripts).

Recovery validation (2026-09-05, after the outage above):

1. `find-k11.sh` reported `192.168.1.7  K11+ (pin match)` (identity also
   confirmed by MAC `38:30:f9:3e:de:81` matching the documented IPv6 EUI-64).
2. Supervisor deployed and started: `harbor-server` restarted, LAN and
   public IPv6 pins verified `b984...7271f`.
3. Kill-test: SIGTERM to `harbor-server` → supervisor logged
   `(re)starting` on the next cycle → port back to `0A`, pin correct.
4. Full pairing E2E through the public IPv6 endpoint passed in 0.92 s.

Remaining honest limits: a supervisor that Android itself kills cannot
restart anything until the next boot entry (Termux:Boot) or manual start

— the wake lock only lowers the odds. After any reboot, confirm with
`smoke.sh`; after any reconnect, clients pinned to the DDNS hostname need
nothing (the name follows the address), literal-IPv6 users re-pin once.

## Latest field validation (2026-09-07)

The current K11+ deployment was upgraded over Termux SSH on port 8022 with the
ARMv7 Android build and the current Phase 4 scripts. Validation evidence:

- `smoke.sh localhost <fingerprint>` passed TCP certificate pinning, STUN-lite
  UDP echo, and unauthenticated TURN Allocate `401`.
- TCP and UDP 9091 were both bound on `[::]:9091`.
- `tailscaled` was stopped; no Tailscale process or listener remained on the
  K11+ during validation.
- Killing `harbor-server` caused the new supervisor to restart it; TCP+UDP and
  the full smoke test passed again after recovery.
- LAN STUN observed the client as `::ffff:192.168.1.4`; the global IPv6 TCP
  endpoint served the pinned certificate.

Still unproven: Android media calls, TURN allocation with minted credentials,
CGNAT/4G traversal, background/suspend behavior, and the complete Gate 3
four-path matrix.

## Operational limits observed on this device

- **Supervision, not invincibility**: `supervise.sh` + the Termux:Boot
  entry restart the server after kills and drops (see above), but if
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
