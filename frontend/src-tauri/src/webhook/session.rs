// Recording-scoped webhook session.
// Transcript observation only updates memory and, when a post is due, spawns
// the HTTP call. It never waits on the network.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use tauri::{AppHandle, Runtime};
use tokio::sync::watch;

use super::dispatch;
use super::logic::{
    contains_wake_word, pending_range, select_context, wake_is_suppressed, StoredLine,
    WebhookPayload, REASON_INTERVAL, REASON_WAKE_WORD, WAKE_WORD_CONTEXT_SECS, WAKE_WORD_COOLDOWN,
};
use super::settings::{self, WebhookSettings};
use crate::audio::transcription::TranscriptUpdate;

struct Session {
    generation: u64,
    meeting_id: String,
    meeting_title: String,
    lines: Vec<StoredLine>,
    /// Index of the first line not yet included in a successful interval post.
    interval_cursor: usize,
    last_wake_at: Option<Instant>,
    posted_wake_sequences: HashSet<u64>,
    send_in_flight: bool,
    logged_missing_url: bool,
}

struct WebhookRuntime {
    settings: WebhookSettings,
    settings_loaded: bool,
    session: Option<Session>,
    next_generation: u64,
    interval_owner: Option<u64>,
    last_meeting_id: Option<String>,
}

impl WebhookRuntime {
    fn new() -> Self {
        Self {
            settings: WebhookSettings::default(),
            settings_loaded: false,
            session: None,
            next_generation: 0,
            interval_owner: None,
            last_meeting_id: None,
        }
    }
}

static RUNTIME: std::sync::LazyLock<Mutex<WebhookRuntime>> =
    std::sync::LazyLock::new(|| Mutex::new(WebhookRuntime::new()));

static INTERVAL_PING: std::sync::LazyLock<watch::Sender<u64>> = std::sync::LazyLock::new(|| {
    let (sender, _receiver) = watch::channel(0);
    sender
});

fn lock() -> MutexGuard<'static, WebhookRuntime> {
    RUNTIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn spawn_interval(generation: u64) -> bool {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(interval_loop(generation));
            true
        }
        Err(_) => {
            log::error!(
                "meeting webhook interval loop was not started: async runtime is not running"
            );
            false
        }
    }
}

fn bump_interval_ping() {
    let next = INTERVAL_PING.borrow().wrapping_add(1);
    let _ = INTERVAL_PING.send(next);
}

fn interval_enabled(settings: &WebhookSettings) -> bool {
    settings.enabled && settings.mode == "interval" && !settings.url.trim().is_empty()
}

fn sync_interval_task(rt: &mut WebhookRuntime) {
    let generation = rt.session.as_ref().map(|session| session.generation);
    let should_run = generation.is_some() && interval_enabled(&rt.settings);
    if should_run {
        if rt.interval_owner != generation {
            let generation = generation.unwrap_or(0);
            if spawn_interval(generation) {
                rt.interval_owner = Some(generation);
            }
        }
    } else if let Some(session) = rt.session.as_mut() {
        if rt.settings.enabled
            && rt.settings.mode == "interval"
            && rt.settings.url.trim().is_empty()
            && !session.logged_missing_url
        {
            log::error!(
                "meeting webhook interval mode is enabled but the URL is empty; not posting"
            );
            session.logged_missing_url = true;
        }
        rt.interval_owner = None;
    } else {
        rt.interval_owner = None;
    }
    bump_interval_ping();
}

/// Start a webhook session for the recording that just became active.
/// Returns the meeting id that later saves should reuse. Never fails the caller.
pub async fn begin_session<R: Runtime>(app: &AppHandle<R>, meeting_title: &str) -> String {
    let mut load_failed = false;
    let loaded = if lock().settings_loaded {
        None
    } else {
        match settings::load_from_app(app).await {
            Ok(settings) => Some(settings),
            Err(err) => {
                log::error!(
                    "meeting webhook settings could not be loaded ({err}); live posts stay off until settings load"
                );
                load_failed = true;
                Some(WebhookSettings::default())
            }
        }
    };

    let title = {
        let trimmed = meeting_title.trim();
        if trimmed.is_empty() {
            "Meeting".to_string()
        } else {
            trimmed.to_string()
        }
    };
    let meeting_id = format!("meeting-{}", uuid::Uuid::new_v4());

    let mut rt = lock();
    if let Some(settings) = loaded {
        rt.settings = settings;
        if !load_failed {
            rt.settings_loaded = true;
        }
    }
    rt.next_generation = rt.next_generation.wrapping_add(1);
    let generation = rt.next_generation;
    rt.session = Some(Session {
        generation,
        meeting_id: meeting_id.clone(),
        meeting_title: title,
        lines: Vec::new(),
        interval_cursor: 0,
        last_wake_at: None,
        posted_wake_sequences: HashSet::new(),
        send_in_flight: false,
        logged_missing_url: false,
    });
    rt.last_meeting_id = Some(meeting_id.clone());
    sync_interval_task(&mut rt);
    log::info!("meeting webhook session started ({meeting_id})");
    meeting_id
}

/// End the session when recording is no longer active. In-flight posts are
/// allowed to finish; nothing new is queued.
pub fn end_session() -> Option<String> {
    let mut rt = lock();
    let meeting_id = rt.session.take().map(|session| session.meeting_id);
    rt.interval_owner = None;
    if meeting_id.is_some() {
        rt.last_meeting_id = meeting_id.clone();
        log::info!("meeting webhook session ended");
    }
    bump_interval_ping();
    meeting_id
}

pub fn last_meeting_id() -> Option<String> {
    lock().last_meeting_id.clone()
}

pub fn apply_settings(settings: WebhookSettings) {
    let mut rt = lock();
    rt.settings = settings;
    rt.settings_loaded = true;
    if let Some(session) = rt.session.as_mut() {
        session.logged_missing_url = false;
    }
    sync_interval_task(&mut rt);
}

struct WakeJob {
    generation: u64,
    settings: WebhookSettings,
    payload: WebhookPayload,
}

/// Record a completed transcript segment and maybe schedule one wake-word post.
/// Returns immediately. Safe to call from the transcription worker.
pub fn observe_transcript(update: &TranscriptUpdate) {
    let job = {
        let mut rt = lock();
        decide_wake(&mut rt, update)
    };
    let Some(job) = job else {
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        log::error!("meeting webhook skipped wake-word post: async runtime is not running");
        clear_in_flight(job.generation);
        return;
    };
    let line_count = job.payload.lines.len();
    handle.spawn(async move {
        let generation = job.generation;
        match dispatch::post(&job.settings, &job.payload).await {
            Ok(status) => {
                log::info!(
                    "meeting webhook posted reason={REASON_WAKE_WORD} lines={line_count} status={status} host={}",
                    dispatch::endpoint_host(&job.settings.url)
                );
            }
            Err(err) => {
                log::error!("meeting webhook wake-word post failed: {err}");
            }
        }
        clear_in_flight(generation);
    });
}

fn clear_in_flight(generation: u64) {
    let mut rt = lock();
    if let Some(session) = rt.session.as_mut() {
        if session.generation == generation {
            session.send_in_flight = false;
        }
    }
}

fn decide_wake(rt: &mut WebhookRuntime, update: &TranscriptUpdate) -> Option<WakeJob> {
    let settings = rt.settings.clone();
    let session = rt.session.as_mut()?;
    upsert_line(session, update);

    // Each VAD segment is transcribed once and emitted here. Whisper's
    // `is_partial` flag means the chunk was shorter than 15s, not that a later
    // final result will replace it. Treat the emission as the finalized line.
    let _ = update.is_partial;

    if !settings.enabled || settings.mode != "wake_word" {
        return None;
    }
    if settings.url.trim().is_empty() {
        if !session.logged_missing_url {
            log::error!("meeting webhook is enabled but the URL is empty; not posting");
            session.logged_missing_url = true;
        }
        return None;
    }
    if !contains_wake_word(&update.text, &settings.wake_word) {
        return None;
    }
    if wake_is_suppressed(
        update.sequence_id,
        &session.posted_wake_sequences,
        session.last_wake_at,
        Instant::now(),
        WAKE_WORD_COOLDOWN,
    ) {
        log::debug!("meeting webhook wake-word post suppressed by debounce");
        return None;
    }
    if session.send_in_flight {
        log::debug!(
            "meeting webhook skipped wake-word post because another post is still in flight"
        );
        return None;
    }

    let lines = select_context(&session.lines, update.sequence_id, WAKE_WORD_CONTEXT_SECS);
    if lines.is_empty() {
        return None;
    }
    let payload = WebhookPayload {
        meeting_id: session.meeting_id.clone(),
        meeting_title: session.meeting_title.clone(),
        reason: REASON_WAKE_WORD.to_string(),
        lines,
    };
    session.posted_wake_sequences.insert(update.sequence_id);
    session.last_wake_at = Some(Instant::now());
    session.send_in_flight = true;
    Some(WakeJob {
        generation: session.generation,
        settings,
        payload,
    })
}

fn upsert_line(session: &mut Session, update: &TranscriptUpdate) {
    let text = update.text.trim();
    if text.is_empty() {
        return;
    }
    if let Some(existing) = session
        .lines
        .iter_mut()
        .find(|line| line.sequence_id == update.sequence_id)
    {
        existing.text = update.text.clone();
        existing.audio_start_time = update.audio_start_time;
        existing.audio_end_time = update.audio_end_time;
        existing.timestamp = update.timestamp.clone();
        return;
    }
    session.lines.push(StoredLine {
        sequence_id: update.sequence_id,
        // Community edition mixes mic and system audio before transcription and
        // does not diarize, so there is no speaker name to send.
        speaker: String::new(),
        text: update.text.clone(),
        audio_start_time: update.audio_start_time,
        audio_end_time: update.audio_end_time,
        timestamp: update.timestamp.clone(),
    });
}

async fn interval_loop(generation: u64) {
    let mut ping = INTERVAL_PING.subscribe();
    loop {
        let minutes = {
            let rt = lock();
            let Some(session) = rt.session.as_ref() else {
                break;
            };
            if session.generation != generation || !interval_enabled(&rt.settings) {
                break;
            }
            rt.settings.interval_minutes.max(1)
        };

        let sleep = tokio::time::sleep(std::time::Duration::from_secs(u64::from(minutes) * 60));
        tokio::pin!(sleep);
        tokio::select! {
            _ = &mut sleep => {}
            changed = ping.changed() => {
                if changed.is_err() {
                    break;
                }
                continue;
            }
        }

        enum Tick {
            Stop,
            Wait,
            Post {
                generation: u64,
                cursor_end: usize,
                settings: WebhookSettings,
                payload: WebhookPayload,
            },
        }

        let tick = {
            let mut rt = lock();
            let generation_matches = rt
                .session
                .as_ref()
                .map(|session| session.generation == generation)
                .unwrap_or(false);
            let still_active = generation_matches && interval_enabled(&rt.settings);
            if !still_active {
                Tick::Stop
            } else if rt
                .session
                .as_ref()
                .map(|session| session.send_in_flight)
                .unwrap_or(false)
            {
                log::debug!("meeting webhook interval tick skipped; a post is already in flight");
                Tick::Wait
            } else {
                let settings = rt.settings.clone();
                let session = rt.session.as_mut().expect("session checked above");
                match pending_range(session.lines.len(), session.interval_cursor) {
                    None => {
                        log::debug!("meeting webhook interval tick skipped; no new lines");
                        Tick::Wait
                    }
                    Some((start, end)) => {
                        let lines = session.lines[start..end]
                            .iter()
                            .map(StoredLine::to_payload)
                            .collect::<Vec<_>>();
                        let payload = WebhookPayload {
                            meeting_id: session.meeting_id.clone(),
                            meeting_title: session.meeting_title.clone(),
                            reason: REASON_INTERVAL.to_string(),
                            lines,
                        };
                        session.send_in_flight = true;
                        Tick::Post {
                            generation: session.generation,
                            cursor_end: end,
                            settings,
                            payload,
                        }
                    }
                }
            }
        };

        let (job_generation, cursor_end, settings, payload) = match tick {
            Tick::Stop => break,
            Tick::Wait => continue,
            Tick::Post {
                generation,
                cursor_end,
                settings,
                payload,
            } => (generation, cursor_end, settings, payload),
        };
        let line_count = payload.lines.len();
        let result = dispatch::post(&settings, &payload).await;
        {
            let mut rt = lock();
            if let Some(session) = rt.session.as_mut() {
                if session.generation == job_generation {
                    session.send_in_flight = false;
                    if result.is_ok() {
                        session.interval_cursor = cursor_end.max(session.interval_cursor);
                    }
                }
            }
        }
        match result {
            Ok(status) => {
                log::info!(
                    "meeting webhook posted reason={REASON_INTERVAL} lines={line_count} status={status} host={}",
                    dispatch::endpoint_host(&settings.url)
                );
            }
            Err(err) => {
                log::error!("meeting webhook interval post failed: {err}");
            }
        }
    }

    let mut rt = lock();
    if rt.interval_owner == Some(generation) {
        rt.interval_owner = None;
    }
}
