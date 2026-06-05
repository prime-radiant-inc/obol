// Package obol is a thin purego binding over obol-core's C ABI. The Rust core owns all
// accounting; this package only marshals C strings and unmarshals JSON. No cgo: the native
// library is loaded at runtime via github.com/ebitengine/purego (CGO_ENABLED=0 works).
package obol

import (
	"encoding/json"
	"fmt"
	"runtime"
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
func drain(out uintptr) []byte {
	if out == 0 {
		return nil
	}
	defer cStringFree(out)
	return []byte(cstr(out))
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

func decodeEstimate(code int32, payload []byte) (*CostEstimate, error) {
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
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	p := append([]byte(path), 0)
	d := dialectBytes(dialect)
	var out uintptr
	code := cEstimatePath(&p[0], bytePtr(d), &out)
	runtime.KeepAlive(p)
	runtime.KeepAlive(d)
	return decodeEstimate(code, drain(out))
}

// EstimateBytes estimates in-memory transcript bytes. dialect "" means auto-detect.
func EstimateBytes(data []byte, dialect string) (*CostEstimate, error) {
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	dptr := data
	if len(dptr) == 0 {
		dptr = []byte{0} // non-nil pointer for len 0; length below stays 0
	}
	d := dialectBytes(dialect)
	var out uintptr
	code := cEstimateBytes(&dptr[0], uintptr(len(data)), bytePtr(d), &out)
	runtime.KeepAlive(dptr)
	runtime.KeepAlive(d)
	return decodeEstimate(code, drain(out))
}

// Refresh pulls fresh pricing tables. asOf is the caller's date string.
func Refresh(asOf string) (*RefreshReport, error) {
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	a := append([]byte(asOf), 0)
	var out uintptr
	code := cRefresh(&a[0], &out)
	runtime.KeepAlive(a)
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

// Version returns the obol core library version (static C string; not freed).
func Version() string {
	if err := ensureLoaded(); err != nil {
		return ""
	}
	return cstr(cVersion())
}
