# Optimize

## Source checks

- Verify async state uses a strict union, cancels stale work, and defers expensive work.
- Check that loading states and fallbacks are coherent.

## Computed checks

- Use page-health, console, page, and network findings to judge robustness.
- Verify behavior under slow or failed network states.
- Report what was not exercised.