# third_party/opus (owned by WP-06, DESIGN §2.3)

Only `SHA256SUMS` and this file are committed. Everything else here is written by
`cargo xtask fetch-opus` and is gitignored:

| Path | What |
|---|---|
| `opus-1.6.1.tar.gz` | release tarball, `https://downloads.xiph.org/releases/opus/opus-1.6.1.tar.gz` |
| `opus_data-<sha256>.tar.gz` | DNN weights, downloaded only when `dnn/*_data.c` are missing from the release |
| `opus-1.6.1/` | extracted sources compiled by `crates/opus-sys/build.rs` |
| `.ok` | stamp with both SHA-256 values; `build.rs` refuses to build without it |

## Where the pins come from (pinned 2026-10-22)

- `opus-1.6.1.tar.gz`: the line for that file in xiph's published
  `https://downloads.xiph.org/releases/opus/SHA256SUMS.txt`.
- `opus_data-….tar.gz`: the hash passed to `dnn/download_model.sh` by `autogen.sh` at
  the upstream `v1.6.1` tag (`https://github.com/xiph/opus/blob/v1.6.1/autogen.sh`). The
  model tarball is `https://media.xiph.org/opus/models/opus_data-<hash>.tar.gz` and its
  file name carries the hash. The release tarball already ships the generated
  `dnn/*_data.c` files, so this download is a fallback. `tools/xtask/src/opus.rs`
  hard-codes the same hash and fails if this file disagrees with it.

`SHA256SUMS` uses the standard two-column format, so
`(cd third_party/opus && shasum -a 256 -c SHA256SUMS)` checks the cached tarballs by hand.

## Updating libopus

1. Take the new release line from xiph's `SHA256SUMS.txt` and the model hash from the
   new tag's `autogen.sh`.
2. Update `SHA256SUMS`, plus `OPUS_VERSION` and `MODEL_SHA256` in
   `tools/xtask/src/opus.rs` and `OPUS_DIR` in `crates/opus-sys/build.rs`.
3. Delete `opus-*/` and `.ok`, then run `cargo xtask fetch-opus && cargo test -p opus-sys --release && cargo xtask opus-smoke`.
