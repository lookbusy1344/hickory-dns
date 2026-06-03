# hickory-resolver discards working DNS configuration over a scoped IPv6 nameserver

**Tracking:** [hickory-dns/hickory-dns#3713](https://github.com/hickory-dns/hickory-dns/issues/3713).
**Versions:** `hickory-resolver 0.26.1`, `resolv-conf 0.7.6`.

---

## 1. The malfunction in one paragraph

When a system's DNS configuration lists an IPv6 link-local nameserver with a zone
identifier — e.g. `fe80::1%en0`, which is how a router commonly advertises itself as the
IPv6 resolver — `hickory-resolver` cannot use it, because it has nowhere to store the
zone id. That much is a missing feature. The actual malfunction is the blast radius: on
macOS, `read_system_conf()` treats the unparseable entry as fatal and returns `Err` for
the **entire** configuration, so every other nameserver in the list — including a
perfectly good IPv4 one — is thrown away with it. A resolver built from that result can
resolve nothing, even though the machine has a usable nameserver and a healthy network.

This is not a parsing nicety. It is a **type-level gap**: hickory models a nameserver
address as `std::net::IpAddr`, and `IpAddr` has no concept of an IPv6 scope.

---

## 2. Two distinct defects, one root cause

### Defect A — hickory cannot represent or reach a scoped nameserver

The nameserver address is a bare `IpAddr`:

```rust
// src/config.rs:196
pub struct NameServerConfig {
    pub ip: IpAddr,            // no room for an IPv6 zone/scope id
    ...
}
```

and the connect address is built from it like this:

```rust
// src/connection_provider.rs:81
let remote_addr = SocketAddr::new(ip, config.port);
```

`SocketAddr::new(IpAddr::V6(..), port)` always produces a `SocketAddrV6` with
**`scope_id = 0`**. A scope only exists on `SocketAddrV6`; it has no home on
`Ipv6Addr`/`IpAddr`. So even if the zone id reached this point, there is no field to put
it in and no code path that would carry it to the socket. Dialling a `fe80::…` address
with scope 0 on a multi-interface host fails with `EINVAL` / "no route". hickory therefore
cannot talk to a link-local nameserver at all.

### Defect B — `read_system_conf()` fails the whole load over one bad entry (macOS)

On Apple targets the system-config reader parses each `ServerAddresses` entry and bails on
the first failure:

```rust
// src/system_conf/apple.rs:36
let addr = IpAddr::from_str(&Cow::from(&*n))
    .map_err(|e| format!("failed to parse nameserver address: {e}"))?;   // `?` fails the WHOLE read
nameservers.push(NameServerConfig::udp_and_tcp(addr));
```

`IpAddr::from_str("fe80::1%en0")` returns `invalid IP address syntax`, the `?` propagates,
and `read_system_conf()` returns `Err` for the entire configuration — discarding any
usable nameserver that appeared elsewhere in the same list. One unrepresentable entry
destroys the whole config instead of being skipped.

### Why they share a root cause

Both follow from `NameServerConfig.ip: IpAddr` having no scope field: (A) a scoped address
cannot be stored or dialled, and (B) the parser's only option for a scoped string is to
reject it. Fix the representation and both become fixable.

### The same root, three different symptoms by platform

- **macOS (`apple.rs`):** hard `Err` on the whole read — the loud, total failure above.
- **Linux (`unix.rs`):** parses via `resolv_conf`, which *keeps* the scope as
  `ScopedIp::V6(addr, Some(zone))`, but hickory then does `ip.into(): IpAddr` and
  **silently drops the zone**, leaving a dead scope-0 `fe80::1` in the config. No error —
  just a nameserver that never answers.
- **Windows:** the Win32 sources return already-parsed `IpAddr` values one at a time, so
  there is no `%zone` string to choke on and no all-or-nothing failure — but a link-local
  entry is still unusable (Defect A) if it is the only server.

So the configuration-destroying behaviour is macOS-specific, but the inability to *use* a
scoped nameserver is universal, and the Linux path quietly produces a dead config from the
same root.

---

## 3. What hickory needs to change

Two independent changes. **B is the cheap mitigation; A is the real fix.**

### Change A — carry the IPv6 scope id end-to-end

1. **Representation.** Give `NameServerConfig` somewhere to hold a scope — either an
   address type that admits a `scope_id`, or an explicit `scope_id: Option<u32>`
   (meaningful only for IPv6 link-local). The bare-`IpAddr` public field is the blocker
   and the crux of #3713.

2. **Connect path.** In `connection_provider.rs`, when the address is IPv6 and a scope is
   present, build the socket as `SocketAddrV6::new(addr, config.port, 0, scope_id)` instead
   of `SocketAddr::new(ip, config.port)`, so the kernel knows which interface to use.

3. **Parsing.** In the `system_conf` readers, parse the `%zone` suffix into a numeric scope
   id — numeric zones directly, interface names via `libc::if_nametoindex` — and populate
   the new field. On Linux this means *using* the `ScopedIp::V6(_, Some(zone))` that
   `resolv_conf` already provides instead of discarding it.

### Change B — make `read_system_conf()` tolerant of entries it cannot use

Independent of A, the readers should **skip** an entry they cannot represent and keep the
rest, rather than failing the whole load:

```rust
// apple.rs, instead of `?`:
let addr = match IpAddr::from_str(&Cow::from(&*n)) {
    Ok(addr) => addr,
    Err(e) => { warn!("skipping unparseable nameserver {n:?}: {e}"); continue; }
};
```

This alone prevents the total failure: the scoped entry is skipped, the usable IPv4
nameserver is kept. It does **not** let hickory use a link-local server (that needs A), but
one unrepresentable entry stops taking the whole configuration down with it. The Linux path
should likewise prefer "skip and keep usable" over "silently keep a dead scope-0 entry."

---

## 4. Test case (reproduced on macOS)

Precondition: the host's active DNS list leads with a scoped link-local server. This is the
default on a typical Mac whose router advertises itself over IPv6 — check with
`scutil --dns`, which shows e.g.:

```
nameserver[0] : fe80::1%en0      # router-advertised IPv6 resolver, with zone id
nameserver[1] : 192.168.1.254    # usable IPv4 resolver
```

Self-contained probe (hickory + std only — needs `hickory-resolver` with the
`system-config` feature):

```rust
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

fn main() {
    // (1) The zone id cannot be parsed into an IpAddr — deterministic, any platform.
    println!("{:?}", IpAddr::from_str("fe80::1%en0"));
    //        => Err(AddrParseError(Ip))

    // (2) A scope cannot survive in an IpAddr/SocketAddr built this way — any platform.
    let SocketAddr::V6(v6) = SocketAddr::new(IpAddr::from_str("fe80::1").unwrap(), 53)
    else { unreachable!() };
    println!("{}", v6.scope_id());   // => 0   (the scope is gone)

    // (3) The whole system config is discarded — macOS, with a scoped entry present.
    println!("{:?}", hickory_resolver::system_conf::read_system_conf());
    //        => Err(Msg("failed to parse nameserver address: invalid IP address syntax"))
}
```

Verified output on an affected macOS host (Darwin 25.5, `hickory-resolver 0.26.1`):

```
Err(AddrParseError(Ip))
0
Err(Msg("failed to parse nameserver address: invalid IP address syntax"))
```

Assertions (1) and (2) are deterministic and environment-free — they hold on any OS and
make good CI unit tests for the type-level gap (Defect A). Assertion (3) is the
configuration-destroying failure (Defect B): note the result is `Err` for the *entire*
config, so `192.168.1.254` — which resolves correctly when queried directly — is thrown
away along with the scoped entry. hickory discards working state.

> **Run it as a normal process.** `read_system_conf()` on macOS reads the System
> Configuration dynamic store; a process that cannot reach that store (e.g. a sandbox that
> blocks it) returns a *different, misleading* error,
> `Err(Msg("failed to access System Configuration dynamic store"))`, which is **not** this
> bug. Probes (1) and (2) are unaffected. To observe (3), run the probe unsandboxed on the
> affected host.

---

## 5. Summary

| | Today (0.26.1) | After Change A | After Change B |
|---|---|---|---|
| Store a scoped nameserver | impossible (`IpAddr`, no scope) | yes | still impossible |
| Dial a link-local server | impossible (scope 0) | yes | no |
| One bad entry kills the whole config (macOS) | yes | yes (parse still strict) | no — skipped |
| Only a link-local resolver on the LAN | dead | works | still dead (needs A) |
| A usable IPv4 server alongside the scoped one | dead (macOS) | works | works |

**Minimum to stop discarding working configuration:** Change B.
**Correct, complete fix:** Change A — the substance of issue #3713.

---

## 6. Reproduction status on this branch (2026-06-03)

The body above describes `hickory-resolver 0.26.1`. Reproduced locally against the
in-tree `0.27.0-alpha.1` (`scoped-nameserver-addrs`), the picture has shifted:

- **Change B has already landed here** — commit `c5e29b9da` ("skip unparseable
  nameservers on macOS instead of failing the whole load"). `apple.rs` now matches on
  `IpAddr::from_str`, `warn!`s, and `continue`s instead of `?`-propagating. So **probe
  (3) no longer reproduces the documented `Err`** — on an affected host it returns
  `Ok(..)` carrying the usable IPv4 server, with the scoped entry skipped. The "macOS"
  row of the §5 table no longer applies to this tree.

- **Defect A is unchanged and still reproduces.** `config.rs` still has `pub ip:
  IpAddr` and `connection_provider.rs` still builds `SocketAddr::new(ip, config.port)`.
  Probes (1) and (2) hold verbatim. A scoped nameserver still cannot be stored or
  dialled. This is the outstanding work for #3713.

- The Linux silent-zone-drop (`unix.rs`, `ip.into(): IpAddr` on a
  `ScopedIp::V6(_, Some(zone))`) is likewise still present.

A self-contained probe covering all of the above lives at
`crates/resolver/examples/scoped_nameserver_repro.rs` (run unsandboxed for the macOS
path; the System Configuration store is unreachable from a sandbox and returns the
misleading "failed to access System Configuration dynamic store" noted in §4).
