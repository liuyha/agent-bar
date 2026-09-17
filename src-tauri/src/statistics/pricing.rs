//! Standard API-equivalent prices, USD per million tokens, verified 2026-09-16.
//! Sources and limitations: docs/token-pricing.md. These are not subscription charges.

use super::Usage;
use crate::models::ProviderId;

#[derive(Clone, Copy)]
struct Price {
    input: f64,
    cached: f64,
    write: Option<f64>,
    write_hour: Option<f64>,
    output: f64,
    long_context: LongContext,
}

#[derive(Clone, Copy)]
enum LongContext {
    None,
    Above272k,
    UnknownAbove200k,
}

pub(super) fn estimate(provider: ProviderId, model: Option<&str>, usage: Usage) -> Option<f64> {
    if usage.total() == 0 {
        return Some(0.0);
    }
    let mut price = lookup(provider, model?)?;
    // Impossible cache accounting should not produce a deceptively precise price.
    let ordinary = usage
        .input
        .checked_sub(usage.cached.checked_add(usage.cache_write)?)?;
    match price.long_context {
        LongContext::Above272k if usage.input > 272_000 => {
            price.input *= 2.0;
            price.cached *= 2.0;
            price.write = price.write.map(|value| value * 2.0);
            price.output *= 1.5;
        }
        LongContext::UnknownAbove200k if usage.input > 200_000 => return None,
        _ => {}
    }
    let write_cost = if usage.cache_write == 0 {
        if usage.write_5m != 0 || usage.write_1h != 0 {
            return None;
        }
        0.0
    } else if usage.write_5m != 0 || usage.write_1h != 0 {
        if usage.write_5m.checked_add(usage.write_1h)? != usage.cache_write {
            return None;
        }
        usage.write_5m as f64 * price.write? + usage.write_1h as f64 * price.write_hour?
    } else {
        // Claude's unsplit cache_creation_input_tokens use the default five-minute TTL.
        usage.cache_write as f64 * price.write?
    };
    Some(
        (ordinary as f64 * price.input
            + usage.cached as f64 * price.cached
            + write_cost
            + usage.output as f64 * price.output)
            / 1_000_000.0,
    )
}

fn lookup(provider: ProviderId, model: &str) -> Option<Price> {
    match provider {
        ProviderId::Codex => {
            let (input, cached, write, output, long_context) = match model {
                "gpt-6-astra" => (10.0, 1.0, Some(12.5), 50.0, LongContext::Above272k),
                "gpt-5.6-sol" => (4.0, 0.4, Some(5.0), 20.0, LongContext::Above272k),
                "gpt-5.6-terra" => (2.0, 0.2, Some(2.5), 12.0, LongContext::Above272k),
                "gpt-5.6-luna" => (0.2, 0.02, Some(0.25), 1.2, LongContext::Above272k),
                "gpt-5.5" => (5.0, 0.5, None, 30.0, LongContext::Above272k),
                "gpt-5.4" => (2.5, 0.25, None, 15.0, LongContext::Above272k),
                "gpt-5.3-codex" => (1.75, 0.175, None, 14.0, LongContext::None),
                _ => return None,
            };
            Some(Price {
                input,
                cached,
                write,
                write_hour: None,
                output,
                long_context,
            })
        }
        ProviderId::Claude => {
            let (input, cached, write, write_hour, output, long_context) = match model {
                "claude-opus-4-5" | "claude-opus-4-5-20251101" => {
                    (5.0, 0.5, 6.25, 10.0, 25.0, LongContext::UnknownAbove200k)
                }
                "claude-opus-4-6" | "claude-opus-4-7" | "claude-opus-4-8" | "claude-opus-5" => {
                    (5.0, 0.5, 6.25, 10.0, 25.0, LongContext::None)
                }
                "claude-sonnet-4"
                | "claude-sonnet-4-20250514"
                | "claude-sonnet-4-5"
                | "claude-sonnet-4-5-20250929" => {
                    (3.0, 0.3, 3.75, 6.0, 15.0, LongContext::UnknownAbove200k)
                }
                "claude-sonnet-4-6" => (3.0, 0.3, 3.75, 6.0, 15.0, LongContext::None),
                "claude-sonnet-5" => (2.0, 0.2, 2.5, 4.0, 10.0, LongContext::None),
                "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => {
                    (1.0, 0.1, 1.25, 2.0, 5.0, LongContext::UnknownAbove200k)
                }
                "claude-opus-4"
                | "claude-opus-4-20250514"
                | "claude-opus-4-1"
                | "claude-opus-4-1-20250805" => {
                    (15.0, 1.5, 18.75, 30.0, 75.0, LongContext::UnknownAbove200k)
                }
                "claude-3-5-haiku-20241022" => {
                    (0.8, 0.08, 1.0, 1.6, 4.0, LongContext::UnknownAbove200k)
                }
                "claude-fable-5-1" => (10.0, 0.25, 12.5, 20.0, 50.0, LongContext::None),
                "claude-fable-5" => (10.0, 1.0, 12.5, 20.0, 50.0, LongContext::None),
                _ => return None,
            };
            Some(Price {
                input,
                cached,
                write: Some(write),
                write_hour: Some(write_hour),
                output,
                long_context,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: Option<f64>, expected: f64) {
        assert!((actual.unwrap() - expected).abs() < 0.000_000_1);
    }

    #[test]
    fn codex_counts_cached_input_once_and_applies_request_context_threshold() {
        let usage = Usage {
            input: 100_000,
            cached: 40_000,
            cache_write: 10_000,
            output: 1_000,
            ..Usage::default()
        };
        close(
            estimate(ProviderId::Codex, Some("gpt-6-astra"), usage),
            0.715,
        );
        let threshold = Usage {
            input: 272_000,
            output: 1_000,
            ..Usage::default()
        };
        close(
            estimate(ProviderId::Codex, Some("gpt-6-astra"), threshold),
            2.77,
        );
        close(
            estimate(
                ProviderId::Codex,
                Some("gpt-6-astra"),
                Usage {
                    input: 272_001,
                    ..threshold
                },
            ),
            5.51502,
        );
    }

    #[test]
    fn claude_prices_cache_ttls_separately() {
        let usage = Usage {
            input: 1_000,
            cached: 300,
            cache_write: 300,
            write_5m: 100,
            write_1h: 200,
            output: 100,
        };
        close(
            estimate(ProviderId::Claude, Some("claude-sonnet-4-6"), usage),
            0.004365,
        );
        assert!(estimate(
            ProviderId::Claude,
            Some("claude-sonnet-4-6"),
            Usage {
                write_1h: 100,
                ..usage
            }
        )
        .is_none());
    }

    #[test]
    fn unknown_models_invalid_cache_and_unverified_legacy_cache_write_are_not_free() {
        let usage = Usage {
            input: 10,
            output: 5,
            ..Usage::default()
        };
        for model in [
            None,
            Some("custom-model"),
            Some("codex-auto-review"),
            Some("gpt-5.5-pro"),
        ] {
            assert!(estimate(ProviderId::Codex, model, usage).is_none());
        }
        assert!(estimate(
            ProviderId::Codex,
            Some("gpt-6-astra"),
            Usage {
                cached: 11,
                ..usage
            }
        )
        .is_none());
        assert!(estimate(
            ProviderId::Codex,
            Some("gpt-5.5"),
            Usage {
                cache_write: 1,
                ..usage
            }
        )
        .is_none());
        assert!(estimate(
            ProviderId::Claude,
            Some("claude-sonnet-4-5"),
            Usage {
                input: 200_001,
                ..usage
            }
        )
        .is_none());
        close(
            estimate(
                ProviderId::Claude,
                Some("claude-sonnet-4-6"),
                Usage {
                    input: 300_000,
                    output: 0,
                    ..usage
                },
            ),
            0.9,
        );
    }
}
