use assert_cmd::Command;
use std::fs;

#[test]
fn estimate_json_on_claude_fixture() {
    let tmp = tempfile::tempdir().unwrap();
    let sample = include_str!("../../obol-core/tests/fixtures/litellm-sample.json");
    let claude = include_str!("../../obol-core/tests/fixtures/claude-mini.jsonl");
    // Seed a price snapshot without network: normalize the sample sheet via obol-core.
    let store_json =
        obol_core::pricing::refresh::normalize_litellm(sample.as_bytes(), "2026-06-04").unwrap();
    let dir = tmp.path().join("obol");
    fs::create_dir_all(&dir).unwrap();
    store_json.save(&dir.join("current.json")).unwrap();

    let transcript = tmp.path().join("session.jsonl");
    fs::write(&transcript, claude).unwrap();

    Command::cargo_bin("obol")
        .unwrap()
        .env("OBOL_PRICING_DIR", &dir)
        .args([
            "estimate",
            transcript.to_str().unwrap(),
            "--dialect",
            "claude",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"total_usd\""));
}

#[test]
fn estimate_reports_bundled_pricing_source() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = include_str!("../../obol-core/tests/fixtures/claude-mini.jsonl");
    let transcript = tmp.path().join("session.jsonl");
    fs::write(&transcript, claude).unwrap();

    // No override + an empty XDG home -> no on-disk snapshot, so the embedded one
    // prices it and the source must be "bundled". Pointing XDG at the (snapshot-free)
    // temp dir keeps this hermetic regardless of the dev's real ~/.local/share/obol.
    Command::cargo_bin("obol")
        .unwrap()
        .env_remove("OBOL_PRICING_DIR")
        .env("XDG_DATA_HOME", tmp.path())
        .args([
            "estimate",
            transcript.to_str().unwrap(),
            "--dialect",
            "claude",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("pricing_source"))
        .stdout(predicates::str::contains("bundled"));
}

#[test]
fn estimate_gemini_dialect_string() {
    let tmp = tempfile::tempdir().unwrap();
    let gemini = include_str!("../../obol-core/tests/fixtures/gemini-mini.jsonl");
    let transcript = tmp.path().join("session.jsonl");
    fs::write(&transcript, gemini).unwrap();

    Command::cargo_bin("obol")
        .unwrap()
        .env_remove("OBOL_PRICING_DIR")
        .args([
            "estimate",
            transcript.to_str().unwrap(),
            "--dialect",
            "gemini",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("gemini-3-flash-preview"));
}

#[test]
fn estimate_obol_dialect_string() {
    let tmp = tempfile::tempdir().unwrap();
    let obol = include_str!("../../obol-core/tests/fixtures/obol-usage-mini.jsonl");
    let transcript = tmp.path().join("usage.jsonl");
    fs::write(&transcript, obol).unwrap();

    Command::cargo_bin("obol")
        .unwrap()
        .env_remove("OBOL_PRICING_DIR")
        .args([
            "estimate",
            transcript.to_str().unwrap(),
            "--dialect",
            "obol",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("claude-opus-4-8"));
}
