//! Read-only session state. Publishing never performs bus I/O on the animation thread.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, SyncSender},
};
use zbus::{blocking::connection::Builder, zvariant::Type};

pub const SERVICE: &str = "org.omarchy.Screensaver";
pub const PATH: &str = "/org/omarchy/Screensaver";
pub const INTERFACE: &str = "org.omarchy.Screensaver1";
pub const MAX_INPUT_BYTES: usize = 65_536;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct Snapshot {
    pub revision: u64,
    pub active: bool,
    pub engine: String,
    pub effect: String,
    pub input: String,
    pub rng_state: Vec<u64>,
}
pub struct Publisher {
    snapshot: Arc<Mutex<Snapshot>>,
    wake: SyncSender<()>,
}
struct Service(Arc<Mutex<Snapshot>>);
#[zbus::interface(name = "org.omarchy.Screensaver1")]
impl Service {
    fn get_state(&self) -> Snapshot {
        self.0.lock().unwrap().clone()
    }
    #[zbus(signal)]
    async fn state_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        state: Snapshot,
    ) -> zbus::Result<()>;
}
impl Publisher {
    pub fn start() -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (wake, notifications) = mpsc::sync_channel(1);
        let current = snapshot.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("screensaver-bus".into())
            .spawn(move || {
                if let Err(error) = Self::serve(current, notifications) {
                    eprintln!("screensaver observer unavailable: {error}");
                }
            })
        {
            eprintln!("screensaver observer unavailable: {error}");
        }
        Self { snapshot, wake }
    }
    pub fn publish(&self, mut snapshot: Snapshot) {
        if snapshot.input.len() > MAX_INPUT_BYTES {
            snapshot = Snapshot::default();
        }
        let mut current = self.snapshot.lock().unwrap();
        snapshot.revision = current.revision.wrapping_add(1);
        *current = snapshot;
        // A queued wake already represents the latest state. Never wait for
        // the bus worker or retain an unbounded transition backlog.
        let _ = self.wake.try_send(());
    }
    fn serve(snapshot: Arc<Mutex<Snapshot>>, notifications: Receiver<()>) -> zbus::Result<()> {
        let connection = Builder::session()?
            .name(SERVICE)?
            .serve_at(PATH, Service(snapshot.clone()))?
            .build()?;
        while notifications.recv().is_ok() {
            let current = snapshot.lock().unwrap().clone();
            connection.emit_signal(None::<&str>, PATH, INTERFACE, "StateChanged", &(current,))?;
        }
        connection.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_coalesces_and_drop_closes_notifications() {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (wake, notifications) = mpsc::sync_channel(1);
        let publisher = Publisher {
            snapshot: snapshot.clone(),
            wake,
        };
        for effect in ["rain", "matrix"] {
            publisher.publish(Snapshot {
                active: true,
                effect: effect.into(),
                ..Snapshot::default()
            });
        }
        assert_eq!(notifications.try_recv(), Ok(()));
        assert_eq!(notifications.try_recv(), Err(mpsc::TryRecvError::Empty));
        assert_eq!(snapshot.lock().unwrap().effect, "matrix");
        assert_eq!(snapshot.lock().unwrap().revision, 2);
        publisher.publish(Snapshot {
            active: true,
            input: "x".repeat(MAX_INPUT_BYTES + 1),
            ..Snapshot::default()
        });
        assert!(!snapshot.lock().unwrap().active);
        assert!(snapshot.lock().unwrap().input.is_empty());
        assert_eq!(snapshot.lock().unwrap().revision, 3);
        assert_eq!(notifications.try_recv(), Ok(()));
        drop(publisher);
        assert_eq!(notifications.recv(), Err(mpsc::RecvError));
    }
}
