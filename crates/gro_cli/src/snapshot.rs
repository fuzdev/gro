//! The startup snapshot this binary embeds, when built with the `snapshot`
//! feature, and `make-snapshot`, the internal subcommand that writes it.

use std::path::Path;

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::snapshot::StartupSnapshot;

/// The snapshot this binary was built with: `snapshot.bin` and `residual.rs`
/// from `fgro make-snapshot`, in the directory `GRO_SNAPSHOT_DIR` names.
#[cfg(feature = "snapshot")]
mod embedded {
    pub static SNAPSHOT: &[u8] = include_bytes!(concat!(env!("GRO_SNAPSHOT_DIR"), "/snapshot.bin"));
    include!(concat!(env!("GRO_SNAPSHOT_DIR"), "/residual.rs"));
}

#[cfg(feature = "snapshot")]
pub const STARTUP_SNAPSHOT: Option<StartupSnapshot> = Some(StartupSnapshot {
    bytes: embedded::SNAPSHOT,
    residual_lazy_js: embedded::RESIDUAL_LAZY_JS,
});

#[cfg(not(feature = "snapshot"))]
pub const STARTUP_SNAPSHOT: Option<StartupSnapshot> = None;

/// Writes `snapshot.bin` and `residual.rs` into `dir`, creating it first so an
/// unwritable directory fails before the snapshot is built.
pub fn make_snapshot(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    // the engine `fgro run` sets up
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    // the xtask's stamp decides when to rerun
    gro_runtime::snapshot::make_snapshot(engine)?.write_to(dir)?;
    Ok(())
}
