# Porting upstream PRs into packages/bun-boa's patch series

Context: packages/bun-boa (in the Bun fork at /home/ay/.claude/src/bun, branch claude/nojsc) is a pure-Rust Bun on Boa (JS engine) + Blitz (HTML/CSS layout and paint). It builds against crates.io tarballs with patch series applied: patches/<crate>-<version>/NNNN-*.patch, materialised by scripts/vendor.ts into vendor/<crate>-<version>/ (git repos, tag `vendor-base` = pristine tarball, one commit per patch). The series already hold ~100 patches (kitesurf's fixes) on top of boa 0.22.0 and blitz 0.3.0-beta.4, so the code is older than upstream main AND already patched. Read patches/<crate>-<version>/ file names (and patch contents) before porting: a PR may already be covered by an existing patch.

Your job: port the PRs assigned below, one at a time, each as one commit per crate it touches, in your own clones (never touch vendor/ or patches/ directly, never commit to the Bun repo, never run vendor.ts).

Tool: scripts/port-pr.ts (read it). Always run from /home/ay/.claude/src/bun/packages/bun-boa with PORT_VENDOR set to your clone root (given below):
PORT_VENDOR=<root> bun scripts/port-pr.ts apply <owner/repo>#<n> # clones the crate on first use, applies the PR diff, rejected hunks → *.rej
PORT_VENDOR=<root> bun scripts/port-pr.ts commit <owner/repo>#<n> "<note>" # commits with the PR's author, date and title
The diff it applied is saved in build/<owner>-<repo>-<n>-<crate>.diff. `gh pr view <n> -R <repo>` / `gh pr diff` for the description and full diff.

Per PR:

1. apply. For each *.rej: port the hunk by hand (Edit tool) to the code as it is in your clone: same behaviour, adapted to the older APIs, file layout (upstream split/renamed files — find the equivalent code with rg) and to what earlier patches changed. Port in-crate unit tests too (src/**/tests.rs etc.); tests outside the crate (e.g. blitz's tests/blitz-tests/) are left out automatically and noted.
2. Delete every *.rej / *.orig. `git -C <clone>/<crate> status` must show only this PR's changes.
3. commit with a note of one or two plain sentences on how the port differs from upstream (e.g. "Adapted to boa 0.22: JsValue::variant() instead of …"). Empty note "" if it applied as is.
4. If the PR does not apply to this code base — it is already implemented by an existing patch (name it), or it depends on upstream features/refactors that are absent here and would have to be ported wholesale — do not commit it; revert the clone (`git checkout -- . && git clean -fd` in that crate, except never delete .git) and report the reason precisely.

Do not build or run cargo on this machine (builds run on the cluster later; I will compile and fix). So be careful: check every API you call exists with the signature you use in this code base (rg the clone, and dependency sources: vendor/<crate>-<version>/ for patched deps such as stylo, taffy, parley, selectors; ~/.cargo/registry/src/*/ for the rest). Keep imports right, no unused items.

Final report (it is the only thing I see): for each PR: `committed <crate> <short-sha> — <note>` or `skipped — <reason>`; plus anything risky I should check when compiling.
