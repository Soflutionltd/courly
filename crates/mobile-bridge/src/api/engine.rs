//! The Courly engine, run inside the app process.
//!
//! On the desktop the Service is a child process the app talks to over
//! stdin and stdout. iOS forbids child processes, so here the very same
//! Service runs on a tokio runtime owned by this module and the two ends
//! are joined by in-memory pipes. The wire protocol is unchanged: every
//! request is a JSON-RPC line from `service-api`, so whatever the desktop
//! Service can do, the phone can ask for in the same words.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use service_api::{
    BootReadyResponse, BoundedLineReader, HealthPingResponse, JsonRpcRequest, ParsedServiceMessage,
    RequestParams, ServiceResponse, parse_service_message, write_message,
};
use tokio::io::DuplexStream;
use tokio::sync::{Mutex, oneshot};

/// Bytes buffered in each direction between the app and the Service.
const PIPE_CAPACITY: usize = 1024 * 1024;

struct Engine {
    runtime: tokio::runtime::Runtime,
    to_service: Mutex<DuplexStream>,
    pending: std::sync::Mutex<HashMap<u64, oneshot::Sender<ServiceResponse>>>,
    next_id: AtomicU64,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// What the engine reported once its boot sequence finished.
pub struct EngineReady {
    /// Highest schema version present after migrations ran.
    pub schema_version: u32,
    /// Migrations applied while booting (0 when the database was current).
    pub migrations_applied: u32,
    /// Problems the boot repaired on its own, in plain words.
    pub recovery_warnings: Vec<String>,
}

/// Start the engine on `data_dir` and wait until its boot sequence is done.
///
/// The encryption key is created on first launch. Calling this again once
/// the engine runs is an error rather than a second engine on the same
/// database.
pub fn start_engine(data_dir: String) -> Result<EngineReady, String> {
    let data_dir = PathBuf::from(data_dir);
    if !data_dir.is_dir() {
        return Err(format!(
            "data directory does not exist: {}",
            data_dir.display()
        ));
    }
    if ENGINE.get().is_some() {
        return Err("the engine is already running".to_owned());
    }
    crypto_key::load_or_create_encryption_key(&data_dir)
        .map_err(|error| format!("encryption key: {error}"))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("courly-engine")
        .build()
        .map_err(|error| format!("start runtime: {error}"))?;
    let (app_writer, service_reader) = tokio::io::duplex(PIPE_CAPACITY);
    let (service_writer, app_reader) = tokio::io::duplex(PIPE_CAPACITY);
    runtime.spawn(service::run_service_with_io(
        service_reader,
        service_writer,
        data_dir,
    ));

    let engine = Engine {
        runtime,
        to_service: Mutex::new(app_writer),
        pending: std::sync::Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(1),
    };
    if ENGINE.set(engine).is_err() {
        return Err("the engine is already running".to_owned());
    }
    let engine = running_engine()?;
    engine.runtime.spawn(route_responses(BoundedLineReader::new(
        app_reader,
        service_api::MAX_FRAME_BYTES,
    )));

    let ready: BootReadyResponse = call(RequestParams::BootReady)?;
    if !ready.ready {
        return Err("the engine finished booting but reports it is not ready".to_owned());
    }
    Ok(EngineReady {
        schema_version: ready.schema_version,
        migrations_applied: ready.migrations_applied,
        recovery_warnings: ready.recovery_warnings,
    })
}

/// Round-trip a ping through the running engine; returns its uptime in
/// milliseconds.
pub fn engine_ping() -> Result<u64, String> {
    let response: HealthPingResponse = call(RequestParams::HealthPing)?;
    Ok(response.uptime_ms)
}

fn running_engine() -> Result<&'static Engine, String> {
    ENGINE
        .get()
        .ok_or_else(|| "the engine is not running; call start_engine first".to_owned())
}

/// Send one request and wait for its response, decoded as `T`.
fn call<T: serde::de::DeserializeOwned>(params: RequestParams) -> Result<T, String> {
    let engine = running_engine()?;
    let id = engine.next_id.fetch_add(1, Ordering::Relaxed);
    let (reply_tx, reply_rx) = oneshot::channel();
    engine
        .pending
        .lock()
        .map_err(|_| "engine request table poisoned".to_owned())?
        .insert(id, reply_tx);

    let method = params.method_name();
    let response = engine.runtime.block_on(async {
        {
            let mut writer = engine.to_service.lock().await;
            write_message(&JsonRpcRequest::new(id, &params), &mut *writer)
                .await
                .map_err(|error| format!("{method}: write request: {error}"))?;
        }
        reply_rx
            .await
            .map_err(|_| format!("{method}: the engine stopped before answering"))
    })?;

    match response {
        ServiceResponse::Success(value) => serde_json::from_value(value)
            .map_err(|error| format!("{method}: unexpected response: {error}")),
        ServiceResponse::Error(error) => Err(format!("{method}: {}", error.message)),
    }
}

/// Hand every response the Service writes to the request waiting for it.
/// Notifications are not consumed yet; they are read and dropped so the
/// pipe never fills.
async fn route_responses(mut reader: BoundedLineReader<DuplexStream>) {
    loop {
        let line = match reader.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => return,
            Err(error) => {
                log::warn!("engine: reading the service output failed: {error}");
                return;
            }
        };
        let Ok(ParsedServiceMessage::Response {
            id: Some(id),
            response,
        }) = parse_service_message(&line)
        else {
            continue;
        };
        let Some(engine) = ENGINE.get() else {
            return;
        };
        let waiter = match engine.pending.lock() {
            Ok(mut pending) => pending.remove(&id),
            Err(_) => return,
        };
        if let Some(waiter) = waiter {
            // The caller may have given up; nothing left to deliver to then.
            let _ = waiter.send(response);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test only: the engine is process-wide, like the app's.
    #[test]
    fn the_engine_boots_in_process_answers_and_refuses_a_second_start() {
        assert!(engine_ping().is_err(), "nothing runs before start_engine");

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().into_owned();
        let ready = start_engine(path.clone()).expect("engine boots");
        assert!(ready.schema_version >= 100);
        assert!(
            ready.migrations_applied > 0,
            "a fresh directory is migrated"
        );
        assert!(
            dir.path().join("courly.key").is_file(),
            "the key is created"
        );

        engine_ping().expect("the running engine answers a ping");
        assert!(start_engine(path).is_err(), "a second engine is refused");
    }
}
