# Docs items deferred from the extension-core review
- docs/types/serie.md ~line 164 (sort_indices row) and ~339 (the ladder sentence): add the Record rung (a record key compares child by child; only value-ordered children build their leaf rows); the is_sorted row (~165) stays accurate.
- the spec's X7 list must include these two rows, not only "Windows by key".
- docs/types/serie.md "Size | 24 bytes" row is stale: 40 B (and a stated column leaf + 32 B after static values).
