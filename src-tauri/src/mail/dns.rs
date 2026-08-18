//! The one DNS query Skim makes: the MX lookup that tells onboarding whether a
//! domain's mail lives at Microsoft. It goes through the system resolver
//! (`DnsQuery_W`), so it uses the machine's own DNS servers and cache — the
//! address never reaches a third-party resolver, and split-horizon DNS on a
//! corporate VPN answers with the truth.

use std::ffi::c_void;
use windows::core::HSTRING;
use windows::Win32::NetworkManagement::Dns::{
    DnsFree, DnsFreeRecordList, DnsQuery_W, DNS_QUERY_STANDARD, DNS_RECORDA, DNS_RECORDW,
    DNS_TYPE_MX,
};

/// The MX exchange hosts of `domain`, or an empty vec on any failure — no
/// records, NXDOMAIN, no network are all the same answer to the caller.
///
/// Blocking: `DnsQuery_W` has no timeout of its own and a dead resolver can sit
/// on the call for seconds, so callers run this on `spawn_blocking` under a
/// `tokio::time::timeout`.
pub fn mx_hosts(domain: &str) -> Vec<String> {
    let name = HSTRING::from(domain);
    let mut head: *mut DNS_RECORDA = std::ptr::null_mut();
    // SAFETY: `name` is a NUL-terminated wide string that outlives the call, and
    // `head` is a valid out-pointer. On success the resolver hands back a list it
    // owns, which goes back to `DnsFree` below.
    let status = unsafe {
        DnsQuery_W(
            &name,
            DNS_TYPE_MX,
            DNS_QUERY_STANDARD,
            None,
            &mut head,
            None,
        )
    };
    if status.is_err() || head.is_null() {
        return Vec::new();
    }

    let mut hosts = Vec::new();
    // SAFETY: `head` is the resolver's singly linked list, valid until `DnsFree`.
    // The list can carry other record types (a CNAME at the head, say), so the
    // `MX` arm of the union is only read after `wType` says it is an MX record.
    // The `_W` query returns wide records; the out-parameter is typed `DNS_RECORDA`
    // for both charsets and the layouts are identical apart from the string type.
    unsafe {
        let mut rec = head.cast::<DNS_RECORDW>();
        while !rec.is_null() {
            if (*rec).wType == DNS_TYPE_MX.0 {
                let exchange = (*rec).Data.MX.pNameExchange;
                if !exchange.is_null() {
                    if let Ok(host) = exchange.to_string() {
                        hosts.push(host);
                    }
                }
            }
            rec = (*rec).pNext;
        }
        DnsFree(Some(head as *const c_void), DnsFreeRecordList);
    }
    hosts
}
