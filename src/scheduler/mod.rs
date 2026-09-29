//! Job scheduling.
//!
//! The OS-cron based scheduler was removed as part of the prune to the Pi-host
//! goal. The database-driven loop over `jobs` is rebuilt here.
