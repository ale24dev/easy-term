//! Keeps project processes alive across GUI restarts.
//!
//! ## Why a daemon
//!
//! Project processes run inside a PTY whose **master** end the app holds. If
//! the app is the process holding it, quitting the app closes that end, the
//! kernel hangs up the terminal, and every process in the PTY's foreground
//! group gets SIGHUP — so quitting killed the very dev servers it was
//! supposed to be managing. (Verified experimentally: a child with a PTY dies
//! when its parent exits, while an otherwise identical child on plain pipes
//! survives. Ignoring SIGHUP doesn't save it either, because writes to a
//! hung-up terminal then fail with EIO.)
//!
//! So keeping the processes alive isn't a matter of *not killing* them — it
//! requires somebody to keep holding the master fd. That somebody is this
//! daemon: a headless instance of the same binary (`easy-term --daemon`) that
//! owns every PTY and outlives the GUI. The GUI becomes a client that
//! connects over a Unix socket, sends commands, and streams events back —
//! the same model tmux uses, and for the same reason.
//!
//! Because the daemon also holds the ring buffers, reconnecting restores the
//! scrollback from before the app was closed, not just the process list.

pub mod client;
pub mod protocol;
pub mod server;
