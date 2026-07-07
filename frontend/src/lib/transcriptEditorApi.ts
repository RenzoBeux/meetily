import { invoke } from '@tauri-apps/api/core';

export interface NewSegmentPayload {
  id: string;
  text: string;
  timestamp: string;
  audio_start_time?: number;
  audio_end_time?: number;
  duration?: number;
  speaker?: string;
}

export async function updateSegmentText(segmentId: string, newText: string): Promise<void> {
  await invoke('api_update_segment_text', { segmentId, newText });
}

export async function deleteSegments(segmentIds: string[]): Promise<number> {
  return invoke<number>('api_delete_segments', { segmentIds });
}

export async function updateSegmentSpeakers(
  updates: Array<[string, string | null]>,
): Promise<number> {
  return invoke<number>('api_update_segment_speakers', { updates });
}

export interface MergeSegmentsArgs {
  keeperId: string;
  mergedText: string;
  audioEndTime: number;
  duration: number;
  speaker: string | null;
  deletedIds: string[];
}

export async function mergeSegments(args: MergeSegmentsArgs): Promise<void> {
  await invoke('api_merge_segments', { ...args });
}

export interface SplitSegmentArgs {
  meetingId: string;
  sourceId: string;
  headText: string;
  headEndTime: number;
  headDuration: number;
  tail: NewSegmentPayload;
}

export async function splitSegment(args: SplitSegmentArgs): Promise<NewSegmentPayload> {
  return invoke<NewSegmentPayload>('api_split_segment', { ...args });
}

export async function insertSegments(
  meetingId: string,
  segments: NewSegmentPayload[],
): Promise<number> {
  return invoke<number>('api_insert_segments', { meetingId, segments });
}

export interface UpdateSegmentBoundsArgs {
  segmentId: string;
  newText: string;
  audioEndTime: number;
  duration: number;
}

export async function updateSegmentBounds(args: UpdateSegmentBoundsArgs): Promise<void> {
  await invoke('api_update_segment_bounds', { ...args });
}
