// Pure meeting-webhook decisions: wake-word matching, context window, debounce.
// No I/O, so recording and transcription cannot fail here.

use serde::Serialize;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const REASON_WAKE_WORD: &str = "wake_word";
pub const REASON_INTERVAL: &str = "interval";
pub const REASON_TEST: &str = "test";

/// How long a wake-word post suppresses another one.
pub const WAKE_WORD_COOLDOWN: Duration = Duration::from_secs(20);

/// Lines included with a wake-word post, measured in recording time.
pub const WAKE_WORD_CONTEXT_SECS: f64 = 120.0;

/// One transcript line kept for the life of a recording.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredLine {
    pub sequence_id: u64,
    pub speaker: String,
    pub text: String,
    pub audio_start_time: f64,
    pub audio_end_time: f64,
    pub timestamp: String,
}

/// One line in the JSON body. `speaker` is empty when the app has no diarization.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WebhookLine {
    pub speaker: String,
    pub text: String,
    pub audio_start_time: f64,
    pub timestamp: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WebhookPayload {
    pub meeting_id: String,
    pub meeting_title: String,
    pub reason: String,
    pub lines: Vec<WebhookLine>,
}

impl StoredLine {
    pub fn to_payload(&self) -> WebhookLine {
        WebhookLine {
            speaker: self.speaker.clone(),
            text: self.text.clone(),
            audio_start_time: self.audio_start_time,
            timestamp: self.timestamp.clone(),
        }
    }
}

/// Case-insensitive whole-word match. The wake word is never treated as a regex.
/// Boundaries are non-word characters (or the ends of the text), so a wake word
/// that itself ends in punctuation still matches as a whole token.
pub fn contains_wake_word(text: &str, wake_word: &str) -> bool {
    let wake = wake_word.trim();
    if wake.is_empty() || text.trim().is_empty() {
        return false;
    }
    let pattern = format!(r"(?:^|\W){}(?:$|\W)", regex::escape(wake));
    match regex::RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .build()
    {
        Ok(re) => re.is_match(text),
        Err(_) => false,
    }
}

/// True when this sequence was already posted, or a wake-word post is still inside
/// the cooldown. One mention therefore produces one post, including a segment that
/// is emitted again with the same sequence id.
pub fn wake_is_suppressed(
    sequence_id: u64,
    posted: &HashSet<u64>,
    last_post: Option<Instant>,
    now: Instant,
    cooldown: Duration,
) -> bool {
    if posted.contains(&sequence_id) {
        return true;
    }
    if let Some(last) = last_post {
        if now.saturating_duration_since(last) < cooldown {
            return true;
        }
    }
    false
}

/// The triggering line plus other lines that overlap the previous `window_secs`
/// of recording time. The trigger is always included.
pub fn select_context(
    lines: &[StoredLine],
    trigger_sequence_id: u64,
    window_secs: f64,
) -> Vec<WebhookLine> {
    let Some(trigger) = lines
        .iter()
        .find(|line| line.sequence_id == trigger_sequence_id)
    else {
        return Vec::new();
    };
    let trigger_end = positive_or(trigger.audio_end_time, trigger.audio_start_time);
    let window_start = (trigger_end - window_secs).max(0.0);

    let mut selected: Vec<&StoredLine> = lines
        .iter()
        .filter(|line| line_in_window(line, trigger_sequence_id, window_start, trigger_end))
        .collect();
    selected.sort_by(|a, b| {
        a.audio_start_time
            .total_cmp(&b.audio_start_time)
            .then(a.sequence_id.cmp(&b.sequence_id))
    });
    selected.into_iter().map(StoredLine::to_payload).collect()
}

fn line_in_window(
    line: &StoredLine,
    trigger_sequence_id: u64,
    window_start: f64,
    trigger_end: f64,
) -> bool {
    if line.sequence_id == trigger_sequence_id {
        return true;
    }
    let line_end = positive_or(line.audio_end_time, line.audio_start_time);
    line_end >= window_start && line.audio_start_time <= trigger_end
}

fn positive_or(primary: f64, fallback: f64) -> f64 {
    if primary.is_finite() && primary > 0.0 {
        primary
    } else if fallback.is_finite() {
        fallback.max(0.0)
    } else {
        0.0
    }
}

/// Half-open range of lines not yet included in a successful interval post.
pub fn pending_range(line_count: usize, cursor: usize) -> Option<(usize, usize)> {
    if cursor >= line_count {
        None
    } else {
        Some((cursor, line_count))
    }
}

pub fn test_payload() -> WebhookPayload {
    WebhookPayload {
        meeting_id: "meeting-test".to_string(),
        meeting_title: "Webhook test".to_string(),
        reason: REASON_TEST.to_string(),
        lines: vec![WebhookLine {
            speaker: String::new(),
            text: "This is a test payload from Meetily.".to_string(),
            audio_start_time: 0.0,
            timestamp: "00:00:00".to_string(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(sequence_id: u64, start: f64, end: f64, text: &str) -> StoredLine {
        StoredLine {
            sequence_id,
            speaker: String::new(),
            text: text.to_string(),
            audio_start_time: start,
            audio_end_time: end,
            timestamp: "12:00:00".to_string(),
        }
    }

    #[test]
    fn wake_word_is_case_insensitive_whole_word() {
        assert!(contains_wake_word("Hey Emma, can you summarize?", "Emma"));
        assert!(contains_wake_word("hey EMMA!", "emma"));
        assert!(contains_wake_word("emma's notes", "Emma"));
        assert!(contains_wake_word("Please ask Emma.", "Emma"));
        assert!(!contains_wake_word("Emmanuel joined", "Emma"));
        assert!(!contains_wake_word("that gemma model", "Emma"));
        assert!(!contains_wake_word("no name here", "Emma"));
        assert!(!contains_wake_word("Emma", "  "));
        assert!(contains_wake_word("Hey Emma, over here", "hey emma"));
        assert!(!contains_wake_word("hey emmanuel", "hey emma"));
    }

    #[test]
    fn wake_word_is_literal_not_a_regex() {
        assert!(contains_wake_word("ping Emma+ now", "Emma+"));
        assert!(!contains_wake_word("ping Emma now", "Emma+"));
    }

    #[test]
    fn cooldown_and_sequence_suppress_repeat_posts() {
        let mut posted = HashSet::new();
        let start = Instant::now();
        assert!(!wake_is_suppressed(
            7,
            &posted,
            None,
            start,
            WAKE_WORD_COOLDOWN
        ));

        posted.insert(7);
        let later = start + Duration::from_secs(30);
        assert!(wake_is_suppressed(
            7,
            &posted,
            Some(start),
            later,
            WAKE_WORD_COOLDOWN
        ));

        let during = start + Duration::from_secs(19);
        assert!(wake_is_suppressed(
            8,
            &posted,
            Some(start),
            during,
            WAKE_WORD_COOLDOWN
        ));

        let after = start + WAKE_WORD_COOLDOWN;
        assert!(!wake_is_suppressed(
            8,
            &posted,
            Some(start),
            after,
            WAKE_WORD_COOLDOWN
        ));
    }

    #[test]
    fn context_is_the_trigger_plus_previous_two_minutes() {
        let lines = vec![
            line(1, 0.0, 2.0, "opening"),
            line(2, 10.0, 20.0, "overlaps the window"),
            line(3, 100.0, 110.0, "inside"),
            line(4, 130.0, 133.0, "Emma please"),
            line(5, 140.0, 145.0, "after the trigger"),
        ];
        let selected = select_context(&lines, 4, WAKE_WORD_CONTEXT_SECS);
        let texts: Vec<&str> = selected.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, vec!["overlaps the window", "inside", "Emma please"]);
        assert!(selected.iter().all(|line| line.speaker.is_empty()));
    }

    #[test]
    fn context_includes_the_trigger_when_timestamps_are_missing() {
        let lines = vec![line(1, f64::NAN, f64::NAN, "Emma")];
        let selected = select_context(&lines, 1, WAKE_WORD_CONTEXT_SECS);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].text, "Emma");
    }

    #[test]
    fn pending_range_skips_when_nothing_is_new() {
        assert_eq!(pending_range(0, 0), None);
        assert_eq!(pending_range(4, 4), None);
        assert_eq!(pending_range(4, 1), Some((1, 4)));
    }

    #[test]
    fn test_payload_uses_the_test_reason() {
        let payload = test_payload();
        assert_eq!(payload.reason, "test");
        assert_eq!(payload.meeting_id, "meeting-test");
        assert_eq!(payload.lines.len(), 1);
        assert_eq!(payload.lines[0].audio_start_time, 0.0);
    }
}
