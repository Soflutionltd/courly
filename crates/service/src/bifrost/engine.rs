use std::sync::Arc;
use std::time::Duration;

use bifrost_net::BandwidthMeter;
use bifrost_sync::{EngineConfig, Error, SyncEngine};

use super::SqliteCheckpointStore;

/// Ceiling for a quiet folder's adaptive poll interval. bifrost's default
/// lets an untouched folder drift to 30 minutes; a message moved into it
/// from Outlook would then take that long to appear. The system folders and
/// the folder on screen are re-read every few seconds by the resident
/// engine's hot cadence, so this only bounds the rest.
const POLL_MAX: Duration = Duration::from_secs(120);

/// Per-account change ring. The hot cadence publishes about two events a
/// second on a Graph account while the resident consumer may spend minutes
/// hydrating one backfill page with its attachments; bifrost's default of 256
/// lags within two minutes, and every lag abandons outstanding pages and
/// reopens settled backfill walks, which feeds the next lag.
const CHANGES_CAPACITY: usize = 4096;

#[derive(Clone)]
pub struct BifrostSyncEngine {
    engine: Arc<SyncEngine>,
    checkpoints: Arc<SqliteCheckpointStore>,
}

impl BifrostSyncEngine {
    pub fn build(
        checkpoints: SqliteCheckpointStore,
        bandwidth: Option<Arc<BandwidthMeter>>,
    ) -> Result<Self, Error> {
        let checkpoint_handle = Arc::new(checkpoints.clone());
        let mut config = EngineConfig::default();
        config.multiplexer.poll_max = POLL_MAX;
        config.multiplexer.changes_capacity = CHANGES_CAPACITY;
        let mut builder = SyncEngine::builder()
            .checkpoints(checkpoint_handle)
            .config(config);
        if let Some(meter) = bandwidth {
            builder = builder.with_bandwidth_meter(meter);
        }
        Ok(Self {
            engine: Arc::new(builder.build()?),
            checkpoints: Arc::new(checkpoints),
        })
    }

    #[must_use]
    pub fn engine(&self) -> Arc<SyncEngine> {
        Arc::clone(&self.engine)
    }

    #[must_use]
    pub fn checkpoints(&self) -> Arc<SqliteCheckpointStore> {
        Arc::clone(&self.checkpoints)
    }
}
