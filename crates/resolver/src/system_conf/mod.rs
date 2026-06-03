// Copyright 2015-2017 Benjamin Fry <benjaminfry@me.com>
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// https://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! System configuration loading
//!
//! This module is responsible for parsing and returning the configuration from
//!  the host system. It will read from the default location on each operating
//!  system, e.g. most Unixes have this written to `/etc/resolv.conf`
#![allow(missing_docs)]

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
#[cfg(feature = "system-config")]
mod unix;

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
#[cfg(feature = "system-config")]
pub use self::unix::{parse_resolv_conf, read_system_conf};

#[cfg(windows)]
#[cfg(feature = "system-config")]
mod windows;

#[cfg(target_os = "windows")]
#[cfg(feature = "system-config")]
pub use self::windows::read_system_conf;

#[cfg(target_os = "android")]
#[cfg(feature = "system-config")]
mod android;

#[cfg(target_os = "android")]
#[cfg(feature = "system-config")]
pub use self::android::read_system_conf;

#[cfg(target_vendor = "apple")]
#[cfg(feature = "system-config")]
mod apple;

#[cfg(target_vendor = "apple")]
#[cfg(feature = "system-config")]
pub use self::apple::read_system_conf;

/// Resolves an IPv6 zone (the `%zone` suffix of an address such as `fe80::1%en0`) to a
/// numeric scope id suitable for a [`SocketAddrV6`](std::net::SocketAddrV6).
///
/// A numeric zone is the scope id directly; an interface name is resolved via
/// `if_nametoindex`. Returns `None` if an interface name does not resolve to an index.
#[cfg(all(feature = "system-config", unix, not(target_os = "android")))]
fn scope_id_from_zone(zone: &str) -> Option<u32> {
    if let Ok(scope_id) = zone.parse::<u32>() {
        return Some(scope_id);
    }

    let name = std::ffi::CString::new(zone).ok()?;
    // SAFETY: `name` is a valid NUL-terminated C string that outlives the call, and
    // `if_nametoindex` only reads from it.
    let scope_id = unsafe { libc::if_nametoindex(name.as_ptr()) };
    // `if_nametoindex` returns 0 to signal an unknown interface.
    (scope_id != 0).then_some(scope_id)
}
