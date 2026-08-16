# Release integrity

Wireglyph publishes unsigned alpha binaries for Linux x86_64, macOS Apple
Silicon, macOS Intel, and Windows x86_64. Platform signing is deliberately not a
release requirement for this small open-source project.

The public release workflow:

1. builds locked release binaries with Rust 1.88 on the matching GitHub-hosted
   runner;
2. packages the binary, README, changelog, and both licenses;
3. generates `SHA256SUMS` after every platform build succeeds;
4. creates a GitHub artifact attestation from those checksums; and
5. publishes the archives, checksums, and attestation from a version-matching
   `v*` tag.

Pull requests run the same platform build and packaging matrix without
publishing a release. Actions are pinned to full commit SHAs, checkout does not
persist credentials, and only the final tag-only job receives `contents: write`
and attestation permissions.

## What users should expect

Because the binaries are not Apple-notarized or Windows Authenticode-signed,
macOS Gatekeeper and Windows SmartScreen may display an unidentified-developer
warning. That warning is expected; checksums and provenance establish which
public workflow produced the bytes, but they do not replace operating-system
publisher signing.

Before extracting an archive:

1. compare it with the matching line in `SHA256SUMS`; and
2. run `gh attestation verify <archive> --repo stoicpickle/wireglyph`.

Users who do not want to override an operating-system warning can build from
source with the locked dependency graph:

```sh
cargo install --git https://github.com/stoicpickle/wireglyph --locked
```

Platform signing can be added later if maintainers intentionally acquire and
configure the necessary identities. Its absence does not block unsigned,
checksummed, provenance-attested alpha releases.
