# `dsc` guarantee index

R72's named-guarantee approach, borrowed from Oli Evans's [`ods`](https://github.com/olizilla/ods/blob/7f95000/docs/tests.md): every cross-cutting behavioural promise `dsc` makes gets a short name here and a pointer to the test(s) that actually enforce it, so a reviewer can check a guarantee still holds without re-deriving it from the command tree, and so a new command surface can be checked against the same list before it ships. This indexes existing coverage as well as new coverage added alongside it; it does not replace any of the linked test files, and adding a guarantee here does not itself enforce anything - the linked test does.

Each guarantee below is phrased as a promise to someone running `dsc`, not as an implementation detail. "Enforced by" is the test(s) that would fail if the promise were broken; most entries include a brief note on *how* the test proves it, since passing a test and actually proving the promise are not automatically the same thing.

## Output and CLI surface

| Guarantee | Enforced by |
|---|---|
| Every `* list` command supports `--format text\|json\|yaml` at minimum | `src/cli.rs::every_list_command_offers_the_standard_formats` - walks the full clap command tree and checks every `list`-named subcommand's `--format` argument for all three values. |
| Every top-level command documents at least one `Examples:` block | `src/cli.rs::every_top_level_command_documents_examples` - walks the command tree checking `after_help`. |
| Every path-shaped argument has a shell-completion hint | `src/cli.rs::every_path_argument_has_a_completion_hint` |
| Piped (non-terminal) stdout/stderr carry no ANSI or terminal-image escapes unless the operator explicitly forced them (`DSC_COLOR=always`, `DSC_EMOJI_INLINE_PROTOCOL`); `NO_COLOR` beats `DSC_COLOR=always`; inherited terminal hints (`TERM_PROGRAM`, `KITTY_WINDOW_ID`, `TERM=xterm-kitty`) never enable escapes into a pipe | `tests/non-terminal-output-test.rs` - black-box over every escape-emitting surface: `dsc update`'s coloured per-forum labels (via a refusing `ssh` stub), `emoji list --inline` image protocols (via a mock serving list and image), and clap help/usage errors. Each suppression has a forced-on positive control; mutation-checked by disabling the stdout-terminal check in `color_discourse_label` and in emoji inline detection. |

## Dry-run

| Guarantee | Enforced by |
|---|---|
| Every command is explicitly triaged as either a dry-run mutation case or a reviewed no-mutation leaf - a new command cannot silently go untested for `--dry-run` | `tests/dry-run-mutation-test.rs::every_command_is_triaged_for_dry_run` - walks the command tree and fails if any leaf is in neither the `CASES` registry nor `NO_SERVER_MUTATION_LEAVES`. |
| `--dry-run` never issues a mutating HTTP request, for every command in the `CASES` registry | `tests/dry-run-mutation-test.rs::dry_run_never_issues_a_mutating_request` - runs each case against a mock server and asserts no non-GET request lands. |
| A command without a complete dry-run plan refuses before any side effect, including config resolution | `tests/dry-run-safety-test.rs::dry_run_unsafe_commands_short_circuit_before_config_resolution` (black-box: points `$DSC_CONFIG` at a file that must never be read) plus `src/cli.rs::dry_run_refusal_reason_identifies_handlers_without_complete_dry_runs` / `dry_run_refusal_reason_allows_stdout_and_dry_run_protected_commands` (pure-function: the classification itself). |
| For every declarative-push family or sub-path covered by a test in `tests/dry-run-plan-fidelity-test.rs` (as of writing: `setting push`, `theme setting push`, `palette push` update, `tag push` description-update/group-create-update/`--prune`-deletion, and `category push`), the dry-run plan's stated targets and values exactly match what a real run sends against the same frozen state | `tests/dry-run-plan-fidelity-test.rs` (one `#[test]` per family or sub-path, sharing `assert_dry_run_plan_matches_real_run`) - R73. Each has been mutation-checked: deliberately sending the wrong value from the apply path makes the relevant test fail. Read the file itself for the current, authoritative list rather than relying on this row staying in sync with every new test added. |

## Config-free commands

| Guarantee | Enforced by |
|---|---|
| `dsc completions`, `dsc man`, and bare `dsc version` never resolve or require configuration, even when none is discoverable | `tests/config-free-generation-test.rs::generation_commands_do_not_resolve_config` |

## Config discovery and selection

Pure-function coverage calls `resolve_config_source_with_env` / `config_search_paths_with_env` directly with an injected environment map - fast, and proves the resolution *logic* in isolation. Black-box coverage runs the compiled binary with no `-c` flag inside a hostile sandbox where every discovery location holds a plausible, misleading file - slower, but proves the *whole program* behaves this way, not just the function that happens to implement it today.

| Guarantee | Pure-function | Black-box |
|---|---|---|
| `--config`/`-c` wins over `$DSC_CONFIG` and everything discoverable | `src/config.rs::flag_wins_over_env_and_discovery` | - |
| A missing `--config` target errors rather than falling through | `src/config.rs::missing_flag_path_errors` | - |
| `$DSC_CONFIG` wins over everything discoverable | `src/config.rs::dsc_config_env_wins_over_discovery` | `tests/hostile-config-test.rs::explicit_dsc_config_wins_over_misleading_discoverable_files_and_contacts_only_that_forum` - both discoverable locations hold a *valid* decoy naming the same discourse, whose baseurl is a closed port; only a real mock server succeeds, so the test also proves no accidental connection to a decoy is attempted. |
| A missing `$DSC_CONFIG` target errors rather than falling through to a valid discoverable config | `src/config.rs::missing_dsc_config_env_path_errors` | `tests/hostile-config-test.rs::missing_explicit_dsc_config_errors_without_falling_through_to_valid_discoverable_files` - both discoverable locations are valid and would check out cleanly if used, so the refusal is proven real rather than incidental. |
| `$DSC_CONFIG_HOME` redirects step 2 of the search order; unset, it reproduces `$XDG_CONFIG_HOME/dsc` -> `~/.config/dsc` | `src/config.rs::dsc_config_home_redirects_step_4`, `unset_config_home_reproduces_home_config_dsc`, `xdg_config_home_default_used_when_dsc_config_home_unset`, `dsc_config_home_overrides_xdg_config_home` | - |
| With nothing set, the search order is byte-for-byte the legacy order (regression guard) | `src/config.rs::unset_everything_resolution_matches_legacy_order`, `no_config_anywhere_returns_default` | - |
| A higher-precedence discoverable file that exists but fails to parse is a hard error - it is never silently skipped in favour of a valid lower-precedence candidate | - | `tests/hostile-config-test.rs::malformed_file_at_the_winning_discovered_path_errors_without_trying_the_next_candidate` - `./dsc.toml` is garbage, `$HOME/.config/dsc/dsc.toml` is valid; the command must still refuse. |
| A malformed explicit `--config` file is a hard error - it is never skipped in favour of a valid `$DSC_CONFIG` or discoverable config | - | `tests/hostile-config-test.rs::malformed_explicit_config_flag_errors_without_falling_through_to_valid_env_or_discoverable_files` - `--config` is garbage while `$DSC_CONFIG` and both discoverable files are valid; the command must refuse and rewrite nothing. |

## No unexpected writes

| Guarantee | Enforced by |
|---|---|
| A read-only or refused command writes nothing anywhere in its environment - not a new file, not a changed byte | Every `tests/hostile-config-test.rs` test snapshots both hostile directories (`$HOME` and the working directory) before and after, asserting byte-for-byte equality. |

## Discourse ID typing

| Guarantee | Enforced by |
|---|---|
| A new `.as_u64()` call site in `src/` cannot land without an explicit, reviewed justification that the field can never be negative (Discourse reserves negative IDs for built-in/system entities - users, themes, colour schemes) | `tests/discourse-id-typing-test.rs::every_as_u64_call_site_is_reviewed_and_allowlisted` - scans `src/` for the literal substring and fails on any site not in the file's allowlist. |

## Extending this index

Adding a new cross-cutting guarantee (a new hostile-environment scenario, a new declarative-push family under R73's dry-run-fidelity harness, a new command-tree-wide property): write the test first, confirm it actually fails on a deliberately broken version of the behaviour it claims to enforce (the standing convention throughout this repo's test suite - see the commits that added each file above), then add one row here naming the promise and pointing at the test. Do not add a row for a property that is not yet tested; this index links tests, it is not a TODO list (that is `spec/roadmap.md`'s job).
