# The fork

This is a fork of [herdr-core](https://github.com/herdrdev/herdr-core), taken at `ec8adcd` (0.1.17). Upstream is Apache-2.0. [LICENSE](LICENSE) is upstream's, unchanged. So is [CHANGELOG.md](CHANGELOG.md), which describes upstream's releases, not ours.

**Changed files (Apache-2.0, section 4(b)).** Every file from upstream that holds one of its names was changed by the rename: 117 of the 170 entries in upstream's tree at `ec8adcd`, and 107 of them also moved. The first line of `crates/limen-core/THIRD-PARTY-NOTICES`, and of what `scripts/notices` writes there, says this is a modified fork of herdr-core. Everything of ours since then shows in `git diff upstream-renamed main`.

## How the names change

`scripts/limenize.py` holds every rule, and it reads git objects only. It writes upstream's tree with the new names as a commit on the branch `upstream-renamed`, and `main` merges that branch. A later upstream release then conflicts only where we changed the same lines, never over names.

| Upstream | Here |
|---|---|
| `herdr`, `Herdr`, `HERDR` | `limen`, `Limen`, `LIMEN` |
| `hc`, `hcs`, `nohc` | `lc`, `lcs`, `nolc` |
| `HC_` at the start of a word | `LIMEN_` |
| `HC` as a word | `LC` |
| `releases.herdr.dev` | `releases.limen.invalid` |

- **Left out of the renamed tree:** `README.md`, `AGENTS.md` and `SECURITY.md`, which are ours on `main`; `.github/`, which will be ours; `CONTRIBUTING.md`; and `tests/macos_distribution.rs`, which reads upstream's release workflow.
- **Left as written:** `LICENSE`, `CHANGELOG.md`, `distribution/notices/*.txt`, `distribution/windows/*`.
- **Refused:** the script writes nothing when an old name survives, a new name collides with one already there, or a name lands on one the Python Limen owns. It lists all of them first.
- **Formatted:** `rustfmt` runs on each changed `.rs` file, since the longer names push some lines past 120 columns. At `ec8adcd` that changed 4 test files.

## Taking an upstream release

```sh
git fetch upstream
python -B scripts/limenize.py sync --ref upstream/master --trailer "<attribution line>"
git merge upstream-renamed
```

Run it from any branch but `upstream-renamed`. `check` in place of `sync` writes nothing, and `--report <file>` saves the full list: every problem, and every distinct name it changed with its new form. Read that list when a release adds names. The script's own tests are `python -B scripts/limenize_test.py`.

## What the rename leaves open

- **No CI yet.** `.github/` is left out, and ours comes once GitHub Actions can run on this account.
- **`tests/upgrade.rs`** needs a release of ours to upgrade from, so it can't pass across the rename until there is one.
- **The app.** The manual, the skill and some error hints mention `limen-app` and "open Limen", and `api/run.rs` expects the app to set `TERM_PROGRAM=limen`. All of that means the desktop app, which arrives later (`audit/SETTLE.md`, step 8).
- **macOS.** `platform/macos_bundle.rs` still checks upstream's Apple team id. We ship no macOS builds, so nothing here can pass that check, or needs to.
- **One sentence changed by hand.** The manual and the skill called `limen` "the older terminal multiplexer". That was true of `herdr` and false here, so they now say the older Python worker tool.
