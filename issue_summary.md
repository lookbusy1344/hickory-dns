hickory-resolver still loses IPv6 scope information for scoped nameservers on `main` (`d54a46c8`).

The original macOS whole-config failure is fixed on `main`. The remaining defect is that hickory still models nameserver addresses as `IpAddr`, so a scoped IPv6 resolver such as `fe80::1%en0` cannot carry its zone all the way to the socket. That makes the link-local resolver unreachable on any platform.

```rust
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

fn main() {
    let server = hickory_resolver::config::NameServerConfig::udp_and_tcp(
        IpAddr::from_str("fe80::1").unwrap(),
    );

    let SocketAddr::V6(v6) = SocketAddr::new(server.ip, 53) else {
        unreachable!();
    };

    assert_eq!(v6.scope_id(), 0);
}
```

The bug is the `0`: a scoped IPv6 resolver needs a nonzero scope id to be reachable on a host with more than one interface. `IpAddr` has no place to store that scope, so hickory cannot preserve it with the current representation.
