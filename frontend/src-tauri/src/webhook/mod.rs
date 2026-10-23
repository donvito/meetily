// Live meeting webhook.
// Posts run only while a recording session is active, off the transcription path.

pub mod commands;
mod dispatch;
mod logic;
mod session;
mod settings;

pub use session::{begin_session, end_session, observe_transcript};
