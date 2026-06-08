//go:build !darwin && !linux

package obol

import "errors"

// Off-target placeholders. On any platform without a purego Dlopen, the binding compiles but
// every entry point fails fast (Version returns ""). darwin/linux use loader.go instead.
var (
	cVersion      func() uintptr
	cEstimatePath func(path *byte, dialect *byte, out *uintptr) int32
	cRefresh      func(asOf *byte, out *uintptr) int32
	cStringFree   func(p uintptr)
)

func ensureLoaded() error {
	return errors.New("obol: libobol_ffi is not available on this platform (only macOS and Linux are built)")
}

func cstr(p uintptr) string { return "" }
