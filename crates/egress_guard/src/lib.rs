//! Process-wide allowlist for outbound network connections.
//!
//! Builds that must never contact a fixed third-party server (LocalShell) call [`restrict`] at
//! startup. From then on, only origins registered through [`set_allowed_urls`] — the AI endpoint
//! the user configured — may be contacted. Every network layer (HTTP client, DNS resolution,
//! websockets) consults this module, so a code path that was missed during the fork still fails
//! closed instead of leaking data.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::SystemTime;

use url::{Host, Origin, Url};

/// The number of blocked attempts kept in memory for [`recent_blocked_attempts`].
const MAX_RECORDED_ATTEMPTS: usize = 200;

static RESTRICTED: AtomicBool = AtomicBool::new(false);
static ALLOWED_ORIGINS: RwLock<Vec<Origin>> = RwLock::new(Vec::new());
static BLOCKED_ATTEMPTS: Mutex<VecDeque<BlockedAttempt>> = Mutex::new(VecDeque::new());

/// An outbound connection that was refused because its target is not allowlisted.
#[derive(Debug, Clone, thiserror::Error)]
#[error(
    "Blocked by LocalShell network policy: {target} is not the configured AI endpoint, so no \
     connection was made"
)]
pub struct Blocked {
    /// The origin (scheme, host and port) or host name that was refused. Paths and query strings
    /// are deliberately dropped so that the record never holds request content.
    pub target: String,
}

/// A record of a refused connection, kept so users and auditors can see what was stopped.
#[derive(Debug, Clone)]
pub struct BlockedAttempt {
    pub target: String,
    pub at: SystemTime,
}

/// Switches the process into fail-closed mode: nothing may be contacted until an origin is
/// allowlisted. This cannot be undone for the lifetime of the process.
pub fn restrict() {
    RESTRICTED.store(true, Ordering::SeqCst);
}

pub fn is_restricted() -> bool {
    RESTRICTED.load(Ordering::SeqCst)
}

/// Replaces the allowlist with the origins of `urls`. URLs with opaque origins (e.g. `data:`) are
/// ignored because they can never be connected to.
pub fn set_allowed_urls<'a>(urls: impl IntoIterator<Item = &'a Url>) {
    let origins = urls
        .into_iter()
        .map(Url::origin)
        .filter(Origin::is_tuple)
        .collect();
    *ALLOWED_ORIGINS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = origins;
}

/// Returns the currently allowlisted origins, serialized as `scheme://host[:port]`.
pub fn allowed_origins() -> Vec<String> {
    read_allowed_origins()
        .iter()
        .map(Origin::ascii_serialization)
        .collect()
}

pub fn is_url_allowed(url: &Url) -> bool {
    if !is_restricted() {
        return true;
    }
    let origin = url.origin();
    read_allowed_origins().contains(&origin)
}

/// Host-level check for layers that only see a host name, such as DNS resolution. Any port on an
/// allowlisted host passes here; the stricter origin check happens in [`check_url`].
pub fn is_host_allowed(host: &str) -> bool {
    if !is_restricted() {
        return true;
    }
    read_allowed_origins().iter().any(|origin| match origin {
        Origin::Tuple(_, allowed_host, _) => host_matches(allowed_host, host),
        Origin::Opaque(_) => false,
    })
}

/// Returns an error, and records the attempt, if `url` may not be contacted.
pub fn check_url(url: &Url) -> Result<(), Blocked> {
    if is_url_allowed(url) {
        return Ok(());
    }
    let target = match url.origin() {
        origin @ Origin::Tuple(..) => origin.ascii_serialization(),
        Origin::Opaque(_) => url.scheme().to_owned(),
    };
    Err(record_blocked(target))
}

/// Returns an error, and records the attempt, if `host` may not be resolved or connected to.
pub fn check_host(host: &str) -> Result<(), Blocked> {
    if is_host_allowed(host) {
        return Ok(());
    }
    Err(record_blocked(host.to_owned()))
}

/// Returns the most recent refused connections, oldest first.
pub fn recent_blocked_attempts() -> Vec<BlockedAttempt> {
    BLOCKED_ATTEMPTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .cloned()
        .collect()
}

fn record_blocked(target: String) -> Blocked {
    log::warn!("egress_guard: blocked outbound connection to {target}");
    let mut attempts = BLOCKED_ATTEMPTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if attempts.len() == MAX_RECORDED_ATTEMPTS {
        attempts.pop_front();
    }
    attempts.push_back(BlockedAttempt {
        target: target.clone(),
        at: SystemTime::now(),
    });
    Blocked { target }
}

fn read_allowed_origins() -> std::sync::RwLockReadGuard<'static, Vec<Origin>> {
    ALLOWED_ORIGINS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn host_matches(allowed: &Host<String>, host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match allowed {
        Host::Domain(domain) => domain.eq_ignore_ascii_case(host),
        Host::Ipv4(ip) => ip.to_string() == host,
        Host::Ipv6(ip) => ip.to_string() == host,
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
