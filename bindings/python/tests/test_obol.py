import json
import os
import re
import shutil
import tempfile
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
TESTDATA = REPO / "bindings" / "testdata"


@pytest.fixture()
def seeded(monkeypatch):
    d = Path(tempfile.mkdtemp(prefix="obol-py-"))
    shutil.copy(TESTDATA / "prices.json", d / "current.json")
    monkeypatch.setenv("OBOL_PRICING_DIR", str(d))
    yield d
    shutil.rmtree(d, ignore_errors=True)


def test_version():
    import obol
    # The binding reports the native lib's crate version; derive the
    # expectation from the workspace manifest so a release bump can't strand
    # a stale literal here (v0.7.0 broke CI exactly that way).
    manifest = (REPO / "Cargo.toml").read_text()
    expected = re.search(r'^version\s*=\s*"([^"]+)"', manifest, re.M).group(1)
    assert obol.version() == expected


def test_estimate_path_matches_expectations(seeded):
    import obol
    est = obol.estimate_path(TESTDATA / "obol-usage-mini.jsonl", dialect="obol")
    assert est.total_usd > 0.0
    assert est.pricing_as_of == "2026-06-05"
    assert isinstance(est.tokens.input, int)


def test_missing_tables_raises(monkeypatch):
    import obol
    monkeypatch.setenv("OBOL_PRICING_DIR", "/nonexistent/obol-py-xyz")
    with pytest.raises(obol.ObolError) as ei:
        obol.estimate_path(TESTDATA / "obol-usage-mini.jsonl", dialect="obol")
    assert ei.value.code == 1
    assert ei.value.kind == "PricingTablesMissing"


def test_refresh_rejects_garbage_as_of(seeded):
    import obol
    with pytest.raises(obol.ObolError) as ei:
        obol.refresh("Apr-2027")
    assert ei.value.code == 7
    assert ei.value.kind == "InvalidArgument"


def test_unknown_dialect_raises(seeded):
    import obol
    with pytest.raises(obol.ObolError) as ei:
        obol.estimate_path(TESTDATA / "claude-mini.jsonl", dialect="banana")
    assert ei.value.code == 7
