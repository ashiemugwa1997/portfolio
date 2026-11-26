# Changelog

## 0.2.2

- `Matcher`: an empty or blank pattern now matches everything with score 0 (`fuzzy_indices` gives no positions), as `fuzzy-matcher` does. Found while porting inquire: prompts showed no options before the user typed. `Index` is unchanged (an empty query returns nothing, which suits palettes).

## 0.2.1

- Better highlights and ranking for scattered matches: letters are aligned to the tightest stretch of text (forward pass to the first possible end, backward pass to the nearest letters), so "rlbk" marks "rollback" instead of the first r and l in the line. Scores use the same alignment, so what is highlighted is what was scored.

## 0.2.0

- `Index::positions` and `Index::search_with_positions`: matched character positions for highlighting, mapped back to the original text when lowercasing changes its length.
- `Matcher`: score one string at a time with `fuzzy_match` / `fuzzy_indices`, the same names and signatures as `fuzzy-matcher`. Caches the parsed pattern per thread and rejects most non-matches before lowercasing.
- `Scorer::positions` (default method, so existing scorers keep compiling).
- Benchmarks against fuzzy-matcher and nucleo-matcher; migration guide from fuzzy-matcher.
- Property tests against a reference model.

## 0.1.0

- First release: `Index` with character-fingerprint quick reject, heap top-k, pluggable `Scorer`.
