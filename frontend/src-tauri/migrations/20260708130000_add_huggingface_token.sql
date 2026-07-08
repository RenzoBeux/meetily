-- Hugging Face access token for the gated pyannote community-1 model used by
-- "Local Pro" diarization (Settings → Transcript → Speaker identification)
ALTER TABLE transcript_settings ADD COLUMN huggingfaceToken TEXT;
