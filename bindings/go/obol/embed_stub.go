package obol

// Dev build: no embedded native library. The published obol-go module REPLACES this
// file with generated embed_<goos>_<goarch>.go files (and embed_unsupported.go). When
// embeddedLib is empty, the loader falls back to OBOL_LIB / the repo target/ dir.
var embeddedLib []byte

const embeddedExt = ""
