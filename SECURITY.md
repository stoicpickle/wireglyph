# Security policy

Wireglyph scans unfamiliar source trees, so containment and honest failure are
part of its product contract.

## Supported versions

Only the latest tagged alpha and the current `main` branch receive security
fixes during the pre-1.0 period.

## Report a vulnerability privately

Use GitHub's **Report a vulnerability** form in this repository's Security tab.
Please do not open a public issue for symlink escapes, unintended file reads,
terminal-restoration failures, command execution, source disclosure, or secret
exposure.

Include the Wireglyph version or commit, operating system, terminal, a minimal
reproduction, and the boundary you expected Wireglyph to preserve. Do not attach
real credentials or proprietary source code.

The maintainer will acknowledge a report as soon as practical, reproduce it,
and coordinate disclosure after a fix is available. No response-time SLA is
promised during the experimental alpha.

## Current trust boundary

- Wireglyph reads supported source and manifest files beneath the selected root.
- It does not execute the inspected project.
- It does not upload source code or require a network connection at runtime.
- It does not write into the inspected project.
- Static relationships are not runtime traces.

See the README for the bounded scan limits and unsupported analysis cases.
