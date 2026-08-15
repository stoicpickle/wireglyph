# Changelog

All notable changes to Wireglyph will be recorded here.

## Unreleased

- Require Developer ID signing and Apple notarization for future macOS release
  archives, and Authenticode signing with an RFC 3161 timestamp for future
  Windows release archives.
- Isolate platform signing credentials behind a tag-restricted GitHub
  environment and remove temporary credential material on every exit path.

## 0.1.0-alpha.1

- Prepare the first sanitized public alpha repository.
- Add bounded Rust, JavaScript, TypeScript, and Python static module scanning.
- Add deterministic Overview, Focus, and Static Path terminal views.
- Add exact evidence inspection and portable versioned graph/path JSON.
- Add amber, green, cyan, red, and monochrome themes with reduced motion.
- Add Linux and macOS CI plus terminal-restoration tests through Windows ConPTY.
