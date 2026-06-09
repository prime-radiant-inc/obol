//! Provider-keyed usage normalizers. Each turns a provider's raw `usage`
//! object into token buckets. The agent dialects and the `obol` house dialect
//! share these so the per-provider accounting — the cache-bucket split, the
//! OpenAI cached-subtraction, the part naive summers get wrong — lives in one
//! place, not once per dialect.

pub mod anthropic;
pub mod openai;

/// Token buckets extracted from a provider `usage` object. Model, provider,
/// namespace and service_tier are the caller's to attach; `request_input_tokens`
/// is derived from these by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderTokens {
    pub input_uncached: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub output: u64,
}
