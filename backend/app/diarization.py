"""
Speaker diarization on saved meeting audio using pyannote/speaker-diarization-3.1.

The pipeline is lazy-loaded on first request (model is ~1GB, downloaded from HF
into the user's HF cache on first use). Subsequent requests reuse the in-memory
pipeline. Diarization runs on CUDA when available, otherwise CPU.
"""

import asyncio
import glob
import logging
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Dict, List, Optional, Tuple

logger = logging.getLogger(__name__)


def _resolve_ffmpeg() -> Optional[str]:
    """Find an ffmpeg executable. Tries PATH first, then common Windows install
    locations. We can't always rely on PATH because the backend is often
    spawned by clean_start_backend.cmd's `start cmd /k`, which inherits the
    parent shell's PATH — and that PATH may have been frozen before the user
    installed ffmpeg.
    """
    found = shutil.which("ffmpeg")
    if found:
        return found

    if os.name == "nt":
        # winget (Gyan.FFmpeg) installs to %LOCALAPPDATA%\Microsoft\WinGet\Packages\Gyan.FFmpeg_*\ffmpeg-*\bin\ffmpeg.exe
        local_appdata = os.environ.get("LOCALAPPDATA")
        if local_appdata:
            patterns = [
                os.path.join(
                    local_appdata,
                    "Microsoft", "WinGet", "Packages",
                    "Gyan.FFmpeg*", "**", "ffmpeg.exe",
                ),
                os.path.join(
                    local_appdata,
                    "Microsoft", "WinGet", "Links", "ffmpeg.exe",
                ),
            ]
            for pattern in patterns:
                for match in glob.glob(pattern, recursive=True):
                    if os.path.isfile(match):
                        logger.info(f"Found ffmpeg via winget fallback: {match}")
                        return match

        # Other common Windows locations
        for candidate in (
            r"C:\ffmpeg\bin\ffmpeg.exe",
            r"C:\Program Files\ffmpeg\bin\ffmpeg.exe",
            r"C:\ProgramData\chocolatey\bin\ffmpeg.exe",
        ):
            if os.path.isfile(candidate):
                return candidate

    return None


def _ensure_wav(audio_path: str) -> Tuple[str, bool]:
    """Convert non-WAV audio to a temp WAV via ffmpeg so torchaudio can load it
    regardless of which backend it was built against. Returns (path, is_temp).
    The caller is responsible for deleting the file when is_temp is True.
    """
    ext = Path(audio_path).suffix.lower()
    if ext == ".wav":
        return audio_path, False

    ffmpeg_path = _resolve_ffmpeg()
    if not ffmpeg_path:
        raise RuntimeError(
            f"Audio file is {ext}, which needs ffmpeg to convert before "
            "diarization. Install ffmpeg (e.g. `winget install Gyan.FFmpeg`) "
            "and make sure it's on PATH or in a standard location."
        )

    fd, tmp = tempfile.mkstemp(suffix=".wav", prefix="meetily-diarize-")
    os.close(fd)
    try:
        # 16 kHz mono is what pyannote operates at internally; downsampling
        # here also speeds up I/O.
        subprocess.run(
            [
                ffmpeg_path, "-y", "-loglevel", "error",
                "-i", audio_path,
                "-ar", "16000", "-ac", "1",
                tmp,
            ],
            check=True,
            capture_output=True,
        )
        return tmp, True
    except subprocess.CalledProcessError as e:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise RuntimeError(
            f"ffmpeg failed to convert {audio_path}: {e.stderr.decode(errors='replace')}"
        )


class DiarizationService:
    def __init__(self) -> None:
        self._pipeline = None
        self._lock = asyncio.Lock()
        self._device_logged = False

    async def _ensure_pipeline(self, hf_token: str):
        async with self._lock:
            if self._pipeline is not None:
                return self._pipeline

            # Imported lazily so the rest of the backend works without torch
            # installed (e.g. while a user is still installing the new deps).
            from pyannote.audio import Pipeline
            import torch

            # Diagnostic: surface what this Python process actually sees so we
            # can tell the difference between "wrong torch installed" vs
            # "CUDA init failed at runtime" when the pipeline ends up on CPU.
            cuda_built = torch.backends.cuda.is_built()
            cuda_avail = torch.cuda.is_available()
            logger.info(
                f"torch version={torch.__version__}, "
                f"cuda_built_in={cuda_built}, cuda_available={cuda_avail}, "
                f"cuda_runtime={torch.version.cuda}, "
                f"device_count={torch.cuda.device_count() if cuda_built else 0}"
            )
            if cuda_built and not cuda_avail:
                # CUDA-capable wheel installed but runtime init failed. Try to
                # surface why — this raises with the underlying CUDA error.
                try:
                    torch.cuda.init()
                except Exception as e:
                    logger.warning(f"CUDA init failed: {e!r}")

            loop = asyncio.get_event_loop()
            pipeline = await loop.run_in_executor(
                None,
                lambda: Pipeline.from_pretrained(
                    "pyannote/speaker-diarization-3.1",
                    use_auth_token=hf_token,
                ),
            )
            if pipeline is None:
                raise RuntimeError(
                    "Pyannote pipeline failed to load. "
                    "Check your HuggingFace token and that you've accepted the "
                    "pyannote/speaker-diarization-3.1 model license."
                )

            if cuda_avail:
                pipeline.to(torch.device("cuda"))
                if not self._device_logged:
                    logger.info(
                        f"Diarization pipeline loaded on CUDA "
                        f"({torch.cuda.get_device_name(0)})"
                    )
                    self._device_logged = True
            else:
                if not self._device_logged:
                    logger.info(
                        "Diarization pipeline loaded on CPU "
                        f"(cuda_built_in={cuda_built}; "
                        + (
                            "CUDA wheel installed but runtime init failed — see warning above"
                            if cuda_built
                            else "you have a CPU-only torch wheel; reinstall with --index-url ...whl/cu121"
                        )
                        + ")"
                    )
                    self._device_logged = True

            self._pipeline = pipeline
            return self._pipeline

    async def diarize(
        self,
        audio_path: str,
        hf_token: str,
        num_speakers: Optional[int] = None,
        min_speakers: Optional[int] = None,
        max_speakers: Optional[int] = None,
    ) -> List[Dict]:
        """Run diarization on the audio file.

        When known, passing `num_speakers` (exact) or `min_speakers`/`max_speakers`
        (bounds) dramatically improves results — it stops the clustering step
        from over- or under-splitting voices.

        Returns a list of {start, end, speaker} dicts where `speaker` is the raw
        pyannote label (e.g. SPEAKER_00). Renumbering happens later in the
        mapping step so each meeting starts at speaker_1.
        """
        pipeline = await self._ensure_pipeline(hf_token)
        loop = asyncio.get_event_loop()

        wav_path, is_temp = await loop.run_in_executor(
            None, _ensure_wav, audio_path
        )

        # Build pyannote kwargs only with values the user actually set, so we
        # don't override pyannote's auto-detection with Nones.
        pipeline_kwargs: Dict = {}
        if num_speakers is not None:
            pipeline_kwargs["num_speakers"] = num_speakers
        else:
            if min_speakers is not None:
                pipeline_kwargs["min_speakers"] = min_speakers
            if max_speakers is not None:
                pipeline_kwargs["max_speakers"] = max_speakers

        if pipeline_kwargs:
            logger.info(f"Running diarization with hint: {pipeline_kwargs}")

        def _run():
            diarization = pipeline(wav_path, **pipeline_kwargs)
            return [
                {"start": float(turn.start), "end": float(turn.end), "speaker": speaker}
                for turn, _, speaker in diarization.itertracks(yield_label=True)
            ]

        try:
            return await loop.run_in_executor(None, _run)
        finally:
            if is_temp:
                try:
                    os.unlink(wav_path)
                except OSError:
                    pass


def map_segments_to_speakers(
    transcripts: List[Dict],
    diarize_segments: List[Dict],
) -> Dict[str, str]:
    """Assign each transcript a speaker label based on max overlap with the
    diarization output.

    Pyannote's labels (SPEAKER_00, SPEAKER_01, ...) are renumbered into
    speaker_1, speaker_2, ... in order of first appearance, so a meeting always
    starts at speaker_1 regardless of internal pyannote ordering.

    Returns {transcript_id: speaker_label}. Transcripts with no overlap (e.g.
    silence misclassified earlier) are simply omitted.
    """
    seen: Dict[str, str] = {}
    next_idx = 1

    def normalize(label: str) -> str:
        nonlocal next_idx
        if label not in seen:
            seen[label] = f"speaker_{next_idx}"
            next_idx += 1
        return seen[label]

    result: Dict[str, str] = {}
    for t in transcripts:
        t_start: Optional[float] = t.get("audio_start_time")
        t_end: Optional[float] = t.get("audio_end_time")
        if t_start is None or t_end is None:
            continue

        # Sum overlap per pyannote speaker
        overlaps: Dict[str, float] = {}
        for seg in diarize_segments:
            ov_start = max(t_start, seg["start"])
            ov_end = min(t_end, seg["end"])
            ov = ov_end - ov_start
            if ov > 0:
                overlaps[seg["speaker"]] = overlaps.get(seg["speaker"], 0.0) + ov

        if not overlaps:
            continue

        best_label, _ = max(overlaps.items(), key=lambda kv: kv[1])
        result[t["id"]] = normalize(best_label)

    return result
