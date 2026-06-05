"""obol — agent-transcript cost estimation. Thin binding over the Rust core via the C ABI."""
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

from . import _lib

__all__ = [
    "estimate_path", "estimate_bytes", "refresh", "version",
    "CostEstimate", "ModelCost", "TokenBuckets", "Approximation", "RefreshReport", "ObolError",
]


class ObolError(Exception):
    def __init__(self, code: int, kind: str, message: str):
        super().__init__(f"obol: {kind} (code {code}): {message}")
        self.code, self.kind, self.message = code, kind, message


@dataclass
class TokenBuckets:
    input: int
    output: int
    cache_read: int
    cache_write: int

    @classmethod
    def from_json(cls, d: dict) -> "TokenBuckets":
        return cls(d["input"], d["output"], d["cache_read"], d["cache_write"])


@dataclass
class ModelCost:
    model: str
    provider: str
    tokens: TokenBuckets
    subtotal_usd: float

    @classmethod
    def from_json(cls, d: dict) -> "ModelCost":
        return cls(d["model"], d["provider"], TokenBuckets.from_json(d["tokens"]), d["subtotal_usd"])


@dataclass
class Approximation:
    kind: str
    detail: Optional[str] = None

    @classmethod
    def from_json(cls, d: dict) -> "Approximation":
        return cls(d["kind"], d.get("detail"))


@dataclass
class CostEstimate:
    total_usd: float
    per_model: list[ModelCost]
    tokens: TokenBuckets
    unpriced_models: list[str]
    approximations: list[Approximation]
    pricing_as_of: str

    @classmethod
    def from_json(cls, d: dict) -> "CostEstimate":
        return cls(
            d["total_usd"],
            [ModelCost.from_json(m) for m in d["per_model"]],
            TokenBuckets.from_json(d["tokens"]),
            list(d["unpriced_models"]),
            [Approximation.from_json(a) for a in d["approximations"]],
            d["pricing_as_of"],
        )


@dataclass
class RefreshReport:
    models: int
    as_of: str
    written_to: str

    @classmethod
    def from_json(cls, d: dict) -> "RefreshReport":
        return cls(d["models"], d["as_of"], d["written_to"])


def _raise(code: int, payload: Optional[bytes]):
    kind, message = "Unknown", "no detail"
    if payload:
        try:
            err = json.loads(payload)["error"]
            kind, message = err.get("kind", kind), err.get("message", message)
            code = err.get("code", code)
        except (ValueError, KeyError):
            pass
    raise ObolError(code, kind, message)


def _estimate_result(code: int, payload: Optional[bytes]) -> CostEstimate:
    if code != 0:
        _raise(code, payload)
    return CostEstimate.from_json(json.loads(payload))


def estimate_path(path, dialect: Optional[str] = None) -> CostEstimate:
    d = dialect.encode() if dialect else None
    code, payload = _lib.call(_lib._lib.obol_estimate_path, str(path).encode(), d)
    return _estimate_result(code, payload)


def estimate_bytes(data: bytes, dialect: Optional[str] = None) -> CostEstimate:
    d = dialect.encode() if dialect else None
    code, payload = _lib.call(_lib._lib.obol_estimate_bytes, data, len(data), d)
    return _estimate_result(code, payload)


def refresh(as_of: str) -> RefreshReport:
    code, payload = _lib.call(_lib._lib.obol_refresh_pricing, as_of.encode())
    if code != 0:
        _raise(code, payload)
    return RefreshReport.from_json(json.loads(payload))


def version() -> str:
    return _lib._lib.obol_version().decode()
