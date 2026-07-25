# `dsc backup setup-s3` - provision an S3 backup bucket + scoped IAM user and point Discourse at it

> **Status: Phase 1 implemented (unreleased).** `dsc backup setup-s3 <discourse>`
> ships: derive names, create bucket + single-bucket policy + user + key (via
> `aws` CLI), set the Discourse S3 settings (using the fixed `update_site_setting`
> path), optional verification backup, and a complete offline `--dry-run`.
> Phase 2 (`--reuse-user`, `--use-iam-profile`, `--all`/`--tags`) and Phase 3
> (native SDK, `--retention`, `backup status`) remain planned.

Spec for one-command setup of off-site Discourse backups on Amazon S3. Goal: replace a ~15-step AWS-console + Discourse-settings runbook with a single `dsc` command. Driver: the Koloki fleet - every self-hosted forum needs off-site backups, and the secure pattern (one bucket + one dedicated single-bucket IAM user per forum) is set up by hand in the AWS console for each one. Production runbook in use since 2023-01.

## Motivation

Every self-hosted Discourse should ship backups off-box, and the secure, least-privilege pattern is: one S3 bucket per forum, one dedicated IAM user per forum, and a policy that grants that user access to ONLY that bucket. Today this is a manual AWS-console slog repeated per install:

1. Create a private bucket `<name>-discourse-backups` (Block All Public Access on, SSE-S3, versioning off, ACLs disabled).
2. Create a managed policy `s3-single-bucket-<name>-discourse-backups` from hand-written JSON (the console's visual editor is avoided; the JSON tab is used).
3. Create an IAM user `<name>-discourse-backup-user`, access-key type, attach the policy directly.
4. Copy the access key id + secret.
5. Paste them + region + bucket into Discourse's S3 backup site settings.
6. Trigger a manual backup to verify, then confirm the object landed in the bucket.

It is fiddly, error-prone (easy to over-scope the policy or mis-name things), and the AWS console permissioning is hostile. `dsc` already owns the Discourse side (site settings + `dsc backup`), so it is the natural place to drive the whole flow.

## Current state (as of 2026-06-24)

- `dsc backup` does create/list/restore against a forum, but assumes the destination is already configured.
- Nothing in `dsc` provisions AWS resources or sets the S3 backup site settings - the entire runbook above is manual.
- The Discourse side is a handful of site settings (`backup_location`, `s3_backup_bucket`, `s3_region`, `s3_access_key_id`, `s3_secret_access_key`, or `s3_use_iam_profile`) - settable via the admin API today, but not wired into any backup-setup flow.

## Proposed CLI surface

```text
dsc backup setup-s3 <discourse> [--region <r>] [--bucket <name>] [--reuse-user]
                                [--use-iam-profile] [--no-test] [--dry-run]
```

- Derives names from the `dsc.toml` entry's `name` (default scheme, overridable with `--bucket`):
  - bucket  `<name>-discourse-backups`
  - policy  `s3-single-bucket-<name>-discourse-backups`
  - user    `<name>-discourse-backup-user`
- Default `--region eu-west-2` (the runbook default; override per customer / country).
- Provisions AWS (via the `aws` CLI - see dependency note): create the private bucket (Block Public Access on, SSE-S3, versioning off, object-ownership BucketOwnerEnforced), create the single-bucket managed policy, create the IAM user, attach the policy, mint one access key.
- Configures Discourse over the admin API: `backup_location=s3`, `s3_backup_bucket`, `s3_region`, and the minted `s3_access_key_id` / `s3_secret_access_key` (or skip the keys and set `s3_use_iam_profile=true` with `--use-iam-profile` when running on an EC2 instance role).
- Unless `--no-test`, runs a `dsc backup create` and confirms the object appears in the bucket (`aws s3 ls`), then reports pass/fail.
- `--dry-run` prints the full plan - resolved names, the exact policy JSON, the `aws` commands, and the settings diff - and touches nothing. This is the review gate before acting on a cloud account, so it must be complete.
- `--reuse-user` skips user/policy creation and only (re)points Discourse at an existing bucket/user (idempotent re-runs, key rotation).

### Dependency / credentials note

AWS provisioning needs credentials with IAM + S3 admin rights. Two options, in rough order of fit for `dsc`'s lightweight stance:

- **Phase 1: shell out to the `aws` CLI** (must be installed + configured with the operator's provisioning profile). `dsc` builds the args + the policy document, runs `aws`, parses the JSON output. No new Rust deps; reuses credentials the operator already has. Honest, documented dependency.
- **Phase 2 (optional): native AWS SDK** (`aws-sdk-s3`, `aws-sdk-iam`) if shelling out proves brittle - heavier build, but removes the `aws` CLI requirement.

The operator's AWS creds are used only for provisioning and are never stored by `dsc`. The minted access key is least-privilege (one bucket) and is written straight into the Discourse setting, not into `dsc.toml` - consistent with "no secret storage beyond `dsc.toml`".

## Reference: API calls observed in the field

Production runbook across the fleet since 2023-01; AWS console steps mapped to their `aws` CLI equivalents. Target: Discourse stable (2026.x) S3 backup settings; AWS S3 + IAM (global).

**Single-bucket IAM policy (the core artefact)** - list on the bucket, object actions confined to its contents:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    { "Effect": "Allow", "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::<NAME>-discourse-backups" },
    { "Effect": "Allow", "Action": "s3:*",
      "Resource": ["arn:aws:s3:::<NAME>-discourse-backups/*"] }
  ]
}
```

(Field-proven form. A tighter object-action set - `GetObject` / `PutObject` / `DeleteObject` / `AbortMultipartUpload` / `ListMultipartUploadParts` - is a reasonable hardening option; the `s3:*` here is already confined to the one bucket's objects.)

**AWS provisioning (`aws` CLI equivalents of the console steps):**

```bash
# 1. private bucket
aws s3api create-bucket --bucket <NAME>-discourse-backups \
  --region eu-west-2 --create-bucket-configuration LocationConstraint=eu-west-2
aws s3api put-public-access-block --bucket <NAME>-discourse-backups \
  --public-access-block-configuration \
  BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true
# (SSE-S3 + BucketOwnerEnforced/ACLs-disabled + versioning-off are the defaults for new buckets)

# 2. managed policy from the JSON above  ->  returns the Policy ARN
aws iam create-policy --policy-name s3-single-bucket-<NAME>-discourse-backups \
  --policy-document file://policy.json

# 3. dedicated user + attach + key  ->  returns AccessKeyId + SecretAccessKey
aws iam create-user --user-name <NAME>-discourse-backup-user
aws iam attach-user-policy --user-name <NAME>-discourse-backup-user --policy-arn <POLICY_ARN>
aws iam create-access-key --user-name <NAME>-discourse-backup-user
```

**Discourse side (admin API).** Order matters: set the bucket/region/credentials
first and flip `backup_location` to `s3` **last** ("enable last"). Discourse
validates `backup_location=s3` against the S3 settings being present, so writing
it first can `422` - leaving AWS provisioned but Discourse half-configured.
(Same pattern as enabling reply-by-email.)

```text
PUT /admin/site_settings/s3_backup_bucket       s3_backup_bucket=<NAME>-discourse-backups
PUT /admin/site_settings/s3_region              s3_region=eu-west-2
PUT /admin/site_settings/s3_access_key_id       s3_access_key_id=<minted>
PUT /admin/site_settings/s3_secret_access_key   s3_secret_access_key=<minted>
PUT /admin/site_settings/backup_location        backup_location=s3    # LAST
# OR, on an instance role:  s3_use_iam_profile=true   (omit the two key settings)
```

Verify: `dsc backup create <discourse>` then `aws s3 ls s3://<NAME>-discourse-backups/` shows the new dump.

Caveat (2026-06-24): `dsc setting set` does not persist site settings reliably (issue #19), so the Discourse-side writes must currently go via the admin API directly; `setup-s3` should use whatever write path that fix lands on.

## Phases

### Phase 1 - blocking (the PITA being removed)

- [x] `dsc backup setup-s3 <discourse>` end-to-end via `aws` CLI shell-out: derive names, create bucket + policy + user + key, set the Discourse S3 settings, optional test backup (polls `aws s3 ls` for the dump).
- [x] `--dry-run` prints resolved names, the policy JSON, the `aws` commands, and the settings diff; touches nothing (offline - no `aws`/network needed).
- [x] Pre-flight: `aws sts get-caller-identity` works + forum reachable (`/about.json`). (Bucket-name availability is left to `create-bucket`, which errors if taken.)

### Phase 2 - iteration ergonomics

- [ ] `--reuse-user` / idempotent re-run + key rotation (`create-access-key`, update the setting, optionally deactivate the old key).
- [ ] `--use-iam-profile` path (EC2 instance role, no static keys).
- [ ] `--all` / `--tags` to set up backups across multiple forums (mirrors `dsc backup create --all`).

### Phase 3 - nice to have

- [ ] Optional native AWS SDK backend (drop the `aws` CLI dependency).
- [ ] `--retention <days>` lifecycle rule (expire old backups in the bucket).
- [ ] `dsc backup status <discourse>` - report destination, last backup, bucket object count/size.

## Backward compatibility

Purely additive - a new `setup-s3` subcommand under the existing `dsc backup`. No change to `backup create/list/restore`. The one shared dependency is the site-setting write path, which must be the fixed/reliable one.

## Out of scope

- Non-AWS S3-compatible providers (Backblaze B2, MinIO, DO Spaces). The setting plumbing is nearly identical (`s3_endpoint`), but bucket/IAM provisioning is provider-specific; revisit on demand.
- Upload (assets) S3 storage - this is the **backup** bucket only. The runbook is explicit: do not reuse the upload bucket. A separate `setup-s3-uploads` could follow the same pattern later.
- Restoring from S3 (already covered by `dsc backup restore`).
- AWS billing / Storage Lens dashboards.
- Holding the operator's AWS provisioning credentials (uses the ambient `aws` CLI profile; never stored by `dsc`).
