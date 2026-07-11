# Roadmap

Ideas validated in discussion but deferred until there is time to test them properly.

## 1. Active-speaker capture via browser extension (Meet + Teams web)

**Status:** idea — needs a spike. **Priority:** high (would qualitatively change speaker attribution).

### The idea

For online meetings, the meeting platform already knows who is speaking — acoustic diarization re-derives information the UI is displaying. Meet and Teams (web) render live **captions that carry the speaker's display name**. A small companion browser extension can read them:

1. Content-script adapters (one for Meet's caption DOM, one for Teams web's) emit `(timestamp, speaker_name, caption_text)` events plus the participant list.
2. The extension streams events to the Murmur desktop app over a **localhost WebSocket** (Tauri hosts the server). Everything stays on-device — no change to the privacy story.
3. Murmur keeps its own Whisper transcription for quality and uses the caption events as a **speaker timeline**: align caption timestamps against transcript segments and replace `speaker_1`/`Others` with real names.
4. The participant list auto-fills the meeting's attendees roster (which already feeds summary/chat prompts and the diarization speaker-count prefill).
5. Acoustic diarization remains the fallback for in-person meetings and non-browser apps. Metadata-based attribution is *right or absent* — it never invents a "Speaker 3".

### Prior art

- [Recall.ai — "How I built a botless meeting recorder from scratch"](https://www.recall.ai/blog/how-i-built-a-botless-meeting-recorder-from-scratch) — caption-DOM scraping for Meet; candid about brittleness; never generalized to Zoom/Teams.
- [IceCubes](https://icecubes.app/blog/what-is-botless-meeting-transcription) — commercial proof the browser-extension caption approach works in production across Meet/Zoom/Teams web.
- [pasrom/meeting-transcriber](https://github.com/pasrom/meeting-transcriber) (MIT, Swift/macOS) — uses the macOS Accessibility API for Teams participant/mute info; closest open-source precedent for the *native-app* route (which we deliberately skip: browser covers Meet + Teams for our use case).

### Spike checklist (~1 evening)

- [ ] Join a Meet call with captions on; in DevTools confirm caption nodes reliably carry the speaker display name; note selectors and update cadence.
- [ ] Same for Teams in the browser (check whether org policy allows captions; note the caption DOM shape).
- [ ] Check caption timing skew vs. wall clock — is segment-level alignment (±2 s) feasible?
- [ ] Minimal extension → `ws://localhost` → console log round-trip while a recording runs.

### Risks / costs

- Meet/Teams DOM changes will periodically break adapters (contained: two adapters, few hundred lines each).
- Captions must be enabled in the meeting; Teams orgs can restrict them by policy.
- New distribution surface (users install an extension alongside the app).
- Timestamp alignment between browser events and the audio pipeline clock needs care — align at segment level, not word level.

## 2. Diarization follow-ups

- **pyannoteAI voiceprint speaker identification** — the cloud API supports voiceprints ([docs](https://docs.pyannote.ai/tutorials/identification-with-voiceprints.md)). Combined with the attendees roster, transcripts could get *real names* ("Lean:") instead of `Speaker 1` even without the browser extension. Deferred until the plain cloud diarization path proves itself.
- **Process isolation for local diarization** — sherpa-onnx / ONNX Runtime runs in-process; a native abort kills the whole app (why auto-diarization was removed; manual runs are still exposed). Move the local engine into a helper process (same pattern as `llama-helper`) so a crash costs one job, not the app. Then auto-diarization after recording could safely return.
- ~~**pyannote community-1**~~ — **shipped** as the "Local Pro" provider: the full community-1 pipeline runs in an on-demand Python sidecar (`uv`-provisioned env under app data, spawned per job, crash-isolated). The narrower idea of swapping community-1's *segmentation* ONNX into the sherpa-onnx pipeline remains open as a lighter-weight default upgrade, if sherpa-onnx ever supports it officially.
- **FLAC upload for cloud diarization** — the masked 16 kHz mono PCM16 WAV is ~115 MB per meeting hour; FLAC would roughly halve it (ffmpeg is already bundled). Do when long meetings hit upload pain.
- **Cancellation for diarization jobs** — neither local nor cloud runs can be cancelled mid-flight today; the cloud path has a 30-minute timeout as a backstop. Wire a `CancellationToken` like `summary/llm_client.rs` if users hit it.
