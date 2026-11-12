---
name: Bug report
about: Report an incorrect result or a failure to read a file.
title: ""
labels: bug
assignees: ""
---

## Revision and environment

Give the commit or package version, operating system and Rust or Python version.

## Reproduction

Give the exact command or minimal code, expected result and actual result.

## File and dataset

Give the file's provenance, dataset name and index. Attach a small synthetic
example, or a public file URL with its licence and SHA-256 checksum.
Do not attach private data.

## Dataset listing

Paste the output of `cargo run -p hdfour --example ls -- <file>`.
If the example is not available in your revision, state that instead.

## Reference result

Include the HDF4 C library version and result, if available.
Report security issues privately through the repository's Security tab.
