//! Platform-independent core of humanize_input: the typing models and the
//! session that drives them.
//!
//! - [`human::HumanPlanner`] (default) and [`plan::Planner`] (the original
//!   model, via [`plan::ClassicKeys`]) turn text into [`KeyAction`]s.
//! - [`session::Session`] plays them through a key-level [`session::Driver`]
//!   on a worker thread, with pause, cancel and focus-loss handling.
//! - [`layout::Layout`] maps characters to physical keys;
//!   [`editor::VirtualEditor`] replays key events into text for tests and
//!   dry runs.

pub mod delay;
pub mod editor;
pub mod geometry;
pub mod human;
pub mod keys;
pub mod layout;
pub mod plan;
pub mod progress;
pub mod session;
pub mod settings;
pub mod text;

pub use editor::VirtualEditor;
pub use geometry::KeyboardGeometry;
pub use human::{HumanModel, HumanPlanner, Macros, Preset, WordDelete};
pub use keys::{Finger, Hand, KeyAction, KeyCode, Mods, Stroke};
pub use layout::{KeyInfo, Layout};
pub use plan::{Action, ClassicKeys, Planner};
pub use progress::TypingProgress;
pub use session::{
    AlwaysFocused, Driver, DriverError, EventSink, FocusTracker, LogEvent, Session, SessionEvent,
    SessionHooks, SessionState, StartError, TypingMode, WindowId,
};
pub use settings::TypingSettings;
