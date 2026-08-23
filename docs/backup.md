# dsc backup

Create, list, download, restore, and set up off-site (S3) backups.

## dsc backup health

```text
dsc backup health [<discourse>] [--tags <tag1,tag2,...>] [--max-age <days>] [--format text|json|yaml|csv]
```

Checks actual S3 objects rather than Discourse's backup catalogue. With no `<discourse>`, checks every configured forum; use `--tags` to select a fleet subset. Every row reports the latest S3 backup archive, its age, its size, total bucket size, total object count, and configured backup frequency. Forums using local backups are reported as `NOT_S3`.

The command reads each forum's `backup_frequency` site setting and marks its latest archive `STALE` only when its age in whole days exceeds that configured interval. `--max-age` can relax the threshold for an invocation but cannot make it stricter than the forum's own schedule. A frequency of `0` is reported as `DISABLED`, not stale. `MISSING`, `STALE`, `MISCONFIGURED`, `INACCESSIBLE`, or `UNKNOWN` rows make the command exit non-zero after printing all selected rows, making it suitable for monitoring.

Text and CSV rows are written and flushed as each S3 query completes. JSON and YAML remain buffered until every forum has completed so they are emitted as one valid, stable document. Long backup filenames are shortened in the middle in text tables only; structured output always retains the complete key.

```bash
# Check every configured forum
dsc backup health

# Allow up to fourteen days without overriding a more relaxed site schedule
dsc backup health myforum --max-age 14

# Export a production fleet report for automation
dsc backup health --tags production --format json
```

Requires the [`aws` CLI](https://docs.aws.amazon.com/cli/). Bucket, region, optional `s3_endpoint`, backup frequency, and S3 credentials are read from each Discourse's admin site settings. Static credentials are passed only to the child process environment and are never printed or stored by `dsc`; forums using `s3_use_iam_profile` retain the normal ambient AWS credential chain. Custom endpoints such as DigitalOcean Spaces are passed to `aws s3api` with `--endpoint-url`. This command is read-only: it never creates backups, changes settings, or deletes objects.

## dsc backup create

```
dsc backup create <discourse>
dsc backup create --all
dsc backup create --tags <tag1,tag2,...>
```

Triggers a backup on the specified Discourse. The backup is created server-side; it is not downloaded locally.

Use `--all` to trigger a backup on every configured forum, or `--tags` to select a matching subset. Tags are comma- or semicolon-separated and match any configured tag, case-insensitively. An empty tag filter is rejected rather than running across the entire fleet.

```bash
dsc backup create --all
dsc backup create --tags production
```

Continues past a forum that fails (missing credentials, unreachable) so one bad entry doesn't stop the rest of the fleet; exits non-zero if any forum failed.

## dsc backup list

```
dsc backup list <discourse> [--format text|markdown|markdown-table|json|yaml|csv|urls] [--verbose]
```

Lists all backups on the specified Discourse. Supports the same formats as `dsc list`. `-v`/`--verbose` includes additional fields where supported.

## dsc backup pull

```text
dsc backup pull <discourse> <backup-filename> [<local-path>]
```

Downloads a backup archive to the local filesystem. `<backup-filename>` is the name shown by `dsc backup list`. If `<local-path>` is omitted, the file is saved in the current directory with the same name.

```bash
dsc backup pull myforum discourse-2026-04-17-230000.tar.gz
dsc backup pull myforum discourse-2026-04-17-230000.tar.gz ./backups/
```

## dsc backup push

```text
dsc backup push <discourse> <backup-path>
```

Restores the specified backup (alias: `dsc backup restore`). `<backup-path>` is the backup filename as shown by `dsc backup list`.

Restoration is destructive and irreversible. Use `--dry-run` (or `-n`) to preview the operation before committing:

```bash
dsc --dry-run backup push myforum discourse-2026-04-17-230000.tar.gz
```

## dsc backup setup-s3

```text
dsc backup setup-s3 <discourse> [--region <r>] [--bucket <name>] [--no-test] [--use-iam-profile]
dsc backup setup-s3 --all|--tags <tag1,tag2,...> [--region <r>] [--no-test] [--use-iam-profile]
```

Provisions off-site backups on Amazon S3 in one command, replacing the per-forum AWS-console runbook: it creates a private bucket, a dedicated **single-bucket** IAM user + least-privilege policy, mints an access key, and points Discourse's S3 backup settings at it - then (unless `--no-test`) triggers a backup and confirms it lands in the bucket.

Defaults derive from the forum's config name:

- bucket `<name>-discourse-backups`, policy `s3-single-bucket-<name>-discourse-backups`, user `<name>-discourse-backup-user`
- region `eu-west-2` (override with `--region`); bucket override with `--bucket`

With `--use-iam-profile` (for forums running on an EC2 instance role that already has bucket access), only the bucket is created - no IAM policy, user, or access key - and Discourse is pointed at S3 with `s3_use_iam_profile=true` instead of static credentials.

Use `--all` to provision every configured forum, or `--tags` to provision a fleet subset (mirrors `dsc backup create --all` and `dsc backup health --tags`). An empty `--tags` value is rejected rather than interpreted as the whole fleet. Each forum still derives its own bucket/policy/user names, so `--bucket` cannot be combined with `--all`/`--tags`. Fan-out continues past a per-forum failure so one bad entry doesn't stop the rest of the fleet, and exits non-zero if any forum could not be provisioned.

**Requirements & safety:**

- The [`aws` CLI](https://docs.aws.amazon.com/cli/) must be installed and configured with a profile that has IAM + S3 admin rights. Those provisioning credentials are used only by `aws` and are **never stored by `dsc`**. The minted least-privilege key is written straight into the Discourse setting (not into `dsc.toml`) and is **never printed**.
- This creates real AWS resources and writes production settings. **Always preview with `-n` / `--dry-run` first** - it prints the resolved names, the full IAM policy JSON, the exact `aws` commands, and the settings diff, and touches nothing.

```bash
# 1) Review the complete plan (creates nothing)
dsc backup setup-s3 -n myforum

# 2) Provision for real (eu-west-2 by default)
dsc backup setup-s3 myforum --region eu-west-1

# 3) On an EC2 instance role that already has bucket access - no static keys
dsc backup setup-s3 myforum --use-iam-profile

# 4) Preview across the whole fleet before provisioning for real
dsc backup setup-s3 -n --all
dsc backup setup-s3 --tags production --use-iam-profile
```

> Phase 1 covers the create-everything flow; `--use-iam-profile` and `--all`/`--tags` are implemented on `main`. `--reuse-user` (idempotent re-runs / key rotation) is planned - see the [backup S3 setup spec](https://github.com/koloki-co/dsc/blob/main/spec/commands/backup-s3-setup.md).
