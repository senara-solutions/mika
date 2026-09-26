# Correction: existing review's markdown link false-negative claim is wrong

**Priority:** P3
**File:** `todos/review-308-fabricated-action-guard.md`
**Issue:** #308

## Problem

The existing review (`review-308-fabricated-action-guard.md`, section 3) claims that
markdown link syntax `[text](url#fragment)` causes a false negative because the `)`
character in the `[^\s)>\]]+` character class terminates URL capture before the fragment
identifier. This claim is incorrect.

Verified with Rust's `regex` crate v1: the regex correctly matches URLs inside markdown
links. For example:

```
Input: "I posted [a comment](https://github.com/org/repo/pull/1#issuecomment-99)"
Match: "https://github.com/org/repo/pull/1#issuecomment-99"
```

The `[^\s)>\]]+` quantifier is greedy and the regex engine tries the longest possible
match. Since the `#issuecomment-99` characters are all valid (not in the exclusion set),
the quantifier consumes them. The `)` that terminates the markdown link is at the very
end, but the alternation `(?:#issuecomment-\d+|...)` matches within the already-consumed
characters, and the overall regex succeeds.

More precisely: the regex engine finds that `[^\s)>\]]+` can match
`org/repo/pull/1#issuecomment-99` (stopping at the `)`), and then the alternation
`#issuecomment-\d+` matches within that span. Rust's `regex` crate handles this
correctly despite using a non-backtracking engine -- it compiles to an NFA that
explores all valid match positions.

## Recommendation

Update the existing review file to correct this claim. The "P2 regex gap" and "P2
missing markdown link test" findings should be reclassified as non-issues. Adding a
markdown link test is still valuable for documenting that the regex handles this case,
but it is not a gap.
