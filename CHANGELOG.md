# Changelog

All notable changes to Wireglyph will be recorded here.

## Unreleased

- Refuse folders with no supported source files instead of reporting an empty
  `SCOPED OK` graph.
- Show state-aware `Esc` back, flow-clear, and drawer-close hints in the live
  command footer.
- Recognize runtime entry points declared through the modern `package.json`
  `exports` field without treating TypeScript declaration or package subpath
  exports as application entries.
- Keep public release archives unsigned while retaining SHA-256 checksums and
  GitHub provenance attestations, with explicit platform-warning guidance.
- Add complete download, integrity verification, extraction, and installation
  instructions for release archives.
- Update the TOML parser dependency to 1.1.4.

## 0.1.0-alpha.1

- Prepare the first sanitized public alpha repository.
- Add bounded Rust, JavaScript, TypeScript, and Python static module scanning.
- Add deterministic Overview, Focus, and Static Path terminal views.
- Add exact evidence inspection and portable versioned graph/path JSON.
- Add amber, green, cyan, red, and monochrome themes with reduced motion.
- Add Linux and macOS CI plus terminal-restoration tests through Windows ConPTY.
