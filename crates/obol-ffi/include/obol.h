#ifndef OBOL_H
#define OBOL_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/*
 Library version as a `'static` NUL-terminated string. Do NOT free.
 */
const char *obol_version(void);

/*
 Free a string previously returned in an `out_json` out-parameter. NULL is a no-op.
 */
void obol_string_free(char *s);

/*
 Estimate cost from transcript bytes (borrowed). See the ownership contract.
 */
int32_t obol_estimate_bytes(const uint8_t *data,
                            uintptr_t len,
                            const char *dialect,
                            char **out_json);

/*
 Estimate cost from a transcript file path (borrowed). See the ownership contract.
 */
int32_t obol_estimate_path(const char *path, const char *dialect, char **out_json);

/*
 Refresh pricing tables (network). `as_of` is the caller's date string. See the contract.
 */
int32_t obol_refresh_pricing(const char *as_of, char **out_json);

#endif  /* OBOL_H */
