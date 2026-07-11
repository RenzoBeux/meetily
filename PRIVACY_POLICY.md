# Murmur Privacy Policy

## Our Privacy-First Commitment

Murmur is built on the principle that your meeting data should remain private and under your control. This fork collects **no telemetry and no usage analytics whatsoever**, and does not phone home for any reason.

## Data Processing Philosophy

### Local-First Processing
- **Meeting transcription**: Processed entirely on your device using local Whisper/Parakeet models
- **Audio recordings**: Never transmitted to external servers
- **Meeting content**: Stored only in a local SQLite database on your machine
- **AI summaries**: Generated locally, or through an LLM provider you explicitly configure

### Your Data Ownership
- You own all meeting data, transcripts, and recordings
- Data is stored locally on your device
- No vendor lock-in — export your data anytime
- Complete control over data retention and deletion

## No Telemetry, No Analytics

This build contains **no analytics or telemetry code at all**. There is:
- ❌ No PostHog, Sentry, Segment, or any other analytics/telemetry SDK
- ❌ No usage tracking, event tracking, session tracking, or crash reporting
- ❌ No automatic update checks or version pings to any server
- ❌ No "opt-in" analytics toggle — there is simply nothing to opt into

The only network connections Murmur makes are ones **you** initiate, listed below.

## Network Connections (all user-initiated)

Murmur connects to the network only for these purposes, and only when you ask it to:

### Model downloads (one-time, to run locally)
- **Speech-to-text models** (Whisper, Parakeet) and **speaker-diarization models** are downloaded from their public model hosts the first time you use them, then run entirely offline.
- **Built-in summary models** are downloaded on demand and then run locally.

These are inbound downloads of open model weights — no personal data is sent.

### LLM providers (only if you configure one)
If you choose a cloud provider for summaries or chat, your transcript is sent to **that provider you selected**, subject to their privacy policy:
- **Anthropic Claude**, **OpenAI**, **Groq**, **OpenRouter**, or a **custom OpenAI-compatible endpoint** you specify.
- **Local providers** (Ollama, LM Studio) and the bundled local summary engine process everything on your machine — nothing leaves your device.

If you only ever use local providers, no meeting content ever leaves your computer.

## Data Security

- Data is stored locally using your device's file system permissions.
- No transmission of meeting data except to an LLM provider you explicitly choose.
- Full source code is available for review — you can verify every one of these claims.

## Contact

For privacy-related questions or concerns:
- **GitHub Issues**: [Create an issue](https://github.com/RenzoBeux/murmur/issues)

## Open Source Commitment

As an open-source project under the MIT license, you can review the complete implementation, verify there is no data collection, modify data handling to meet your requirements, and deploy entirely on your own infrastructure.
