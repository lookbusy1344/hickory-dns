//! Reproduction probe for hickory-dns#3713:
//! "hickory-resolver discards working DNS configuration over a scoped IPv6 nameserver".
//!
//! Run with:
//!   cargo run -p hickory-resolver --example scoped_nameserver_repro --features system-config

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use hickory_resolver::config::NameServerConfig;

fn main() {
    // (1) Defect A, type-level gap: a zone id cannot be parsed into an IpAddr.
    //     Deterministic, environment-free, holds on any platform.
    let parsed = IpAddr::from_str("fe80::1%en0");
    println!("(1) IpAddr::from_str(\"fe80::1%en0\") = {parsed:?}");
    assert!(
        parsed.is_err(),
        "expected scoped address to fail IpAddr parse"
    );

    // (2) Defect A, type-level gap: a scope cannot survive in a SocketAddr built
    //     via SocketAddr::new(IpAddr::V6(..), port) — scope_id is forced to 0.
    let SocketAddr::V6(v6) = SocketAddr::new(IpAddr::from_str("fe80::1").unwrap(), 53) else {
        unreachable!()
    };
    println!(
        "(2) SocketAddr::new(fe80::1, 53).scope_id() = {}",
        v6.scope_id()
    );
    assert_eq!(v6.scope_id(), 0, "scope id should be silently lost");

    // (2b) NameServerConfig itself has nowhere to hold a scope: its address is a bare
    //      IpAddr. Even if we could parse the zone, there is no field for it.
    let ns = NameServerConfig::udp_and_tcp(IpAddr::from_str("fe80::1").unwrap()).with_scope_id(3);
    println!(
        "(2b) NameServerConfig.addr = {} -> socket {} (scope now carried)",
        ns.addr,
        ns.addr.socket_addr(53),
    );

    // (3) Linux silent-drop (Defect A symptom on the unix path): resolv_conf keeps the
    //     zone as ScopedIp::V6(addr, Some(zone)), but `.into(): IpAddr` discards it,
    //     leaving a dead scope-0 fe80::1 in the parsed config. No error is raised.
    #[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
    {
        match hickory_resolver::system_conf::parse_resolv_conf(
            "nameserver fe80::1%en0\nnameserver 192.168.1.254\n",
        ) {
            Ok((cfg, _)) => {
                println!("(3) parse_resolv_conf with a scoped nameserver succeeded:");
                for (i, ns) in cfg.name_servers().iter().enumerate() {
                    println!(
                        "      nameserver[{i}] = {}  (zone id dropped, unreachable if link-local)",
                        ns.ip
                    );
                }
            }
            Err(e) => println!("(3) parse_resolv_conf error: {e:?}"),
        }
    }

    // (4) macOS whole-config failure (Defect B). NOTE: in THIS repo apple.rs already
    //     skips unparseable entries (commit c5e29b9da), so this no longer returns Err
    //     for the whole config — the scoped entry is dropped and usable servers kept.
    #[cfg(target_vendor = "apple")]
    {
        println!(
            "(4) read_system_conf() = {:?}",
            hickory_resolver::system_conf::read_system_conf()
        );
    }
}
