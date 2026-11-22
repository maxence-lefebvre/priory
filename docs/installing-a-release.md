# How to install Priory

This guide shows how to install `priory` without a clone: the release binary with `install.sh`,
or a build with Nix. It assumes Linux on x86_64 or aarch64, or macOS on Apple silicon. To build
from a clone instead, see [Getting started](../GETTING_STARTED.md#1-build-priory).

## With the install script

`install.sh` installs the binary of the latest
[release](https://github.com/maxence-lefebvre/priory/releases) to `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/maxence-lefebvre/priory/main/install.sh | sh
priory --version
```

### What the script checks

The script takes the binary for Linux x86_64, Linux aarch64 or macOS on Apple silicon, and checks
it against its `.sha256`. The `.sha256` sits beside the binary in the same release, so it only
catches a broken download. When [gh](https://cli.github.com/) is on your `PATH` and signed in, the
script also runs `gh attestation verify`, which proves this repository's release workflow built the
binary, and refuses a binary it cannot verify. Without gh, it prints a note and skips that check.
Run `sh install.sh --prefix /usr/local` from a clone to install to `/usr/local/bin`. Set
`PRIORY_RELEASE_URL` to a release's download address to pick a release other than the latest. With
[cargo-binstall](https://github.com/cargo-bins/cargo-binstall),
`cargo binstall --git https://github.com/maxence-lefebvre/priory priory` downloads the same binary.

### What a release holds

Each release also holds `priory-herdr.tar.gz`, the [Herdr adapter](../adapters/herdr/), and
`priory-github-extension.zip`, the [GitHub extension](adapters/github.md).

The release binaries leave out the cargo features. A build from source can add them: the
in-process embedding model for search with `--features model`, and the Postgres event log, which
lets several machines share one log, with `--features postgres` (see
[`[log]`](reference/factory.md#log)).

## With Nix

The repository is a Nix flake. Its default package builds `priory` from source with nixpkgs' Rust
toolchain, so the first install compiles for several minutes. The flake also holds a development
shell, `nix develop`, with the tools the tests need.

Run `priory` once without installing it:

```sh
nix run github:maxence-lefebvre/priory -- --version
```

Install it to your Nix profile:

```sh
nix profile install github:maxence-lefebvre/priory
```

To pin a version, add a commit or a tag to the reference, such as
`github:maxence-lefebvre/priory/<commit>`. Releases after v0.1.0 hold the flake; v0.1.0 does not.

To add Priory to your own flake, such as a NixOS or home-manager configuration, take it as an
input and use its package or its overlay:

```nix
{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.priory.url = "github:maxence-lefebvre/priory";

  outputs = { nixpkgs, priory, ... }: {
    nixosConfigurations.my-machine = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ({ pkgs, ... }: {
          nixpkgs.overlays = [ priory.overlays.default ];
          environment.systemPackages = [ pkgs.priory ];
        })
      ];
    };
  };
}
```

The flake builds the default cargo features only, like the release binaries. To run Priory as
services, see [How to run Priory as services](running-as-services.md) and point `ExecStart` at the
`priory` of your profile.
