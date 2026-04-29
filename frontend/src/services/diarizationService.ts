import { invoke } from '@tauri-apps/api/core';

export interface DiarizeMeetingResult {
  updated: number;
  total_speakers: number;
  diarize_segment_count: number;
}

export interface DiarizationOptions {
  hfToken?: string;
  /** Exact speaker count. Pins clustering — overrides min/max if set. */
  numSpeakers?: number;
  /** Lower bound on the speaker count (used only if numSpeakers is unset). */
  minSpeakers?: number;
  /** Upper bound on the speaker count (used only if numSpeakers is unset). */
  maxSpeakers?: number;
}

/**
 * Trigger speaker diarization for a saved meeting via the Tauri backend, which
 * forwards to the FastAPI backend running pyannote and then writes the
 * resulting speaker tags into the local SQLite.
 *
 * The HF token is optional: if omitted, the backend falls back to the
 * HF_TOKEN environment variable. The token must have access to
 * pyannote/speaker-diarization-3.1.
 *
 * Passing a known speaker count via `numSpeakers` (or bounds via
 * min/maxSpeakers) usually improves results — pyannote can otherwise over- or
 * under-split voices.
 */
export async function runDiarization(
  meetingId: string,
  options: DiarizationOptions = {},
): Promise<DiarizeMeetingResult> {
  return invoke<DiarizeMeetingResult>('api_diarize_meeting', {
    meetingId,
    hfToken: options.hfToken || null,
    numSpeakers: options.numSpeakers ?? null,
    minSpeakers: options.minSpeakers ?? null,
    maxSpeakers: options.maxSpeakers ?? null,
  });
}

const HF_TOKEN_STORAGE_KEY = 'meetily.hf_token';

export function loadHfToken(): string | null {
  if (typeof window === 'undefined') return null;
  try {
    return localStorage.getItem(HF_TOKEN_STORAGE_KEY);
  } catch {
    return null;
  }
}

export function saveHfToken(token: string): void {
  if (typeof window === 'undefined') return;
  try {
    if (token.trim()) {
      localStorage.setItem(HF_TOKEN_STORAGE_KEY, token.trim());
    } else {
      localStorage.removeItem(HF_TOKEN_STORAGE_KEY);
    }
  } catch {
    /* ignore */
  }
}

/**
 * Reassign a single transcript segment to a different speaker. Used when
 * diarization made a mistake on one chunk.
 */
export async function updateTranscriptSpeaker(
  transcriptId: string,
  speaker: string,
): Promise<void> {
  return invoke('api_update_transcript_speaker', {
    transcriptId,
    speaker,
  });
}

/**
 * Rename a speaker across every segment in a meeting. Used to give a
 * diarization-assigned ID (e.g. "speaker_1") a real name like "Alice".
 */
export async function renameSpeakerInMeeting(
  meetingId: string,
  oldSpeaker: string,
  newSpeaker: string,
): Promise<{ updated: number }> {
  return invoke<{ updated: number }>('api_rename_speaker_in_meeting', {
    meetingId,
    oldSpeaker,
    newSpeaker,
  });
}
