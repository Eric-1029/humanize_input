//! Platform-independent core of humanize_input: the typing model and the
//! session that drives it.
//!
//! - [`plan::Planner`] turns text + [`TypingSettings`] into [`plan::Action`]s.
//! - [`session::Session`] executes them through a [`session::Driver`] on a
//!   worker thread, with pause, cancel and focus-loss handling.

pub mod delay;
pub mod geometry;
pub mod plan;
pub mod progress;
pub mod session;
pub mod settings;
pub mod text;

pub use geometry::KeyboardGeometry;
pub use plan::{Action, Planner};
pub use progress::TypingProgress;
pub use session::{
    AlwaysFocused, Driver, DriverError, EventSink, FocusTracker, LogEvent, Session, SessionEvent,
    SessionState, StartError, WindowId,
};
pub use settings::TypingSettings;
