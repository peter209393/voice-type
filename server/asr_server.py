"""Local ASR HTTP server backed by faster-whisper (CTranslate2).

Exposes an OpenAI-compatible endpoint so the Rust client can POST a WAV
multipart `file` and get back `{"text": "..."}`.

Run with uv (isolated Python 3.12, avoids system Python 3.14 wheel gaps):

    uv run --python 3.12 \
        --with fastapi --with "uvicorn[standard]" \
        --with "faster-whisper" --with python-multipart \
        uvicorn asr_server:app --host 127.0.0.1 --port 8000

Tuned for CPU-only machines: device="cpu", compute_type="int8".
"""

from __future__ import annotations

import io
import os
from typing import Optional

from fastapi import FastAPI, File, UploadFile
from fastapi.responses import JSONResponse

from faster_whisper import WhisperModel

ASR_MODEL = os.environ.get("VT_ASR_MODEL", "medium")
ASR_DEVICE = os.environ.get("VT_ASR_DEVICE", "cpu")
ASR_COMPUTE_TYPE = os.environ.get("VT_ASR_COMPUTE_TYPE", "int8")

app = FastAPI(title="voice-type asr")

_model: Optional[WhisperModel] = None


def get_model() -> WhisperModel:
    global _model
    if _model is None:
        print(
            f"[asr] loading model='{ASR_MODEL}' "
            f"device='{ASR_DEVICE}' compute_type='{ASR_COMPUTE_TYPE}' ..."
        )
        _model = WhisperModel(
            ASR_MODEL, device=ASR_DEVICE, compute_type=ASR_COMPUTE_TYPE
        )
        print("[asr] model ready")
    return _model


@app.get("/healthz")
def healthz() -> dict:
    return {"status": "ok", "model": ASR_MODEL}


@app.post("/v1/audio/transcriptions")
async def transcriptions(file: UploadFile = File(...)) -> JSONResponse:
    data = await file.read()
    if not data:
        return JSONResponse({"text": ""})

    segments, _info = get_model().transcribe(
        io.BytesIO(data),
        vad_filter=True,
        beam_size=1,
        language=None,
        task="transcribe",
    )
    # segments is a generator; materialize it.
    text = "".join(seg.text for seg in segments).strip()
    return JSONResponse({"text": text})


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(
        "asr_server:app",
        host=os.environ.get("VT_ASR_HOST", "127.0.0.1"),
        port=int(os.environ.get("VT_ASR_PORT", "8000")),
    )
