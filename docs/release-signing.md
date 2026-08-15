# Release signing

Wireglyph release tags fail closed unless the macOS and Windows binaries are
signed before packaging, checksum generation, provenance attestation, and
publication. Pull requests still build unsigned archives to validate the
packaging logic without exposing release credentials.

Signing credentials belong only in the `release-signing` GitHub environment.
Do not add them as repository secrets, local dotenv files, fixtures, or issue
attachments.

## macOS

Apple requires a paid Apple Developer Program membership and a **Developer ID
Application** certificate for command-line tools distributed outside the Mac
App Store. An Apple Development certificate is not sufficient.

Export the Developer ID identity and private key as a password-protected P12,
then configure these environment secrets:

- `APPLE_DEVELOPER_ID_P12_BASE64`: base64-encoded P12 bytes;
- `APPLE_DEVELOPER_ID_P12_PASSWORD`: the P12 password;
- `APPLE_NOTARY_APPLE_ID`: the Apple ID used for notarization;
- `APPLE_NOTARY_TEAM_ID`: the Developer Program team ID; and
- `APPLE_NOTARY_PASSWORD`: an app-specific Apple ID password.

The workflow imports the P12 into a temporary keychain, signs each Mach-O
binary with the hardened runtime and a secure timestamp, submits a temporary
ZIP container to `notarytool`, rejects warnings and errors, and then repacks the
same signed binary into the public tarball. The temporary certificate and
keychain are removed even when a later step fails.

## Windows

Windows requires a trusted Authenticode code-signing identity. The no-cost
first choice for this open-source project is to apply to
[SignPath Foundation](https://signpath.org/). It keeps the private key in an HSM
and provides qualifying open-source projects with managed signing. Its approval
and project configuration happen outside this repository.

The workflow also supports an existing exportable PFX. Configure:

- `WINDOWS_SIGNING_PFX_BASE64`: base64-encoded PFX bytes containing the
  certificate and private key; and
- `WINDOWS_SIGNING_PFX_PASSWORD`: the PFX password.

The workflow signs `wireglyph.exe` with SHA-256, obtains an RFC 3161 timestamp,
and runs SignTool policy verification before recreating the release ZIP.

Public certificate authorities generally require newly issued OV and EV
private keys to be protected by a hardware token or HSM. If the selected
provider does not permit an exportable PFX, this workflow must be adapted to
that provider's cloud or hardware-backed signing interface instead of copying a
private key into GitHub.

## GitHub environment

The `release-signing` environment accepts only tags matching `v*` and requires
maintainer approval before either signing job starts. Its secrets are
unavailable to pull requests and ordinary branch builds. A release tag must
match the version in `Cargo.toml`; after platform verification, the public
workflow generates `SHA256SUMS`, creates GitHub/Sigstore provenance, and
publishes the release.

Before cutting a tag, verify that all seven secret names above are present:

```sh
gh secret list --env release-signing --repo stoicpickle/wireglyph
```

Never print, download, or test the secret values from an untrusted branch.
