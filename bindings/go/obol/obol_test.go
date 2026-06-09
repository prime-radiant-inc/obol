package obol

import (
	"os"
	"path/filepath"
	"testing"
)

func repoRoot(t *testing.T) string {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	return filepath.Join(wd, "..", "..", "..") // bindings/go/obol -> repo root
}

func testdata(t *testing.T) string { return filepath.Join(repoRoot(t), "bindings", "testdata") }

func seed(t *testing.T) {
	dir := t.TempDir()
	src, err := os.ReadFile(filepath.Join(testdata(t), "prices.json"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "current.json"), src, 0o644); err != nil {
		t.Fatal(err)
	}
	setPricingDir(t, dir)
}

func TestVersion(t *testing.T) {
	if Version() == "" {
		t.Fatal("empty version")
	}
}

func TestEstimatePath(t *testing.T) {
	seed(t)
	est, err := EstimatePath(filepath.Join(testdata(t), "claude-mini.jsonl"), "claude")
	if err != nil {
		t.Fatal(err)
	}
	if est.TotalUSD <= 0 {
		t.Fatalf("expected positive total, got %v", est.TotalUSD)
	}
	if est.PricingAsOf != "2026-06-05" {
		t.Fatalf("unexpected pricing_as_of %q", est.PricingAsOf)
	}
}

func TestMissingTablesIsError(t *testing.T) {
	setPricingDir(t, "/nonexistent/obol-go-xyz")
	_, err := EstimatePath(filepath.Join(testdata(t), "claude-mini.jsonl"), "claude")
	oe, ok := err.(*ObolError)
	if !ok || oe.Code != 1 {
		t.Fatalf("expected ObolError code 1, got %v", err)
	}
}

func TestRefreshRejectsGarbageAsOf(t *testing.T) {
	seed(t)
	_, err := Refresh("Apr-2027")
	oe, ok := err.(*ObolError)
	if !ok || oe.Code != 7 || oe.Kind != "InvalidArgument" {
		t.Fatalf("expected ObolError code 7 InvalidArgument, got %v", err)
	}
}

func TestUnknownDialectIsError(t *testing.T) {
	seed(t)
	_, err := EstimatePath(filepath.Join(testdata(t), "claude-mini.jsonl"), "banana")
	oe, ok := err.(*ObolError)
	if !ok || oe.Code != 7 {
		t.Fatalf("expected ObolError code 7, got %v", err)
	}
}
