# Contributing to Agent Console

Thanks for considering a contribution. This document is short on purpose —
read it once before your first pull request.

## License of contributions

Agent Console is distributed under the
[GNU AGPL-3.0-only](LICENSE) (releases up to and including v0.75.0 remain
MIT). To keep the project's licensing future flexible — including the option
to offer the code under additional licenses later — contributions are
accepted under **inbound = MIT**:

> By submitting a contribution to this repository, you agree that your
> contribution is licensed under the [MIT License](https://opensource.org/license/mit),
> and that the project may distribute it as part of the AGPL-3.0-only
> combined work (and under other license terms the maintainer chooses).

This is deliberately more permissive inbound than outbound. It is what makes
the licensing reversible and dual-licensing possible without ever needing a
retroactive CLA conversation. If this arrangement doesn't work for you,
open an issue and say so before writing code — we'd rather discuss it first.

## Developer Certificate of Origin (DCO)

Every commit must be signed off, certifying the
[Developer Certificate of Origin 1.1](https://developercertificate.org/):
you wrote the change, or otherwise have the right to submit it under the
licenses above.

Sign off by adding a `Signed-off-by` trailer with your real name and email —
`git commit -s` does it for you:

```
Signed-off-by: Your Name <you@example.com>
```

CI rejects pull requests containing commits without a sign-off. Forgot one?
`git commit --amend -s` (last commit) or `git rebase --signoff main` (whole
branch) and force-push your branch.

## Practical notes

- Match the surrounding code: comment density, naming, formatting. CI runs
  `cargo fmt --check`, clippy with `-D warnings`, `prettier --check`, and
  both test suites — run them locally before pushing.
- Changes to GUI behavior need a human verification note in the PR: what you
  clicked, on which platform, and what you saw. Webview quirks (WebKitGTK
  especially) have a history of passing every automated gate and failing on
  a real desktop.
- Keep pull requests scoped to one change. If you found a second bug on the
  way, open a second PR — small PRs merge fast here.
