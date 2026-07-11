# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "pyannote.audio>=4.0,<5",
#     "soundfile>=0.12",
# ]
#
# [tool.uv]
# # Pick the PyTorch build that matches this machine: a CUDA wheel on NVIDIA
# # (Windows/Linux), the default MPS-capable wheel on macOS, a CPU wheel
# # otherwise. Without this uv installs the CPU-only torch and diarization
# # always runs on CPU. The script's own device detection then selects
# # cuda / mps / cpu accordingly.
# torch-backend = "auto"
# ///

# Murmur "Local Pro" diarization sidecar.
#
# Runs the pyannote community-1 pipeline fully locally and prints the speaker
# segments as JSON on stdout (last line). All human-readable progress goes to
# stderr. Spawned per job by the Tauri app via `uv run` — no resident server.
# Dependencies and the GPU torch-backend are declared in the PEP 723 block
# above, so `uv run` provisions the environment straight from this file.
#
# argv[1]: JSON {"wav_path": str, "num_speakers": int | null}
# env:     HF_TOKEN (required: pyannote/speaker-diarization-community-1 is a
#          gated model), HF_HOME (model cache location)

import json
import os
import sys
import warnings

MODEL = "pyannote/speaker-diarization-community-1"

# pyannote probes torchcodec when its audio-IO module is imported and, when the
# load fails (no FFmpeg shared libraries on Windows), emits a ~60-line UserWarning
# with a full traceback embedded in the message. We never let pyannote decode
# files — load_audio() hands it an in-memory waveform — so that probe is
# irrelevant noise here. Silence just that one warning. A real decode failure
# would *raise* (as it did before we switched to waveform input), not warn, so
# this hides nothing that matters. The `(?s)` lets `.` span the message's
# leading newline.
warnings.filterwarnings(
    "ignore",
    message=r"(?s).*torchcodec is not installed correctly",
    category=UserWarning,
)


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def load_audio(wav_path):
    """Decode the WAV ourselves and hand pyannote an in-memory waveform.

    pyannote.audio 4.x decodes file paths via torchcodec, which needs FFmpeg
    shared libraries present at runtime (a fragile, version-sensitive setup on
    Windows). Passing a {'waveform', 'sample_rate'} dict takes the decode path
    out of pyannote's hands entirely, so it works the same on every platform.
    """
    import numpy as np
    import soundfile as sf
    import torch

    # (time, channels), float32 in [-1, 1] for PCM; raw floats for float WAV.
    data, sample_rate = sf.read(wav_path, dtype="float32", always_2d=True)
    # Diarization operates on mono; average channels (no-op if already mono).
    mono = data.mean(axis=1, dtype="float32")
    waveform = torch.from_numpy(np.ascontiguousarray(mono)).unsqueeze(0)
    return {"waveform": waveform, "sample_rate": int(sample_rate)}


def main():
    args = json.loads(sys.argv[1])
    wav_path = args["wav_path"]
    num_speakers = args.get("num_speakers")

    token = os.environ.get("HF_TOKEN")
    if not token:
        log("ERROR: HF_TOKEN is not set")
        sys.exit(2)

    log("loading pipeline (first run downloads the model)...")
    import torch
    from pyannote.audio import Pipeline

    try:
        pipeline = Pipeline.from_pretrained(MODEL, token=token)
    except TypeError:
        # Older pyannote.audio versions use the use_auth_token kwarg.
        pipeline = Pipeline.from_pretrained(MODEL, use_auth_token=token)
    if pipeline is None:
        log(
            "ERROR: could not load the pipeline. Make sure you accepted the "
            f"model conditions at https://huggingface.co/{MODEL} and that the "
            "token has read access."
        )
        sys.exit(3)

    if torch.backends.mps.is_available():
        device = "mps"
    elif torch.cuda.is_available():
        device = "cuda"
    else:
        device = "cpu"
    pipeline.to(torch.device(device))
    log(f"running on {device}")

    kwargs = {}
    if num_speakers:
        kwargs["num_speakers"] = int(num_speakers)

    audio_input = load_audio(wav_path)
    output = pipeline(audio_input, **kwargs)
    # pyannote.audio 4.x wraps the Annotation in an output object; 3.x returns
    # the Annotation directly.
    annotation = getattr(output, "speaker_diarization", output)
    segments = [
        {"speaker": label, "start": float(turn.start), "end": float(turn.end)}
        for turn, _, label in annotation.itertracks(yield_label=True)
    ]
    log(f"done: {len(segments)} segments")
    print(json.dumps({"segments": segments}), flush=True)


if __name__ == "__main__":
    main()
