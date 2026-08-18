# `dsc backup health` - fleet visibility of newest backup and S3 bucket growth

> **Status: Phase 1 implemented in v0.13.0; endpoint/frequency/streaming follow-up implemented on `main`.** `dsc backup health` checks one forum, all configured forums, or a tag-selected fleet; inspects actual paginated S3-compatible objects through the AWS CLI; reports newest archive age/size, configured frequency, and total bucket accounting in text, JSON, YAML, or CSV; and exits non-zero for unhealthy rows.

Spec for a read-only S3-backed backup health check. Goal: show whether every configured Discourse is producing recent backups, how large the newest archive is, and whether its backup bucket is growing unexpectedly. Driver: a recurring fleet check currently performed with ad hoc AWS CLI scripting to find the newest object in each backup bucket and inspect its upload time and size.

## Motivation

A backup configuration can look healthy while scheduled backups silently stop, credentials expire, uploads fail, or retention stops deleting old archives. The operator needs one fleet view that answers three questions for every relevant forum: when did the newest backup reach off-site storage, how old is it, and how much storage does the backup bucket consume? Today that requires manually resolving bucket names and running `aws s3 ls --recursive` for each bucket, which is slow and easy to skip.

## Current state (as of 2026-07-28)

`dsc backup list <discourse>` asks Discourse for its known backups and shows filename, date, size, and configured location. It cannot prove that objects are still present in S3, calculate total bucket storage, or check a fleet in one invocation. `dsc backup setup-s3` provisions the standard one-bucket-per-forum pattern through the AWS CLI, but does not retain AWS credentials or inspect ongoing health.

## Proposed CLI surface

```text
dsc backup health [<discourse>] [--tags <tag1,tag2,...>] [--max-age <days>] [--format text|json|yaml|csv]
```

- **`dsc backup health`** - checks every configured forum with S3 backups. For each forum, resolves its backup bucket and region from the Discourse site settings (`backup_location`, `s3_backup_bucket`, `s3_region`), lists S3 objects, identifies the most recently modified backup archive, sums all object bytes in the bucket, and prints one row. Forums without `backup_location=s3` are reported as `not_s3`, not silently omitted.
- **`dsc backup health <discourse>`** - checks one named forum. This is useful for investigation and follows the normal single-forum command pattern.
- **`--tags <tag1,tag2,...>`** - checks forums matching any configured tag, consistent with fleet commands. It is mutually exclusive with `<discourse>`.
- **`--max-age <days>`** - optional relaxed health threshold. The effective threshold is the greater of this value and the forum's `backup_frequency` site setting, so `dsc` never calls a scheduled backup stale before its configured interval has elapsed. With no flag, the site setting is authoritative. Frequency `0` is `disabled` rather than stale.
- **`--format text|json|yaml|csv`** - text is a concise stable table; structured rows are designed for monitoring and include endpoint, configured/effective thresholds, raw bytes, and RFC 3339 timestamps. Text and CSV rows stream and flush as each bucket completes. JSON and YAML remain buffered and sorted so each invocation emits one valid document. Empty structured output is `[]`; text says `No Discourses selected.`

Example text output:

```text
Forum          Status        Latest backup                                    Age   Every   Latest size   Bucket size Bucket
koloki-demo    OK            koloki-demo-2026-07...v20260701000000.tar.gz       0d      1d        1.8 GB       14.2 GB koloki-demo-discourse-backups
client-forum   STALE         client-2026-07-22-02...v20260701000000.tar.gz      8d      7d        6.4 GB       87.1 GB client-forum-discourse-backups
legacy-forum   NOT_S3        -                                                  -       -             -             - -
```

Example JSON row:

```json
{
  "discourse": "koloki-demo",
  "status": "ok",
  "bucket": "koloki-demo-discourse-backups",
  "region": "eu-west-2",
  "backup_frequency_days": 1,
  "stale_after_days": 1,
  "latest_key": "backups/default/koloki-demo-2026-07-28-020001-v20260701000000.tar.gz",
  "latest_modified_at": "2026-07-28T02:00:17Z",
  "age_days": 0,
  "latest_size_bytes": 1932735283,
  "bucket_size_bytes": 15247133902,
  "bucket_object_count": 8
}
```

## Data source and semantics

The health command uses the AWS CLI and adds no Rust AWS SDK dependency. It obtains bucket, region, optional custom endpoint, backup frequency, static access key, static secret, and IAM-profile selection from the Discourse admin site settings. Static secrets remain process-local, are passed only through the child environment, and are skipped by every serializer. IAM-profile forums use the ambient AWS credential chain. It invokes one paginated read command per selected forum:

```bash
aws s3api list-objects-v2 --bucket <bucket> --region <region> --output json
# Custom providers additionally receive:
aws s3api list-objects-v2 ... --endpoint-url <s3_endpoint> --output json
```

For buckets with more than 1,000 objects, `dsc` follows `NextContinuationToken` until `IsTruncated` is false. It sums `Contents[].Size` across every returned object, picks the newest `LastModified` backup archive, and records the total object count. It must inspect object keys rather than assume a flat bucket because Discourse commonly writes under `backups/default/`.

An object qualifies as a backup archive only when its basename matches Discourse backup formats (`.tar.gz` or `.tar`) and is not an unrelated report, manifest, or S3 service artifact. The total bucket size intentionally includes every object, not only backup archives: stray uploads and abandoned artifacts are the storage-growth problem this command should surface.

`age_days` is `floor(now_utc - latest_modified_at)` in UTC. A future timestamp is reported as `0` days with a warning in the row detail, never a negative age. The CLI does not infer backup health from a filename date.

## S3 permission model

The standard per-forum backup credentials already grant `s3:ListBucket` on the bucket and object-level access to its contents. `backup health` therefore needs no additional permission. The read operation itself is the provider-neutral credential and reachability check; an AWS STS preflight is deliberately not used because S3-compatible providers do not expose it. The command distinguishes these outcomes per forum:

- `not_s3` - Discourse says backup location is not S3.
- `misconfigured` - S3 is selected but bucket or region is missing.
- `missing` - S3 listing succeeds but contains no backup archive.
- `stale` - latest archive exists and exceeds the effective site-frequency threshold.
- `disabled` - `backup_frequency=0`; any existing archive is reported but is not classified stale.
- `ok` - latest archive is within threshold.
- `inaccessible` - the S3 provider denies or fails the bucket request; stderr is summarized without exposing credentials.
- `unknown` - site-setting lookup or timestamp parsing failed.

## Reference: API calls observed in the field

Driver is the existing per-forum S3 backup layout created by `dsc backup setup-s3` and older manual/provider-specific runbooks.

```text
GET /admin/site_settings.json
Api-Key: <redacted>
Api-Username: <admin>

→ locate values for:
  backup_location=s3
  backup_frequency=1
  s3_backup_bucket=<forum>-discourse-backups
  s3_region=eu-west-2
  s3_endpoint=<optional S3-compatible endpoint>
  s3_access_key_id=<redacted>
  s3_secret_access_key=<redacted>
  s3_use_iam_profile=false
```

```bash
aws s3api list-objects-v2 \
  --bucket <forum>-discourse-backups \
  --region eu-west-2 \
  --output json
```

Read-only field observation on 2026-08-18 against Discourse stable at WeAllCount: `backup_location=s3`, `backup_frequency=1`, `s3_backup_bucket=weallcount-forum-discourse-backups`, `s3_region=us-east-1`, and `s3_endpoint=https://sfo3.digitaloceanspaces.com`. Credential values were confirmed present without retaining or printing them. An endpoint-aware `list-objects-v2` returned the current DigitalOcean Spaces backup inventory successfully.

Representative S3 response:

```json
{
  "IsTruncated": false,
  "Contents": [
    {
      "Key": "backups/default/forum-2026-07-28-020001-v20260701000000.tar.gz",
      "LastModified": "2026-07-28T02:00:17+00:00",
      "Size": 1932735283
    }
  ]
}
```

## Phases

### Phase 1 - blocking

- [x] Add `dsc backup health` for all configured forums, one forum, and `--tags` selection.
- [x] Resolve S3 location, bucket, and region from Discourse settings; report non-S3 and incomplete configurations explicitly.
- [x] Preflight the ambient AWS CLI once and paginate `s3api list-objects-v2` safely for every unique bucket.
- [x] Report newest archive name, actual S3 modification timestamp, elapsed days, newest archive bytes, total bucket bytes, and object count.
- [x] Add `--max-age` with non-zero aggregate exit status for stale/missing/inaccessible/misconfigured results.
- [x] Implement text, JSON, YAML, and CSV output with stable machine fields and no credential output.
- [x] Add offline fixture tests for archive selection, byte totals, age boundaries, tag selection, and output status fields.
- [x] Read `backup_frequency` and never classify an archive stale before the configured schedule has elapsed.
- [x] Stream and flush text/CSV rows as bucket reads complete while retaining buffered JSON/YAML documents.
- [x] Support S3-compatible `s3_endpoint` settings and process-local static credentials; verified read-only against DigitalOcean Spaces.

### Phase 2 - iteration ergonomics

- [ ] `--bucket`/`--region` explicit legacy override.
- [ ] Optional per-forum expected maximum bucket size in `dsc.toml`; freshness already follows each forum's `backup_frequency` site setting.
- [ ] Optional `--details` to show every backup archive and non-backup object prefix responsible for bucket growth.
- [ ] Emit a monitoring-friendly one-line format or Prometheus textfile output only if a real monitoring integration needs it.

### Phase 3 - remediation, only on real demand

- [ ] Add a reviewed lifecycle/retention rule workflow under `backup setup-s3` rather than deleting objects directly.
- [ ] Consider a guarded cleanup plan that lists candidate old archives and requires explicit confirmation, only after validating Discourse's retention behavior and versioned-bucket semantics.

## Backward compatibility

Purely additive under the existing `backup` command. Existing `backup list` remains the source of Discourse's own backup catalogue; `backup health` adds independent S3 evidence and bucket accounting. No existing AWS configuration is changed.

## Out of scope

- Creating a backup or writing to Discourse/S3.
- Deleting old archives or changing retention automatically.
- Provider-specific bucket and credential provisioning; `backup health` is read-only and endpoint-compatible, while `backup setup-s3` remains AWS-specific.
- S3 version-history, Glacier storage-class, billing, Storage Lens, or replication analysis.
- Upload-asset bucket health; this covers backup buckets only.
- Treating a recent archive as proof that the archive can be restored successfully.
