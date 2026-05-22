'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { toast } from 'sonner';
import { Transcript } from '@/types';
import {
  deleteSegments as apiDeleteSegments,
  insertSegments as apiInsertSegments,
  mergeSegments as apiMergeSegments,
  splitSegment as apiSplitSegment,
  updateSegmentBounds as apiUpdateSegmentBounds,
  updateSegmentSpeakers as apiUpdateSegmentSpeakers,
  updateSegmentText as apiUpdateSegmentText,
  type NewSegmentPayload,
} from '@/lib/transcriptEditorApi';

interface UseTranscriptEditorProps {
  transcripts: Transcript[];
  applyLocalMutation: (mutator: (prev: Transcript[]) => Transcript[]) => void;
  meetingId?: string;
}

export type MergeValidation =
  | { ok: true; segments: Transcript[]; speakers: string[] }
  | { ok: false; reason: string };

// Operations are stored on the undo/redo stacks. Each carries enough state to
// dispatch either direction; the "after" side holds what's currently visible.
type EditorOp =
  | { kind: 'edit_text'; id: string; before: string; after: string }
  | { kind: 'delete'; rows: Transcript[] }
  | {
      kind: 'reassign';
      before: Array<{ id: string; speaker?: string }>;
      after: string | null;
    }
  | {
      kind: 'merge';
      keeperBefore: Transcript;
      otherDeleted: Transcript[];
      mergedAfter: Transcript;
    }
  | {
      kind: 'split';
      sourceBefore: Transcript;
      sourceAfter: Transcript;
      tail: Transcript;
    };

function generateUUID(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function toPayload(row: Transcript): NewSegmentPayload {
  return {
    id: row.id,
    text: row.text,
    timestamp: row.timestamp,
    audio_start_time: row.audio_start_time,
    audio_end_time: row.audio_end_time,
    duration: row.duration,
    speaker: row.speaker,
  };
}

export interface TranscriptEditorState {
  isEditMode: boolean;
  selectedIds: Set<string>;
  editingId: string | null;
  knownSpeakers: string[];
  selectionCount: number;
  hasSelection: boolean;
  canUndo: boolean;
  canRedo: boolean;

  enterEditMode: () => void;
  exitEditMode: () => void;
  toggleSelect: (id: string, withShift?: boolean) => void;
  clearSelection: () => void;
  selectRange: (fromId: string, toId: string) => void;
  startEdit: (id: string) => void;
  cancelEdit: () => void;
  editText: (id: string, newText: string) => Promise<void>;
  deleteSelected: () => Promise<void>;
  deleteSegment: (id: string) => Promise<void>;
  reassignSpeakers: (ids: string[], speaker: string | null) => Promise<void>;
  validateMerge: (ids: string[]) => MergeValidation;
  mergeSegments: (ids: string[], speakerOverride?: string | null) => Promise<void>;
  splitSegment: (id: string, charOffset: number, currentText: string) => Promise<void>;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
}

export function useTranscriptEditor({
  transcripts,
  applyLocalMutation,
  meetingId,
}: UseTranscriptEditorProps): TranscriptEditorState {
  const [isEditMode, setIsEditMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [lastSelectedId, setLastSelectedId] = useState<string | null>(null);
  const [undoStack, setUndoStack] = useState<EditorOp[]>([]);
  const [redoStack, setRedoStack] = useState<EditorOp[]>([]);

  const knownSpeakers = useMemo(() => {
    const set = new Set<string>();
    for (const t of transcripts) {
      if (t.speaker && t.speaker.trim() !== '') set.add(t.speaker);
    }
    return Array.from(set);
  }, [transcripts]);

  const enterEditMode = useCallback(() => setIsEditMode(true), []);
  const exitEditMode = useCallback(() => {
    setIsEditMode(false);
    setSelectedIds(new Set());
    setEditingId(null);
    setLastSelectedId(null);
    setUndoStack([]);
    setRedoStack([]);
  }, []);

  const clearSelection = useCallback(() => {
    setSelectedIds(new Set());
    setLastSelectedId(null);
  }, []);

  const selectRange = useCallback(
    (fromId: string, toId: string) => {
      const ids = transcripts.map((t) => t.id);
      const fromIdx = ids.indexOf(fromId);
      const toIdx = ids.indexOf(toId);
      if (fromIdx === -1 || toIdx === -1) return;
      const [lo, hi] = fromIdx <= toIdx ? [fromIdx, toIdx] : [toIdx, fromIdx];
      setSelectedIds((prev) => {
        const next = new Set(prev);
        for (let i = lo; i <= hi; i++) next.add(ids[i]);
        return next;
      });
    },
    [transcripts],
  );

  const toggleSelect = useCallback(
    (id: string, withShift: boolean = false) => {
      if (withShift && lastSelectedId && lastSelectedId !== id) {
        selectRange(lastSelectedId, id);
        setLastSelectedId(id);
        return;
      }
      setSelectedIds((prev) => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
      setLastSelectedId(id);
    },
    [lastSelectedId, selectRange],
  );

  const startEdit = useCallback((id: string) => setEditingId(id), []);
  const cancelEdit = useCallback(() => setEditingId(null), []);

  // Keep transcripts in a ref so dispatchers can read the latest snapshot
  // without re-creating the dispatch function on every keystroke.
  const transcriptsRef = useRef(transcripts);
  useEffect(() => {
    transcriptsRef.current = transcripts;
  }, [transcripts]);
  const meetingIdRef = useRef(meetingId);
  useEffect(() => {
    meetingIdRef.current = meetingId;
  }, [meetingId]);

  // Forward / backward dispatchers. Each returns void on success and throws on
  // failure (so the caller can decide whether to rollback or surface a toast).

  const applyEditText = useCallback(
    async (op: EditorOp & { kind: 'edit_text' }, dir: 'forward' | 'backward') => {
      const id = op.id;
      const target = dir === 'forward' ? op.after : op.before;
      applyLocalMutation((rows) => rows.map((r) => (r.id === id ? { ...r, text: target } : r)));
      await apiUpdateSegmentText(id, target);
    },
    [applyLocalMutation],
  );

  const applyDelete = useCallback(
    async (op: EditorOp & { kind: 'delete' }, dir: 'forward' | 'backward') => {
      const ids = op.rows.map((r) => r.id);
      if (dir === 'forward') {
        applyLocalMutation((rows) => rows.filter((r) => !ids.includes(r.id)));
        await apiDeleteSegments(ids);
      } else {
        applyLocalMutation((rows) => {
          const next = [...rows];
          // Re-insert into sorted position by audio_start_time.
          for (const row of op.rows) {
            const ts = row.audio_start_time ?? 0;
            const insertIdx = next.findIndex((r) => (r.audio_start_time ?? 0) > ts);
            if (insertIdx === -1) next.push(row);
            else next.splice(insertIdx, 0, row);
          }
          return next;
        });
        if (!meetingIdRef.current) throw new Error('No meeting context for restore');
        await apiInsertSegments(meetingIdRef.current, op.rows.map(toPayload));
      }
    },
    [applyLocalMutation],
  );

  const applyReassign = useCallback(
    async (op: EditorOp & { kind: 'reassign' }, dir: 'forward' | 'backward') => {
      const ids = op.before.map((b) => b.id);
      if (dir === 'forward') {
        const next = op.after ?? undefined;
        const idSet = new Set(ids);
        applyLocalMutation((rows) =>
          rows.map((r) => (idSet.has(r.id) ? { ...r, speaker: next } : r)),
        );
        await apiUpdateSegmentSpeakers(ids.map((id) => [id, op.after]));
      } else {
        const map = new Map(op.before.map((b) => [b.id, b.speaker]));
        applyLocalMutation((rows) =>
          rows.map((r) => (map.has(r.id) ? { ...r, speaker: map.get(r.id) } : r)),
        );
        await apiUpdateSegmentSpeakers(op.before.map((b) => [b.id, b.speaker ?? null]));
      }
    },
    [applyLocalMutation],
  );

  const applyMerge = useCallback(
    async (op: EditorOp & { kind: 'merge' }, dir: 'forward' | 'backward') => {
      const keeperId = op.keeperBefore.id;
      const otherIds = op.otherDeleted.map((r) => r.id);
      if (dir === 'forward') {
        const deletedSet = new Set(otherIds);
        applyLocalMutation((rows) =>
          rows
            .map((r) => (r.id === keeperId ? op.mergedAfter : r))
            .filter((r) => !deletedSet.has(r.id)),
        );
        await apiMergeSegments({
          keeperId,
          mergedText: op.mergedAfter.text,
          audioEndTime: op.mergedAfter.audio_end_time ?? 0,
          duration: op.mergedAfter.duration ?? 0,
          speaker: op.mergedAfter.speaker ?? null,
          deletedIds: otherIds,
        });
      } else {
        applyLocalMutation((rows) => {
          // Restore keeper to its pre-merge state and re-insert other rows.
          const next = rows.map((r) => (r.id === keeperId ? op.keeperBefore : r));
          for (const row of op.otherDeleted) {
            const ts = row.audio_start_time ?? 0;
            const insertIdx = next.findIndex((r) => (r.audio_start_time ?? 0) > ts);
            if (insertIdx === -1) next.push(row);
            else next.splice(insertIdx, 0, row);
          }
          return next;
        });
        if (!meetingIdRef.current) throw new Error('No meeting context for restore');
        await apiInsertSegments(meetingIdRef.current, op.otherDeleted.map(toPayload));
        // Restore keeper bounds + speaker.
        await apiUpdateSegmentBounds({
          segmentId: keeperId,
          newText: op.keeperBefore.text,
          audioEndTime: op.keeperBefore.audio_end_time ?? 0,
          duration: op.keeperBefore.duration ?? 0,
        });
        await apiUpdateSegmentSpeakers([[keeperId, op.keeperBefore.speaker ?? null]]);
      }
    },
    [applyLocalMutation],
  );

  const applySplit = useCallback(
    async (op: EditorOp & { kind: 'split' }, dir: 'forward' | 'backward') => {
      const sourceId = op.sourceBefore.id;
      if (dir === 'forward') {
        applyLocalMutation((rows) => {
          const next = rows.map((r) => (r.id === sourceId ? op.sourceAfter : r));
          const idx = next.findIndex((r) => r.id === sourceId);
          const insertAt = idx === -1 ? next.length : idx + 1;
          next.splice(insertAt, 0, op.tail);
          return next;
        });
        if (!meetingIdRef.current) throw new Error('No meeting context for split');
        await apiSplitSegment({
          meetingId: meetingIdRef.current,
          sourceId,
          headText: op.sourceAfter.text,
          headEndTime: op.sourceAfter.audio_end_time ?? 0,
          headDuration: op.sourceAfter.duration ?? 0,
          tail: toPayload(op.tail),
        });
      } else {
        applyLocalMutation((rows) =>
          rows.filter((r) => r.id !== op.tail.id).map((r) => (r.id === sourceId ? op.sourceBefore : r)),
        );
        await apiDeleteSegments([op.tail.id]);
        await apiUpdateSegmentBounds({
          segmentId: sourceId,
          newText: op.sourceBefore.text,
          audioEndTime: op.sourceBefore.audio_end_time ?? 0,
          duration: op.sourceBefore.duration ?? 0,
        });
      }
    },
    [applyLocalMutation],
  );

  const dispatch = useCallback(
    async (op: EditorOp, dir: 'forward' | 'backward'): Promise<void> => {
      switch (op.kind) {
        case 'edit_text':
          return applyEditText(op, dir);
        case 'delete':
          return applyDelete(op, dir);
        case 'reassign':
          return applyReassign(op, dir);
        case 'merge':
          return applyMerge(op, dir);
        case 'split':
          return applySplit(op, dir);
      }
    },
    [applyEditText, applyDelete, applyReassign, applyMerge, applySplit],
  );

  // Run a user-initiated op: dispatch forward, push to undo on success, clear redo.
  // On failure, dispatch the inverse to roll local state back and toast.
  const runUserOp = useCallback(
    async (op: EditorOp, errorTitle: string) => {
      try {
        await dispatch(op, 'forward');
        setUndoStack((s) => [...s, op]);
        setRedoStack([]);
      } catch (err) {
        console.error(errorTitle, err);
        // Try to revert local state by replaying the backward direction
        // without hitting the server (server never succeeded).
        try {
          // Best-effort local rollback: we re-apply the backward local update
          // by composing with the latest snapshot — but the forward already
          // mutated local state, so dispatch backward will write the inverse
          // both locally AND remotely. The remote write is harmless if the
          // remote forward failed (it will just succeed or no-op). Worst case
          // a toast tells the user.
          await dispatch(op, 'backward');
        } catch (rollbackErr) {
          console.error('Rollback also failed:', rollbackErr);
        }
        toast.error(errorTitle, {
          description: String(err ?? 'Unknown error'),
        });
      }
    },
    [dispatch],
  );

  const editText = useCallback(
    async (id: string, newText: string) => {
      const prev = transcriptsRef.current.find((t) => t.id === id);
      if (!prev) {
        setEditingId(null);
        return;
      }
      if (prev.text === newText) {
        setEditingId(null);
        return;
      }
      // If the user blanked out the text entirely, treat the commit as a
      // delete (with confirm). The backend allows empty strings but rendering
      // them as "[Silence]" rows is rarely what the user wants.
      if (newText.trim() === '') {
        setEditingId(null);
        const confirmed = typeof window !== 'undefined' && window.confirm(
          'The segment is now empty. Delete it instead?',
        );
        if (confirmed) {
          await runUserOp({ kind: 'delete', rows: [prev] }, 'Could not delete segment');
        }
        return;
      }
      setEditingId(null);
      await runUserOp(
        { kind: 'edit_text', id, before: prev.text, after: newText },
        'Could not save edit',
      );
    },
    [runUserOp],
  );

  const deleteSegment = useCallback(
    async (id: string) => {
      const prev = transcriptsRef.current.find((t) => t.id === id);
      if (!prev) return;
      setSelectedIds((s) => {
        if (!s.has(id)) return s;
        const next = new Set(s);
        next.delete(id);
        return next;
      });
      if (editingId === id) setEditingId(null);
      await runUserOp({ kind: 'delete', rows: [prev] }, 'Could not delete segment');
    },
    [editingId, runUserOp],
  );

  const deleteSelected = useCallback(async () => {
    const ids = Array.from(selectedIds);
    if (ids.length === 0) return;
    const rows = transcriptsRef.current.filter((t) => ids.includes(t.id));
    if (rows.length === 0) return;
    setSelectedIds(new Set());
    if (editingId && ids.includes(editingId)) setEditingId(null);
    await runUserOp({ kind: 'delete', rows }, 'Could not delete segments');
  }, [selectedIds, editingId, runUserOp]);

  const reassignSpeakers = useCallback(
    async (ids: string[], speaker: string | null) => {
      if (ids.length === 0) return;
      const idSet = new Set(ids);
      const before = transcriptsRef.current
        .filter((t) => idSet.has(t.id))
        .map((t) => ({ id: t.id, speaker: t.speaker }));
      if (before.length === 0) return;
      await runUserOp(
        { kind: 'reassign', before, after: speaker },
        'Could not update speaker',
      );
    },
    [runUserOp],
  );

  const validateMerge = useCallback(
    (ids: string[]): MergeValidation => {
      if (ids.length < 2) {
        return { ok: false, reason: 'Select at least two segments to merge.' };
      }
      const sorted = [...transcriptsRef.current].sort(
        (a, b) => (a.audio_start_time ?? 0) - (b.audio_start_time ?? 0),
      );
      const positions = ids
        .map((id) => sorted.findIndex((s) => s.id === id))
        .sort((a, b) => a - b);
      if (positions.some((p) => p === -1)) {
        return { ok: false, reason: 'Some selected segments could not be found.' };
      }
      for (let i = 1; i < positions.length; i++) {
        if (positions[i] !== positions[i - 1] + 1) {
          return {
            ok: false,
            reason: 'Selected segments must be contiguous to merge.',
          };
        }
      }
      const segments = positions.map((p) => sorted[p]);
      if (segments.some((s) => s.audio_start_time === undefined || s.audio_end_time === undefined)) {
        return { ok: false, reason: 'Some selected segments are missing timing info.' };
      }
      const speakers = Array.from(
        new Set(segments.map((s) => s.speaker).filter((s): s is string => !!s)),
      );
      return { ok: true, segments, speakers };
    },
    [],
  );

  const mergeSegments = useCallback(
    async (ids: string[], speakerOverride?: string | null) => {
      const validation = validateMerge(ids);
      if (!validation.ok) {
        toast.error('Cannot merge', { description: validation.reason });
        return;
      }
      const { segments } = validation;
      const keeper = segments[0];
      const last = segments[segments.length - 1];
      const mergedText = segments
        .map((s) => s.text.trim())
        .filter((t) => t.length > 0)
        .join(' ');
      const audioStart = keeper.audio_start_time ?? 0;
      const audioEnd = last.audio_end_time ?? audioStart;
      const duration = audioEnd - audioStart;
      const speaker =
        speakerOverride !== undefined
          ? speakerOverride
          : validation.speakers.length <= 1
            ? validation.speakers[0] ?? null
            : null;

      const mergedAfter: Transcript = {
        ...keeper,
        text: mergedText,
        audio_end_time: audioEnd,
        duration,
        speaker: speaker ?? undefined,
      };
      const op: EditorOp = {
        kind: 'merge',
        keeperBefore: keeper,
        otherDeleted: segments.slice(1),
        mergedAfter,
      };
      setSelectedIds(new Set());
      await runUserOp(op, 'Could not merge segments');
    },
    [validateMerge, runUserOp],
  );

  const splitSegment = useCallback(
    async (id: string, charOffset: number, currentText: string) => {
      if (!meetingIdRef.current) {
        toast.error('Cannot split — meeting not loaded');
        return;
      }
      const source = transcriptsRef.current.find((t) => t.id === id);
      if (!source) return;
      if (charOffset <= 0 || charOffset >= currentText.length) {
        toast.error('Place the caret inside the segment text to split.');
        return;
      }
      if (source.audio_start_time === undefined || source.audio_end_time === undefined) {
        toast.error('Cannot split — segment is missing timing info.');
        return;
      }
      const headText = currentText.slice(0, charOffset).trim();
      const tailText = currentText.slice(charOffset).trim();
      if (!headText || !tailText) {
        toast.error('Caret must be between two words.');
        return;
      }

      // Interpolate timestamps by character ratio (matches Whisper's coarse
      // segment-boundary precision).
      const ratio = charOffset / currentText.length;
      const totalDuration = source.audio_end_time - source.audio_start_time;
      const headEndTime = source.audio_start_time + totalDuration * ratio;
      const headDuration = headEndTime - source.audio_start_time;
      const tailStart = headEndTime;
      const tailEnd = source.audio_end_time;
      const tailDuration = tailEnd - tailStart;

      const tailId = generateUUID();
      const tail: Transcript = {
        id: tailId,
        text: tailText,
        timestamp: source.timestamp,
        audio_start_time: tailStart,
        audio_end_time: tailEnd,
        duration: tailDuration,
        speaker: source.speaker,
        confidence: source.confidence,
      };
      const sourceAfter: Transcript = {
        ...source,
        text: headText,
        audio_end_time: headEndTime,
        duration: headDuration,
      };
      setEditingId(null);
      await runUserOp(
        { kind: 'split', sourceBefore: source, sourceAfter, tail },
        'Could not split segment',
      );
    },
    [runUserOp],
  );

  const undo = useCallback(async () => {
    const op = undoStack[undoStack.length - 1];
    if (!op) return;
    setUndoStack((s) => s.slice(0, -1));
    try {
      await dispatch(op, 'backward');
      setRedoStack((s) => [...s, op]);
    } catch (err) {
      console.error('Undo failed:', err);
      // Put the op back on the undo stack since the inverse didn't take effect.
      setUndoStack((s) => [...s, op]);
      toast.error('Undo failed', {
        description: String(err ?? 'Unknown error'),
      });
    }
  }, [undoStack, dispatch]);

  const redo = useCallback(async () => {
    const op = redoStack[redoStack.length - 1];
    if (!op) return;
    setRedoStack((s) => s.slice(0, -1));
    try {
      await dispatch(op, 'forward');
      setUndoStack((s) => [...s, op]);
    } catch (err) {
      console.error('Redo failed:', err);
      setRedoStack((s) => [...s, op]);
      toast.error('Redo failed', {
        description: String(err ?? 'Unknown error'),
      });
    }
  }, [redoStack, dispatch]);

  return {
    isEditMode,
    selectedIds,
    editingId,
    knownSpeakers,
    selectionCount: selectedIds.size,
    hasSelection: selectedIds.size > 0,
    canUndo: undoStack.length > 0,
    canRedo: redoStack.length > 0,
    enterEditMode,
    exitEditMode,
    toggleSelect,
    clearSelection,
    selectRange,
    startEdit,
    cancelEdit,
    editText,
    deleteSelected,
    deleteSegment,
    reassignSpeakers,
    validateMerge,
    mergeSegments,
    splitSegment,
    undo,
    redo,
  };
}
