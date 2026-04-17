//! Per-source-IP concurrent connection cap for the SMTP listener.
//!
//! Sits between `TcpListener::accept()` and the session spawn: callers
//! `try_acquire(peer_ip)` and either receive an [`IpConnGuard`] that they
//! move into the spawned task, or `None` when the configured cap is already
//! reached. Dropping the guard decrements the count and reclaims the map
//! slot when it reaches zero, so RAII cleanup survives panics and early
//! returns in the session handler.
//!
//! The cap is the coarsest useful DoS defense for the post-DMARC
//! authorization flow: without it, a single abusive client could keep many
//! sessions open streaming up to `max_message_size_bytes` each before the
//! end-of-DATA Cedar check rejects them.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

/// Tracks live connection counts per peer IP.
///
/// Cheap to clone — internal state is one `Arc<Mutex<…>>`.
#[derive(Clone)]
pub struct IpLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, u32>>>,
    max_per_ip: u32,
}

impl IpLimiter {
    /// Creates a limiter with the given cap. `max_per_ip == 0` disables the
    /// limiter entirely — every `try_acquire` returns a no-op guard.
    pub fn new(max_per_ip: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_per_ip,
        }
    }

    /// Attempts to reserve a connection slot for `ip`. Returns `None` when
    /// the cap is already reached.
    pub fn try_acquire(&self, ip: IpAddr) -> Option<IpConnGuard> {
        if self.max_per_ip == 0 {
            return Some(IpConnGuard {
                limiter: None,
                ip,
            });
        }
        let mut map = self.inner.lock().expect("ip-limiter mutex poisoned");
        let entry = map.entry(ip).or_insert(0);
        if *entry >= self.max_per_ip {
            None
        } else {
            *entry += 1;
            Some(IpConnGuard {
                limiter: Some(self.inner.clone()),
                ip,
            })
        }
    }
}

/// RAII token released when the session ends.
///
/// Holding `Option<Arc<…>>` rather than `&IpLimiter` keeps the guard
/// `'static`, which is what `tokio::spawn` requires.
pub struct IpConnGuard {
    limiter: Option<Arc<Mutex<HashMap<IpAddr, u32>>>>,
    ip: IpAddr,
}

impl Drop for IpConnGuard {
    fn drop(&mut self) {
        let Some(limiter) = self.limiter.as_ref() else {
            return;
        };
        let Ok(mut map) = limiter.lock() else {
            return;
        };
        if let Some(count) = map.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn cap_zero_disables_limiter() {
        let lim = IpLimiter::new(0);
        let peer = ip(127, 0, 0, 1);
        let guards: Vec<_> = (0..1000).map(|_| lim.try_acquire(peer).expect("disabled")).collect();
        assert_eq!(guards.len(), 1000);
    }

    #[test]
    fn refuses_beyond_cap() {
        let lim = IpLimiter::new(2);
        let peer = ip(10, 0, 0, 1);
        let g1 = lim.try_acquire(peer).unwrap();
        let g2 = lim.try_acquire(peer).unwrap();
        assert!(lim.try_acquire(peer).is_none(), "third should be refused");
        drop(g1);
        let g3 = lim.try_acquire(peer).expect("slot freed after drop");
        drop((g2, g3));
    }

    #[test]
    fn separate_ips_have_independent_counts() {
        let lim = IpLimiter::new(1);
        let a = ip(10, 0, 0, 1);
        let b = ip(10, 0, 0, 2);
        let _ga = lim.try_acquire(a).unwrap();
        let _gb = lim.try_acquire(b).unwrap();
        assert!(lim.try_acquire(a).is_none());
        assert!(lim.try_acquire(b).is_none());
    }

    #[test]
    fn guard_removes_map_entry_when_count_returns_to_zero() {
        let lim = IpLimiter::new(3);
        let peer = ip(192, 168, 0, 1);
        {
            let _g = lim.try_acquire(peer).unwrap();
            assert_eq!(lim.inner.lock().unwrap().get(&peer).copied(), Some(1));
        }
        assert!(
            lim.inner.lock().unwrap().get(&peer).is_none(),
            "zeroed entries must be removed to bound memory"
        );
    }
}
