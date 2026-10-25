//! Registry change hub (ADR-007): when sessions and terminals change, `/v1/events` subscribers get a new snapshot.
//! Sources: `touch()` from `Session::emit`, session and terminal creation/removal; the snapshot is built on demand
//! with a 250 ms debounce and is sent only if it differs from the previous one.
use std::sync::OnceLock;
use tokio::sync::broadcast;

static CHANGED: OnceLock<broadcast::Sender<()>> = OnceLock::new();

fn chan() -> &'static broadcast::Sender<()> { CHANGED.get_or_init(|| broadcast::channel(64).0) }

/// Something in the registry changed (cheap, no allocations; with no subscribers nothing
/// happens).
pub fn touch() { let _ = chan().send(()); }

pub fn subscribe() -> broadcast::Receiver<()> { chan().subscribe() }
