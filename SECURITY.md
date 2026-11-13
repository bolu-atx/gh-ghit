# Security policy

## Report a vulnerability

Report privately through GitHub:
[Security → Report a vulnerability](https://github.com/bolu-atx/gh-ghit/security/advisories/new).
Do not open a public issue.

Include the ghit version (`ghit --version`), your OS and terminal,
and the steps to reproduce. Expect a first reply within 7 days.

## Supported versions

Only the latest release gets fixes.

## Scope

ghit shows text from GitHub that anyone can write: issue and PR
bodies, comments, titles, labels and diffs. These are in scope:

- Text from GitHub that changes the terminal, for example escape
  sequences that set the title, write the clipboard or move the cursor.
- Opening a URL, file or program that the user did not choose.
- Leaks of the GitHub token, for example to logs, the cache or
  another host.
- Local files that other users on the machine can read or change.
- Release binaries or CI workflows that can be tampered with.

## Verify a release binary

Each release has a `SHA256SUMS` file and a build-provenance
attestation:

```bash
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify gh-ghit-<os>-<arch> -R bolu-atx/gh-ghit
```
