<!-- Keep PRs focused on one change. See CONTRIBUTING.md for full conventions. -->

## What and why

<!-- One paragraph: what changed, and what real need drove it. -->

## Checklist

- [ ] `s/test-fmt-clippy` passes (fmt, strict Clippy, full test gate)
- [ ] New/changed command paths have unit tests, plus a `tests/` integration test if it talks to Discourse
- [ ] Docs updated: `--help`/doc-comment, `docs/<command>.md`, and a README row if it's a new top-level command
- [ ] Conventional commit prefix (`feat(area):`, `fix(area):`, `docs:`, …)
- [ ] If this adds a new command surface: does it support every lifecycle verb the API exposes - create/read/update/delete/list? If a verb is missing, note why (e.g. the API doesn't expose it) or file a roadmap entry for it (see [spec/roadmap.md](../spec/roadmap.md) R61)
