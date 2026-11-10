---
name: dead-code-sweep
description: Find and remove dead Rust code after refactors using compiler warnings plus reference checks.
---

# Dead-Code Sweep

Run after any refactor, deprecation, or layer move that can orphan functions,
fields, or state. Every symbol earns its place; what loses its last caller
gets deleted in the same change.

## Procedure

1. Compiler first. `cargo build` must report zero warnings — `dead_code`
   already catches uncalled functions. Fix those before hunting further.
2. List candidates. `grep -n "fn "` across `src/` gives every function;
   struct fields and enum variants need review too.
3. Check references per candidate, preferred instrument first:
   - LSP: rust-analyzer `textDocument/references` on each symbol.
   - Fallback: ripgrep for the symbol name across `src/`.
4. Cross-check empties. Search tools can silently mishandle patterns
   (escaping, word boundaries). Any "no callers" result must be confirmed
   with a second instrument (`grep -rn`) before deleting.
5. Delete what the evidence supports:
   - Functions with no callers outside their own definition and tests.
   - Write-only struct fields (assigned, never read).
   - Unread struct fields. Note: serde ignores unknown JSON on read, so
     dropping a field keeps old caches parsing.
6. Spare by default: trait-impl members (external callers exist), test
   helpers, `#[deprecated]` shims that document a removal plan, and enum
   variants needed for exhaustive matches.
7. Verify: `cargo build` zero warnings, full test suite green.

## Reporting

State the count honestly — "sweep found N removals" or "none found" are
both valid results. Name each removed symbol and its last-caller evidence.
