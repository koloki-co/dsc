# Snapshot provenance

Spec for the common, informational provenance header on declarative snapshots. Goal: make a detached snapshot explain its origin without turning its origin into an application constraint.

## Contract

Settings, theme-settings, and board snapshots may carry this optional header:

```yaml
provenance:
  source: https://forum.example.com
  exporter_version: 0.24.0
```

- `source` is the configured base URL, normalized without trailing slashes and with URL user-info removed. It must never contain API credentials or other URL-embedded credentials.
- `exporter_version` is the `dsc` version that wrote the snapshot.
- Existing `pulled_at` and `discourse_version` fields remain separate because they describe the retrieval event and source server, rather than the exporting tool or site identity.
- A missing `provenance` field means provenance is unknown. Version 1 snapshots written before this header remain valid inputs.

## Apply and comparison behaviour

Provenance is informational only. Push and diff logic must ignore it: a reviewed snapshot may intentionally be copied from staging to production or between client forums. It must not create a semantic change, a warning-only write, or a same-site restriction.

## Scope

The initial R71 implementation covered `setting pull`, `theme setting pull`, and `board pull`. `tag pull` (taxonomy) and `category def pull` (definitions) adopted the same nested object on 2026-11-19; `tests/snapshot-provenance-test.rs` pulls each from a mock forum configured with URL user-info, asserts the credential-free header, then retargets `provenance.source` to another site and confirms `push --dry-run` plans no change and sends only GETs. `category def`'s file keeps `deny_unknown_fields`, so the provenance object is strict too. `theme palette pull` (and the deprecated `palette pull` alias) adopted it on 2026-11-28: pull, the refreshed snapshot after an update push, and both snapshots written after a create carry the header for the site just read; the same test file proves a retargeted palette snapshot still plans `= unchanged` under `push --dry-run` with only GETs. No schema `version` was added: the palette file never rejected unknown fields, so older `dsc` releases read new files unchanged and new releases read old files as unknown provenance.

### Remaining inventory

| Family | Current origin fields | Status |
| --- | --- | --- |
| `user-field pull` | top-level `source` string from `normalize_baseurl` | Deliberately different: push *enforces* `source` as a same-site guard before creation, which is the opposite of informational provenance. Note that `source` is not stripped of URL user-info the way `provenance.source` is; any change needs both pull and the push comparison to strip together. Needs an author decision before adopting `provenance`. |
| `palette pull` / `theme palette pull` | `provenance` (2026-11-28) | Adopted; the file stays unversioned because it tolerates unknown fields in both directions. |
| `theme pull` / `theme field pull` | none | Candidate; theme payload files mirror the API object, so a header needs push to strip it. |
| `topic`/`post`/`category pull` (Markdown front matter) | `url`, `pulled_at` | Out of scope: the front-matter `url` already identifies the source. |
| `emoji pull`, `backup pull` | binary assets/archives | Out of scope: no declarative document to carry a header. |
