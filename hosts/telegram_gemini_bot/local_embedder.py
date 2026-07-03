"""Local embedding helper for the Telegram host.

This module is host-side policy. The Rust core receives vectors; it never
loads an embedding model or calls an embedding provider.
"""

from __future__ import annotations

import math
import time
from dataclasses import dataclass
from typing import Iterable


DEFAULT_EMBEDDING_MODEL = "intfloat/multilingual-e5-small"
DEFAULT_EMBEDDING_DIM = 384


class LocalEmbedderUnavailable(RuntimeError):
    pass


@dataclass(frozen=True)
class EmbeddingTelemetry:
    model_id: str
    dim: int
    count: int
    duration_ms: int


class LocalEmbedder:
    def __init__(
        self,
        model_id: str = DEFAULT_EMBEDDING_MODEL,
        dim: int = DEFAULT_EMBEDDING_DIM,
    ) -> None:
        self.model_id = model_id
        self.dim = dim
        self._model = None

    def embed_passages(self, texts: Iterable[str]) -> tuple[list[list[float]], EmbeddingTelemetry]:
        return self._embed([f"passage: {text}" for text in texts])

    def embed_query(self, text: str) -> tuple[list[float], EmbeddingTelemetry]:
        vectors, telemetry = self._embed([f"query: {text}"])
        return vectors[0], telemetry

    def _embed(self, texts: list[str]) -> tuple[list[list[float]], EmbeddingTelemetry]:
        started = time.perf_counter()
        model = self._load_model()
        vectors = [_normalize_vector(list(vector)) for vector in model.embed(texts)]
        for vector in vectors:
            if len(vector) != self.dim:
                raise LocalEmbedderUnavailable(
                    f"local embedder returned dim={len(vector)}, expected {self.dim}"
                )
        duration_ms = int((time.perf_counter() - started) * 1000)
        return vectors, EmbeddingTelemetry(
            model_id=self.model_id,
            dim=self.dim,
            count=len(vectors),
            duration_ms=duration_ms,
        )

    def _load_model(self):
        if self._model is not None:
            return self._model
        try:
            from fastembed import TextEmbedding
        except ImportError as err:
            raise LocalEmbedderUnavailable(
                "fastembed is not installed. Install it in the Telegram host venv "
                "to enable vector recall: python -m pip install fastembed"
            ) from err
        self._model = TextEmbedding(model_name=self.model_id)
        return self._model


def _normalize_vector(vector: list[float]) -> list[float]:
    if any(not math.isfinite(value) for value in vector):
        raise LocalEmbedderUnavailable("local embedder returned a non-finite vector")
    norm = math.sqrt(sum(value * value for value in vector))
    if norm == 0.0:
        raise LocalEmbedderUnavailable("local embedder returned a zero vector")
    return [float(value / norm) for value in vector]
