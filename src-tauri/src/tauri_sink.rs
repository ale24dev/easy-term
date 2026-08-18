//! The GUI's [`EventSink`]: turns process events into Tauri events.
//!
//! This is the half of the split that stays in the app — the daemon's own
//! sink broadcasts to socket clients instead. Both sides feed the exact same
//! `Event` stream, so `process_manager` doesn't know or care which one it's
//! reporting to.
//!
//! It also handles the two side effects that used to live inline in
//! `process_manager` and are genuinely UI concerns: refreshing the tray on a
//! status change, and posting a native notification on a crash.

use crate::daemon::protocol::Event;
use crate::process_manager::EventSink;
use tauri::{AppHandle, Emitter};

pub struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl EventSink for TauriSink {
    fn emit(&self, event: Event) {
        // Forward to the frontend under the `process:*` names it already
        // listens for, so none of the UI code had to change for the split.
        if let Some((name, payload)) = event.tauri_event() {
            let _ = self.app.emit(name, payload);
        }

        match &event {
            Event::Status { .. } => crate::tray::refresh(&self.app),
            Event::Crashed { id, name, code } => {
                crate::notifications::notify_crash(&self.app, id, name, *code)
            }
            _ => {}
        }
    }
}
