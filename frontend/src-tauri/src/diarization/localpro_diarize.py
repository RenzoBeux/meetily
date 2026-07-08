# Meetily "Local Pro" diarization sidecar.
#
# Runs the pyannote community-1 pipeline fully locally and prints the speaker
# segments as JSON on stdout (last line). All human-readable progress goes to
# stderr. Spawned per job by the Tauri app via `uv run` — no resident server.
#
# argv[1]: JSON {"wav_path": str, "num_speakers": int | null}
# env:     HF_TOKEN (required: pyannote/speaker-diarization-community-1 is a
#          gated model), HF_HOME (model cache location)

import json
import os
import sys

MODEL = "pyannote/speaker-diarization-community-1"


def log(msg):
    print(msg, file=sys.stderr, flush=True)


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

    output = pipeline(wav_path, **kwargs)
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
