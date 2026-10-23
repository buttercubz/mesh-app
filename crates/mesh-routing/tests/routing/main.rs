//! mesh-routing integration tests (WP-09 acceptance): pruning equivalence, floor
//! contention with per-link caps, floor convergence/safety/liveness incl. a mid-burst
//! join, and the cap steady state of the pacing loop.

mod cap_steady;
mod floor_props;
mod net;
mod pruning;
mod talker;
