//! `PlatformHooks` (contract C4, DESIGN §3.5): implemented in Swift (`EngineHost`,
//! MeshEngineKit) and Kotlin (`EngineHost`, meshengine), called by the engine thread.
//! Every method must return quickly and never block on the engine.

use super::outputs::{
    FfiAudioCommand, FfiLogLevel, FfiNotification, FfiPromptRequest, FfiRadioCommand,
    FfiSendOutcome,
};

/// Engine → platform calls.
#[uniffi::export(with_foreign)]
pub trait PlatformHooks: Send + Sync {
    /// Writes one unit on a link (stream chunk, ATT value or datagram). Non-blocking.
    fn link_send(&self, link: u32, bytes: Vec<u8>) -> FfiSendOutcome;
    /// Executes a radio decision.
    fn radio(&self, cmd: FfiRadioCommand);
    /// Executes an audio decision.
    fn audio(&self, cmd: FfiAudioCommand);
    /// A UI event (JSON), forwarded verbatim to the plugin.
    fn ui_event(&self, json: String);
    /// Secure storage write (Keychain / Keystore); `None` deletes the key.
    fn persist(&self, key: String, value: Option<Vec<u8>>);
    /// Local notification, Live Activity or FGS notification update.
    fn notify(&self, n: FfiNotification);
    /// Appends one labrun/1 JSONL line to `<runId>.jsonl`.
    fn lab_write(&self, run_id: String, line: String);
    /// Appends one line (header or row) to `<runId>-summary.csv`.
    fn lab_write_summary(&self, run_id: String, line: String);
    /// Deletes every file of a Lab run.
    fn lab_delete(&self, run_id: String);
    /// Renders spoken prompts with platform TTS; results come back via `on_prompt_pcm`.
    fn render_prompts(&self, reqs: Vec<FfiPromptRequest>);
    /// A log line.
    fn log(&self, level: FfiLogLevel, msg: String);
    /// Appends `lines` (ridelog/1 JSONL, no trailing newlines) to segment `seg` of
    /// diagnostics session `session_id` (`diag/<sessionId>/seg-NNNN.jsonl` in
    /// app-private storage). Called at most about once per second; the first line of a
    /// new segment is its `hdr`. Must not block.
    fn diag_write(&self, session_id: String, seg: u32, lines: Vec<String>);
}
