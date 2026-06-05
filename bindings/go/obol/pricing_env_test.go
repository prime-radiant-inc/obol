package obol

import (
	"runtime"
	"sync"
	"testing"

	"github.com/ebitengine/purego"
)

var (
	libcOnce sync.Once
	cSetenv  func(name *byte, val *byte, overwrite int32) int32
)

func libcSetenv() func(name *byte, val *byte, overwrite int32) int32 {
	libcOnce.Do(func() {
		name := "libc.so.6"
		if runtime.GOOS == "darwin" {
			name = "libSystem.B.dylib"
		}
		h, err := purego.Dlopen(name, purego.RTLD_NOW|purego.RTLD_GLOBAL)
		if err != nil {
			return // cSetenv stays nil; setPricingDir falls back to os.Setenv only
		}
		purego.RegisterLibFunc(&cSetenv, h, "setenv")
	})
	return cSetenv
}

// setPricingDir sets OBOL_PRICING_DIR so the dlopen'd core's getenv sees it on every
// platform: os.Setenv (Go-side reads + macOS) AND libc setenv (Linux, CGO_ENABLED=0).
// Restores via t.Cleanup.
func setPricingDir(t *testing.T, dir string) {
	t.Helper()
	t.Setenv("OBOL_PRICING_DIR", dir) // Go-side + macOS; auto-restored by t.Setenv
	if fn := libcSetenv(); fn != nil {
		name := append([]byte("OBOL_PRICING_DIR"), 0)
		val := append([]byte(dir), 0)
		fn(&name[0], &val[0], 1)
		runtime.KeepAlive(name)
		runtime.KeepAlive(val)
		t.Cleanup(func() {
			empty := append([]byte("OBOL_PRICING_DIR"), 0)
			z := []byte{0}
			fn(&empty[0], &z[0], 1) // best-effort clear for native readers
			runtime.KeepAlive(empty)
		})
	}
}
