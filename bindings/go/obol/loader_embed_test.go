package obol

import (
	"os"
	"testing"

	"github.com/ebitengine/purego"
)

// Feed the dev dylib's bytes through the embedded extract+dlopen path and confirm a
// real symbol resolves — the same mechanics the published obol-go module relies on.
func TestExtractEmbeddedLoads(t *testing.T) {
	var libPath string
	for _, p := range devTargets() {
		if fileExists(p) {
			libPath = p
			break
		}
	}
	if libPath == "" {
		t.Skip("build the cdylib first: mise exec rust@1.96.0 -- cargo build -p obol-ffi")
	}
	raw, err := os.ReadFile(libPath)
	if err != nil {
		t.Fatal(err)
	}
	path, err := extractEmbedded(raw, libExt(), t.TempDir())
	if err != nil {
		t.Fatalf("extractEmbedded: %v", err)
	}
	if !fileExists(path) {
		t.Fatalf("extracted path missing: %s", path)
	}
	h, err := purego.Dlopen(path, purego.RTLD_NOW|purego.RTLD_GLOBAL)
	if err != nil {
		t.Fatalf("dlopen extracted: %v", err)
	}
	var version func() uintptr
	purego.RegisterLibFunc(&version, h, "obol_version")
	if got := cstr(version()); got == "" {
		t.Fatal("empty version from extracted lib")
	}
}

func TestExtractEmbeddedIdempotent(t *testing.T) {
	base := t.TempDir()
	b := []byte("not a real lib, just bytes for path logic")
	p1, err := extractEmbedded(b, "so", base)
	if err != nil {
		t.Fatal(err)
	}
	p2, err := extractEmbedded(b, "so", base)
	if err != nil {
		t.Fatal(err)
	}
	if p1 != p2 {
		t.Fatalf("paths differ: %s vs %s", p1, p2)
	}
}
