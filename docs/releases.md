# Sniplet releases

The three release workflows have manual triggers only. They do not run on a
schedule, a push, or a new tag. The existing CI workflow still checks pushes
and pull requests.

The workflow files must be on `main` before the Run workflow buttons appear.
See [GitHub's manual workflow instructions](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow).
Keep the workflow branch set to `main`. Write access to the repository is
required. The workflows use GitHub's supplied token; no extra release token
is required.

## Set the next version

Open **Actions → Bump Version → Run workflow**.

| Input | Effect |
| --- | --- |
| `patch` | `0.1.0` becomes `0.1.1` |
| `minor` | `0.1.1` becomes `0.2.0` |
| `major` | `0.2.0` becomes `1.0.0` |
| `set` | Uses the exact version input, such as `0.3.0` |

The new version must be greater than the current version. Use three numeric
parts without a `v` prefix or build suffix. The exact version field is used
only when `set` is selected.

The version is stored in `[workspace.package]` in `Cargo.toml`. All three crates
inherit it. The workflow updates the workspace package entries in `Cargo.lock`
and commits both files to `main`. It does not start a nightly build. If another
commit reaches `main` during the run, the push stops; run the workflow again.
Branch rules that prevent direct pushes will also stop this step.

For a local change, use Python 3.11 or later:

```sh
python3 scripts/release.py bump patch
python3 scripts/release.py bump set --version 0.3.0
```

Commit the changed `Cargo.toml` and `Cargo.lock` before building a release.

## Build a nightly

Open **Actions → Build Nightly → Run workflow**. Enter `main`, another branch,
a tag, or a full commit SHA in the `ref` field. The source must contain the
release scripts. The workflow resolves that source to one full commit before
it starts the builds.

All nightly tags include the first seven characters of the source SHA. For
example, version `0.2.0` at commit `a1b2c3d…` produces `v0.2.0-a1b2c3d`.
The app's internal version remains `0.2.0`, so its packages can be promoted
without a rebuild or a change to signed files.

Four jobs run in parallel. Each job checks the code, runs the workspace tests,
builds an optimized application, and makes its package. Cargo caches are
separate for each target. Format checking runs once on Linux. Completed archives
are uploaded without a second compression pass.

| Download | Contents |
| --- | --- |
| `Sniplet-0.2.0-a1b2c3d-macos-arm64.zip` | Mac Apple Silicon `.app` |
| `Sniplet-0.2.0-a1b2c3d-macos-x64.zip` | Mac Intel `.app` |
| `Sniplet-0.2.0-a1b2c3d-windows-x64.zip` | Portable Windows executable and support files |
| `Sniplet-0.2.0-a1b2c3d-linux-x64.tar.gz` | Linux executable, desktop entry, icons, and support files |
| `SHA256SUMS` | SHA-256 checksums for all four packages |
| `release.json` | Version, full source SHA, target names, package sizes, and checksums |

The publisher waits for all four jobs to pass. It uploads and verifies all files
in a draft, then publishes the nightly as a GitHub prerelease. Nightlies do not
replace the Latest stable release. Packages live in GitHub Releases; temporary
Actions artifacts expire after seven days.

## Repeat a build

- The same version at a different commit produces a different nightly tag.
- The same version and commit reuse the published nightly and skip the builds.
- An interrupted upload leaves a draft. Use Re-run failed jobs to retry the
  publisher, or start another manual run to rebuild and complete that draft.
- A short SHA collision stops the run. An existing tag cannot be moved to
  another commit.
- Published packages are never replaced by these workflows.

## Promote a stable release

Test the nightly packages on the target systems first. Then open
**Actions → Promote Stable → Run workflow** and enter its exact nightly tag,
such as `v0.2.0-a1b2c3d`.

Promotion checks the nightly's tag, full source SHA, platform package set,
sizes, and checksums. It copies the same package bytes to a new release and
removes the SHA suffix from download names. The stable tag is `v0.2.0` at the
nightly's source commit. The app's numeric version stays the same.

The nightly remains available. GitHub selects Latest from stable releases by
its [version/date rule](https://docs.github.com/en/rest/releases/releases#update-a-release); promoting an older version does not request that it
replace a newer Latest release. Repeating the same promotion reuses the existing
stable release. Promoting another commit to an existing stable version stops
with an error.

## Signing and platform limits

Mac packages use ad hoc signing and are not notarized. macOS can require manual
approval before opening them or a new Screen Recording grant after an update.
The workflow has no Apple account credentials and does not use the local
development certificate. Public Developer ID signing and notarization need a
separate setup.

Windows packages are portable ZIP files without an installer or Authenticode
signature. Linux packages are built on Ubuntu 24.04 and need compatible system
libraries, including the graphics and capture libraries listed in the README.
They are not AppImages or distribution packages.

Local release checks:

```sh
python3 -m unittest discover -s scripts -p test_release.py
cargo metadata --format-version 1 --no-deps --locked
```

These checks do not publish a release or start a workflow.
