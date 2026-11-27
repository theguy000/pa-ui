# Minimal release version increments

Release tags use `vMAJOR.MINOR.PATCH` and step by the smallest increment (`+0.0.1` per release, such as `v0.1.0` to `v0.1.1`). Patch numbers run up to `.99` (`v0.1.99`) before rolling over to the next minor (`v0.2.0`), and minor numbers run up to `.99` before a new major version. Keeping version jumps minimal avoids inflating minor and major numbers during frequent iterative desktop releases while remaining compatible with SemVer and Windows installer metadata.
