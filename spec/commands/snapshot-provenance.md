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

## Initial scope

The initial R71 implementation covers `setting pull`, `theme setting pull`, and `board pull`. Other snapshot families remain inventory work; they should adopt this exact nested object where their file schemas gain provenance.
