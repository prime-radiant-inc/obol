// Package obol is a thin cgo binding over obol-core's C ABI. The Rust core owns all
// accounting; this package only marshals C strings and unmarshals JSON.
package obol

/*
#cgo CFLAGS: -I${SRCDIR}/../../../crates/obol-ffi/include
#cgo LDFLAGS: -L${SRCDIR}/../../../target/debug -lobol_ffi -Wl,-rpath,${SRCDIR}/../../../target/debug
#include <stdlib.h>
#include "obol.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

type TokenBuckets struct {
	Input      uint64 `json:"input"`
	Output     uint64 `json:"output"`
	CacheRead  uint64 `json:"cache_read"`
	CacheWrite uint64 `json:"cache_write"`
}

type ModelCost struct {
	Model       string       `json:"model"`
	Provider    string       `json:"provider"`
	Tokens      TokenBuckets `json:"tokens"`
	SubtotalUSD float64      `json:"subtotal_usd"`
}

type Approximation struct {
	Kind   string `json:"kind"`
	Detail string `json:"detail,omitempty"`
}

type CostEstimate struct {
	TotalUSD       float64         `json:"total_usd"`
	PerModel       []ModelCost     `json:"per_model"`
	Tokens         TokenBuckets    `json:"tokens"`
	UnpricedModels []string        `json:"unpriced_models"`
	Approximations []Approximation `json:"approximations"`
	PricingAsOf    string          `json:"pricing_as_of"`
}

type RefreshReport struct {
	Models    uint64 `json:"models"`
	AsOf      string `json:"as_of"`
	WrittenTo string `json:"written_to"`
}

// ObolError carries the FFI error envelope.
type ObolError struct {
	Code    int    `json:"code"`
	Kind    string `json:"kind"`
	Message string `json:"message"`
}

func (e *ObolError) Error() string {
	return fmt.Sprintf("obol: %s (code %d): %s", e.Kind, e.Code, e.Message)
}

// drain copies the obol-owned C string into a Go []byte and frees it. Always frees.
func drain(out *C.char) []byte {
	if out == nil {
		return nil
	}
	defer C.obol_string_free(out)
	return []byte(C.GoString(out))
}

func toError(code int, payload []byte) error {
	e := &ObolError{Code: code, Kind: "Unknown", Message: "no detail"}
	if len(payload) > 0 {
		var env struct {
			Error ObolError `json:"error"`
		}
		if json.Unmarshal(payload, &env) == nil && env.Error.Code != 0 {
			*e = env.Error
		}
	}
	return e
}

func decodeEstimate(code C.int32_t, payload []byte) (*CostEstimate, error) {
	if int(code) != 0 {
		return nil, toError(int(code), payload)
	}
	var est CostEstimate
	if err := json.Unmarshal(payload, &est); err != nil {
		return nil, err
	}
	return &est, nil
}

// EstimatePath estimates a transcript file's cost. dialect "" means auto-detect.
func EstimatePath(path, dialect string) (*CostEstimate, error) {
	cPath := C.CString(path)
	defer C.free(unsafe.Pointer(cPath))
	cDialect := dialectArg(dialect)
	defer freeDialect(cDialect)
	var out *C.char
	code := C.obol_estimate_path(cPath, cDialect, &out)
	return decodeEstimate(code, drain(out))
}

// EstimateBytes estimates in-memory transcript bytes. dialect "" means auto-detect.
func EstimateBytes(data []byte, dialect string) (*CostEstimate, error) {
	var dptr *C.uint8_t
	if len(data) > 0 {
		dptr = (*C.uint8_t)(unsafe.Pointer(&data[0]))
	} else {
		dptr = (*C.uint8_t)(unsafe.Pointer(&[]byte{0}[0])) // non-nil for len 0
	}
	cDialect := dialectArg(dialect)
	defer freeDialect(cDialect)
	var out *C.char
	code := C.obol_estimate_bytes(dptr, C.uintptr_t(len(data)), cDialect, &out)
	return decodeEstimate(code, drain(out))
}

// Refresh pulls fresh pricing tables. asOf is the caller's date string.
func Refresh(asOf string) (*RefreshReport, error) {
	cAsOf := C.CString(asOf)
	defer C.free(unsafe.Pointer(cAsOf))
	var out *C.char
	code := C.obol_refresh_pricing(cAsOf, &out)
	payload := drain(out)
	if int(code) != 0 {
		return nil, toError(int(code), payload)
	}
	var r RefreshReport
	if err := json.Unmarshal(payload, &r); err != nil {
		return nil, err
	}
	return &r, nil
}

// Version returns the obol library version (static C string; not freed).
func Version() string {
	return C.GoString(C.obol_version())
}

func dialectArg(dialect string) *C.char {
	if dialect == "" {
		return nil
	}
	return C.CString(dialect)
}

func freeDialect(p *C.char) {
	if p != nil {
		C.free(unsafe.Pointer(p))
	}
}
