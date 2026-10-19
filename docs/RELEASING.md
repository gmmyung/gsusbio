# Releasing gsusbio

The [publish workflow](../.github/workflows/publish.yml) publishes a stable crate version to crates.io when a GitHub Release is published. It uses the repository secret `CARGO_REGISTRY_TOKEN`; the token is passed only to the `cargo publish` step.

Before publishing, the workflow checks that the release tag is `v` plus the version in `Cargo.toml`, the tagged commit is reachable from `master`, and tests and packaging pass. Prereleases and drafts do not publish.

## Registry setup

Create a crates.io API token with permission to publish `gsusbio` and store it as the GitHub repository secret `CARGO_REGISTRY_TOKEN`. Never put the token in a commit, release description, or workflow log. After the first version exists, replace a broad bootstrap token with a crate scoped token for later releases.

## Publish a version

1. Update `version` in `Cargo.toml` and review the README and API changes. Merge the release commit into `master`.
2. Publish a GitHub Release tagged `vX.Y.Z` at that commit. The tag must match the manifest version exactly.
3. Watch the [Publish to crates.io workflow](https://github.com/gmmyung/gsusbio/actions/workflows/publish.yml). Check the new version on crates.io after the job finishes.

Crates.io versions cannot be overwritten. If a publish fails after the upload, check whether the version appeared on crates.io before rerunning the workflow.
