# Releasing gsusbio

The [publish workflow](../.github/workflows/publish.yml) publishes a stable crate version when a GitHub Release is published. It verifies that the release tag is `v` plus the version in `Cargo.toml`, that the tagged commit is on `master`, and that tests and packaging pass. The publish job obtains a short lived crates.io token through trusted publishing.

## One-time setup for version 0.1.0

Crates.io requires a crate to exist before a trusted publisher can be configured. The first version therefore needs a one-time manual publish by the crate owner:

1. Sign in to [crates.io](https://crates.io/) and verify the account's email address. Create an API token with permission to publish the new `gsusbio` crate.
2. From the reviewed `master` commit with a clean worktree, run `cargo fmt --all -- --check`, `cargo test --all-features --all-targets`, and `cargo publish --dry-run`. Then use the API token with `cargo publish --registry crates-io` to publish 0.1.0. Keep the token out of the repository and revoke it after setup.
3. In the new crate's crates.io **Settings → Trusted Publishing**, add a GitHub Actions publisher with owner `gmmyung`, repository `gsusbio`, workflow filename `publish.yml`, and no environment name. The workflow filename must match exactly.
4. Tag the published commit `v0.1.0` and push the tag. A tag push alone does not run the publish workflow, so it will not try to publish 0.1.0 again.

The [crates.io trusted publishing guide](https://crates.io/docs/trusted-publishing) explains the one-time registry configuration. The workflow uses the official [crates-io-auth-action](https://github.com/rust-lang/crates-io-auth-action) and does not require a GitHub repository secret.

## Later releases

1. Update `version` in `Cargo.toml`, review the README and API changes, and merge the release commit into `master`.
2. Publish a GitHub Release tagged `vX.Y.Z` at that commit. The tag must match the manifest version exactly and be reachable from `master`.
3. Watch the [Publish to crates.io workflow](https://github.com/gmmyung/gsusbio/actions/workflows/publish.yml). It verifies the package and publishes with a temporary token. Check the new version on crates.io after the job finishes.

Do not reuse a version number: crates.io versions cannot be overwritten.
