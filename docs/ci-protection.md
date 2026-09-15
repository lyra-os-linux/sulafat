# Required CI for main

The GitHub branch protection for `main` requires the check named `contracts`,
published by GitHub Actions (app ID `15368`). The check name comes from the
published check run, rather than the workflow title `Contracts`.

The branch must be up to date before merging (`strict=true`). Protection applies
to administrators (`enforce_admins=true`). Changes use pull requests; no extra
reviewer approval is required. Force pushes and branch deletion remain disabled.
This policy is configured in GitHub settings, not by this Markdown file.

## Coverage

The existing workflow uses Debian Trixie for the GTK/VTE build dependencies and
runs `cargo test --workspace --locked`, Rust formatting and Clippy. Tests cover
SSH configuration parsing and persistence, atomic writes, symlink rejection,
permissions, external changes and metadata. Integration tests compare effective
configuration with OpenSSH `ssh -G`, without connecting to an SSH server.

The workflow also checks translations, validates desktop/AppStream metadata and
parses the RPM spec. PRs have no path or branch filter, and the required job has
no job-level skip condition.

These checks do not build or install an RPM, exercise the interactive terminal
in a native GNOME session, or qualify real remote SSH connections. They do not
replace signed OBS artifact qualification or the Lyra ISO gates. The container
tag and stable Rust toolchain can advance; this CI is not a reproducible release
build or a complete offline build qualification.

## Qualification

On 2026-09-15 `main` already had branch protection requiring pull requests,
but no required status checks; administrator enforcement was disabled. The
applicable-rules query returned an empty list. Only required status checks and
administrator enforcement were changed; the remaining protection was preserved.

[PR #4](https://github.com/lyra-os-linux/sulafat/pull/4) exercised actual merge
requests using the administrator account and the exact head revision
`0c856d109de44c1b40ee3efb6df666ec999752d0`:

| Required check state | Merge response |
| --- | --- |
| Running | HTTP 405: `Required status check "contracts" is in progress.` |
| Intentionally failed | HTTP 405: `Required status check "contracts" is failing.` |

The [controlled run](https://github.com/lyra-os-linux/sulafat/actions/runs/34996210160)
used a temporary failure step limited to that PR branch. Both refusals left
`main` at `a799db5beae52ba326bceff5b9fc8121964e8333`. The probe was then removed,
restoring the workflow byte for byte before running the full CI and integrating
this documentation. The final passing run and successful squash merge receipt
are recorded in [issue #1](https://github.com/lyra-os-linux/sulafat/issues/1).

## Exceptions and recovery

No user, team or app bypass is configured in this branch protection. Repository
administrators can still deliberately edit or remove the rules; administrator
enforcement constrains merges while the policy is in effect.

GitHub also accepts `neutral` and `skipped` check conclusions. Keep the required
job executing when changing workflow conditions. If the check name or producer
changes, update protection to the exact published name and app identity.

If an explicitly approved recovery requires reverting this change, remove the
required status checks and restore administrator enforcement to `false`, leaving
the existing PR requirement and all other safeguards intact. Diagnose a failing
check before considering that recovery; do not disable protection for routine
merges.
