# Live meeting webhook

While a recording is active, Meetily can POST finalized transcript lines to a webhook you configure. Nothing about the receiver is built in: the URL, header, wake word, and interval all come from Settings.

Open **Settings → Recordings → Live meeting webhook**.

## Settings

| Setting | Default | Notes |
| --- | --- | --- |
| Enable webhook | off | Posts run only while a recording is active. |
| Webhook URL | empty | `https://` or `http://`. |
| Auth header name | `Authorization` | Sent as the HTTP header name. |
| Sender key | empty | The header value, stored locally in the same SQLite database as other API keys (`meeting_minutes.sqlite` in the app data directory). It is masked in the UI. |
| Mode | Wake word | `wake_word` or `interval`. |
| Wake word | `Emma` | Whole word, case-insensitive. Used in wake-word mode. |
| Interval (minutes) | `5` | Used in interval mode. |

Click **Save** to apply the settings. Saved values apply immediately, including to a recording that is already running.

**Send test** posts a sample payload with reason `test` using the values currently in the form, even if they are not saved yet, and shows success or failure. A test does not require a recording and does not require the enable switch to be on.

Paste the sender key exactly as the receiver shows it. Meetily does not add a `Bearer ` prefix. If the panel value is already `Bearer ...`, paste that whole value. Surrounding whitespace from a paste is removed; the value is not otherwise changed. The header is sent as:

```http
Authorization: <value you pasted>
```

with whatever header name you set. If the sender key is empty, the header is omitted.

Redirects are not followed, so the header is not forwarded to another host.

## When posts are sent

Posts happen only while a recording session is active (including while it is paused, and while the last audio chunks are still being transcribed after you press stop). The session ends when the recording flag is cleared.

Failures never stop recording or transcription. Each post runs asynchronously, off the transcription path, with a 5 second timeout. There is no retry queue and no unbounded backlog: at most one request is in flight. Errors are written to the app log and nowhere else.

Meetily emits each completed speech segment once. Whisper marks a chunk shorter than 15 seconds as `is_partial`; that flag is a duration hint, not an interim hypothesis that will be replaced later. Those completed segments are the finalized lines the webhook uses. A 20 second cooldown, plus remembering the segment id, keeps one mention from posting twice if the same segment is emitted again.

### Wake word

Default mode. When a finalized segment contains the wake word as a whole word (ignoring case), Meetily posts that segment plus the other lines from about the previous 2 minutes of recording time. Further matches in that same segment, and any match in the following 20 seconds, do not post again. A failed post is not retried.

### Interval

Every N minutes, Meetily posts the lines that are new since the last **successful** post. If nothing new has arrived, the tick is skipped. If the post fails, the cursor does not move, so the next interval includes those lines again. That later tick is the next scheduled post, not a retry loop.

## Payload

```json
{
  "meeting_id": "meeting-3f1c0c2e-7b4a-4e0d-9c2a-1b6e8a0d4f21",
  "meeting_title": "Meeting 2026-10-22_09-15-03",
  "reason": "wake_word",
  "lines": [
    {
      "speaker": "",
      "text": "Emma, can you summarize the last two minutes?",
      "audio_start_time": 125.3,
      "timestamp": "14:30:05"
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `meeting_id` | `meeting-<uuid>` assigned when the recording starts. A normal stop saves the meeting in SQLite under this same id. |
| `meeting_title` | Title used for the recording. |
| `reason` | `wake_word`, `interval`, or `test`. |
| `lines` | Transcript lines included in this post, in recording order. |
| `lines[].speaker` | Empty string. This edition mixes microphone and system audio before transcription and does not identify speakers. |
| `lines[].text` | Transcript text for the segment. |
| `lines[].audio_start_time` | Seconds from the start of the recording. |
| `lines[].timestamp` | Wall-clock `HH:MM:SS` already stored on the segment (UTC). |

A test payload uses `meeting_id` `meeting-test`, title `Webhook test`, reason `test`, and one sample line.
