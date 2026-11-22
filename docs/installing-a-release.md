# How to install a release

This guide shows how to install a prebuilt `priory` binary instead of building it from source. It
assumes Linux on x86_64 or aarch64, or macOS on Apple silicon.

`install.sh` installs the binary of the latest
[release](https://github.com/maxence-lefebvre/priory/releases) to `~/.local/bin`. When that page
lists no release, the script fails with a 404: build from source, as
[Getting started](../GETTING_STARTED.md#1-build-priory) shows.

```sh
curl -fsSL https://raw.githubusercontent.com/maxence-lefebvre/priory/main/install.sh | sh
```

## What the script checks

The script takes the binary for Linux x86_64, Linux aarch64 or macOS on Apple silicon, and checks
it against its `.sha256`. The `.sha256` sits beside the binary in the same release, so it only
catches a broken download. When [gh](https://cli.github.com/) is on your `PATH` and signed in, the
script also runs `gh attestation verify`, which proves this repository's release workflow built the
binary, and refuses a binary it cannot verify. Without gh, it prints a note and skips that check.
Run `sh install.sh --prefix /usr/local` from a clone to install to `/usr/local/bin`. Set
`PRIORY_RELEASE_URL` to a release's download address to pick a release other than the latest. With
[cargo-binstall](https://github.com/cargo-bins/cargo-binstall),
`cargo binstall --git https://github.com/maxence-lefebvre/priory priory` downloads the same binary.

## What a release holds

Each release also holds `priory-herdr.tar.gz`, the [Herdr adapter](../adapters/herdr/), and
`priory-github-extension.zip`, the [GitHub extension](adapters/github.md).

The release binaries leave out the cargo features. A build from source can add them: the
in-process embedding model for search with `--features model`, and the Postgres event log, which
lets several machines share one log, with `--features postgres` (see
[`[log]`](reference/factory.md#log)).
