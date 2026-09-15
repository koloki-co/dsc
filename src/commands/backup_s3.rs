// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::commands::backup::{
    AWS_CALL_TIMEOUT, MAX_S3_PAGE_BYTES, MAX_S3_PAGES, is_backup_archive, list_s3_args,
    run_aws_json,
};
use crate::commands::common::{ensure_api_credentials, select_discourse, selected_discourses};
use crate::config::Config;
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, SubsecRound, Utc};
use serde_json::{Value, json};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// AWS resource names derived for a forum's S3 backup setup.
struct Names {
    bucket: String,
    policy: String,
    user: String,
}

/// Derive the bucket / policy / user names. The bucket is `<forum>-discourse-backups`
/// unless overridden; the policy tracks the bucket name; the user is forum-derived.
fn derive_names(forum: &str, bucket_override: Option<&str>) -> Names {
    let bucket = bucket_override
        .map(str::to_string)
        .unwrap_or_else(|| format!("{forum}-discourse-backups"));
    Names {
        policy: format!("s3-single-bucket-{bucket}"),
        user: format!("{forum}-discourse-backup-user"),
        bucket,
    }
}

/// The single-bucket, least-privilege IAM policy: list on the bucket, object
/// actions confined to its contents.
fn single_bucket_policy(bucket: &str) -> Value {
    json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Effect": "Allow",
                "Action": "s3:ListBucket",
                "Resource": format!("arn:aws:s3:::{bucket}")
            },
            {
                "Effect": "Allow",
                "Action": "s3:*",
                "Resource": [format!("arn:aws:s3:::{bucket}/*")]
            }
        ]
    })
}

/// Args for `aws s3api create-bucket`. `us-east-1` must NOT carry a
/// `LocationConstraint` (S3 rejects it there); every other region must.
fn create_bucket_args(bucket: &str, region: &str) -> Vec<String> {
    let mut args = vec![
        "s3api".into(),
        "create-bucket".into(),
        "--bucket".into(),
        bucket.into(),
        "--region".into(),
        region.into(),
    ];
    if region != "us-east-1" {
        args.push("--create-bucket-configuration".into());
        args.push(format!("LocationConstraint={region}"));
    }
    args
}

const PUBLIC_ACCESS_BLOCK: &str =
    "BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true";

/// Run `aws <args>` (JSON output) and return parsed stdout. Errors carry stderr.
fn aws_json(args: &[String]) -> Result<Value> {
    let output = Command::new("aws")
        .args(args)
        .args(["--output", "json"])
        .output()
        .context("running `aws` - is the AWS CLI installed and on PATH?")?;
    if !output.status.success() {
        bail!(
            "aws {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&stdout)
        .with_context(|| format!("parsing `aws {}` output", args.join(" ")))
}

/// Run `aws <args>` ignoring stdout (for commands that return nothing useful).
fn aws_run(args: &[String]) -> Result<()> {
    let output = Command::new("aws")
        .args(args)
        .output()
        .context("running `aws` - is the AWS CLI installed and on PATH?")?;
    if !output.status.success() {
        bail!(
            "aws {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Rotate the access key for an existing `--reuse-user` IAM user: mint a new
/// key, point Discourse at it, then deactivate the key(s) that existed
/// before rotation. AWS caps a user at two access keys (active or
/// inactive) regardless of status, so a user already at that cap is
/// refused up front rather than left with a mix of live and orphaned keys.
fn rotate_access_key(client: &DiscourseClient, names: &Names) -> Result<()> {
    let existing = aws_json(&[
        "iam".into(),
        "list-access-keys".into(),
        "--user-name".into(),
        names.user.clone(),
    ])?;
    let old_key_ids: Vec<String> = existing
        .get("AccessKeyMetadata")
        .and_then(|v| v.as_array())
        .map(|keys| {
            keys.iter()
                .filter_map(|k| k.get("AccessKeyId").and_then(|v| v.as_str()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    ensure_access_key_capacity(&names.user, old_key_ids.len())?;

    let key = aws_json(&[
        "iam".into(),
        "create-access-key".into(),
        "--user-name".into(),
        names.user.clone(),
    ])?;
    let access_key_id = key
        .get("AccessKey")
        .and_then(|k| k.get("AccessKeyId"))
        .and_then(|v| v.as_str())
        .context("create-access-key did not return an AccessKeyId")?
        .to_string();
    let secret_access_key = key
        .get("AccessKey")
        .and_then(|k| k.get("SecretAccessKey"))
        .and_then(|v| v.as_str())
        .context("create-access-key did not return a SecretAccessKey")?
        .to_string();
    println!(
        "  minted new access key {} for {}",
        access_key_id, names.user
    );

    // Point Discourse at the new key before deactivating the old one, so
    // there is no window where neither key works.
    client.update_site_setting("s3_access_key_id", &access_key_id)?;
    client.update_site_setting("s3_secret_access_key", &secret_access_key)?;
    // A prior --use-iam-profile run leaves this enabled.
    client.update_site_setting("s3_use_iam_profile", "false")?;

    for old_id in old_key_ids {
        aws_run(&[
            "iam".into(),
            "update-access-key".into(),
            "--user-name".into(),
            names.user.clone(),
            "--access-key-id".into(),
            old_id.clone(),
            "--status".into(),
            "Inactive".into(),
        ])?;
        println!("  deactivated old access key {old_id}");
    }
    Ok(())
}

/// AWS counts inactive access keys against the same two-key cap. Rotation can
/// proceed only after an existing key has been deleted, not merely disabled.
fn ensure_access_key_capacity(user: &str, key_count: usize) -> Result<()> {
    if key_count >= 2 {
        bail!(
            "{user} already has {key_count} access keys (AWS's two-key limit, including inactive keys) - \
             delete an existing key before retrying --reuse-user"
        );
    }
    Ok(())
}

/// One-command S3 backup provisioning (spec/backup-s3-setup.md, Phase 1):
/// create a private bucket + single-bucket IAM user/policy, point Discourse at
/// it, and (unless `--no-test`) trigger a backup and confirm it lands.
#[allow(clippy::too_many_arguments)]
pub fn setup_s3(
    config: &Config,
    discourse_name: &str,
    region: &str,
    bucket: Option<&str>,
    no_test: bool,
    use_iam_profile: bool,
    reuse_user: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let names = derive_names(&discourse.name, bucket);
    let policy_doc = single_bucket_policy(&names.bucket);
    let policy_json = serde_json::to_string(&policy_doc)?;
    let policy_pretty = serde_json::to_string_pretty(&policy_doc)?;

    if dry_run {
        print_plan(
            &discourse.name,
            &names,
            region,
            &policy_pretty,
            no_test,
            use_iam_profile,
            reuse_user,
        );
        return Ok(());
    }

    // Pre-flight: aws usable, identity known, forum reachable.
    let identity = aws_json(&["sts".into(), "get-caller-identity".into()])
        .context("AWS pre-flight failed (need credentials with IAM + S3 admin rights)")?;
    let account = identity
        .get("Account")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)");
    let client = DiscourseClient::new(discourse)?;
    client
        .fetch_version_info()
        .context("forum pre-flight failed: could not reach the Discourse admin API")?;
    println!(
        "Provisioning S3 backups for {} in AWS account {} (region {})",
        discourse.name, account, region
    );

    // 1. Private bucket + block-public-access. Skipped with --reuse-user,
    // which targets a bucket/user already provisioned by an earlier run.
    if reuse_user {
        println!(
            "  reusing existing bucket {} and user {} (--reuse-user)",
            names.bucket, names.user
        );
    } else {
        aws_run(&create_bucket_args(&names.bucket, region))?;
        aws_run(&[
            "s3api".into(),
            "put-public-access-block".into(),
            "--bucket".into(),
            names.bucket.clone(),
            "--public-access-block-configuration".into(),
            PUBLIC_ACCESS_BLOCK.into(),
        ])?;
        println!("  created bucket {} (public access blocked)", names.bucket);
    }

    // 2./3. With --use-iam-profile the EC2 instance role already carries the
    // bucket permissions (provisioned outside dsc), so no dedicated IAM user
    // or policy is minted here - only the bucket exists to be granted access to.
    if use_iam_profile {
        println!(
            "  skipped IAM user/policy creation (--use-iam-profile); \
             ensure the instance role can access s3://{}",
            names.bucket
        );
    } else if reuse_user {
        rotate_access_key(&client, &names)?;
    } else {
        // Single-bucket managed policy -> ARN.
        let policy = aws_json(&[
            "iam".into(),
            "create-policy".into(),
            "--policy-name".into(),
            names.policy.clone(),
            "--policy-document".into(),
            policy_json,
        ])?;
        let policy_arn = policy
            .get("Policy")
            .and_then(|p| p.get("Arn"))
            .and_then(|v| v.as_str())
            .context("create-policy did not return a Policy ARN")?
            .to_string();
        println!("  created policy {}", names.policy);

        // Dedicated user + attach + access key.
        aws_run(&[
            "iam".into(),
            "create-user".into(),
            "--user-name".into(),
            names.user.clone(),
        ])?;
        aws_run(&[
            "iam".into(),
            "attach-user-policy".into(),
            "--user-name".into(),
            names.user.clone(),
            "--policy-arn".into(),
            policy_arn,
        ])?;
        let key = aws_json(&[
            "iam".into(),
            "create-access-key".into(),
            "--user-name".into(),
            names.user.clone(),
        ])?;
        let access_key_id = key
            .get("AccessKey")
            .and_then(|k| k.get("AccessKeyId"))
            .and_then(|v| v.as_str())
            .context("create-access-key did not return an AccessKeyId")?
            .to_string();
        let secret_access_key = key
            .get("AccessKey")
            .and_then(|k| k.get("SecretAccessKey"))
            .and_then(|v| v.as_str())
            .context("create-access-key did not return a SecretAccessKey")?
            .to_string();
        println!(
            "  created user {} with access key {}",
            names.user, access_key_id
        );

        // Point Discourse at the bucket with the minted static credentials
        // (the secret goes straight into the setting, never into dsc.toml and
        // never printed).
        client.update_site_setting("s3_access_key_id", &access_key_id)?;
        client.update_site_setting("s3_secret_access_key", &secret_access_key)?;
        // A prior --use-iam-profile run leaves this enabled. Disable it only
        // after the replacement static credentials are stored.
        client.update_site_setting("s3_use_iam_profile", "false")?;
    }

    // 4. Point Discourse at the bucket.
    // "Enable last": Discourse validates `backup_location=s3` against the S3
    // settings being present, so set the bucket/region/credentials FIRST and
    // flip `backup_location` to s3 only once they're in place. Doing it the
    // other way round can 422 - leaving AWS provisioned but Discourse
    // half-configured. (Same pattern as enabling reply-by-email.)
    client.update_site_setting("s3_backup_bucket", &names.bucket)?;
    client.update_site_setting("s3_region", region)?;
    if use_iam_profile {
        client.update_site_setting("s3_use_iam_profile", "true")?;
    }
    client.update_site_setting("backup_location", "s3")?;
    if use_iam_profile {
        println!("  set Discourse S3 backup settings (s3_use_iam_profile, no static keys)");
    } else {
        println!("  set Discourse S3 backup settings (secret written to the setting, not stored)");
    }

    // 5. Optional verification backup.
    if no_test {
        println!(
            "Done. Skipped the test backup (--no-test); run `dsc backup create {}` to verify.",
            discourse.name
        );
        return Ok(());
    }
    println!("Triggering a test backup and waiting for it to land in the bucket...");
    client.create_backup()?;
    // Record the trigger time so the verification poll only accepts an
    // archive newer than this moment as proof the new backup landed (P14);
    // a bucket holding pre-existing archives (the `--reuse-user` case) must
    // not otherwise be mistaken for success. Truncating to whole seconds
    // keeps the comparison robust against sub-second clock skew between
    // Discourse's clock and the S3 upload stamping.
    let triggered_at = Utc::now().trunc_subsecs(0);
    if wait_for_backup_object(&names.bucket, region, triggered_at)? {
        println!(
            "✓ Test backup landed in s3://{}/ - setup verified.",
            names.bucket
        );
    } else {
        println!(
            "Backup triggered, but nothing newer than {triggered_at} appeared in \
             s3://{}/ within the poll window. Discourse backups run asynchronously - \
             re-check with `aws s3 ls s3://{}/` shortly.",
            names.bucket, names.bucket
        );
    }
    Ok(())
}

/// Fan out `backup setup-s3` to every configured forum, optionally filtered
/// by `--tags`. Continues past a per-forum failure (AWS provisioning error,
/// unreachable forum, bucket name already taken) so one bad entry doesn't
/// stop the rest of the fleet; fails at the end if any forum could not be
/// provisioned. Each forum derives its own bucket/policy/user names, so a
/// `--bucket` override is not accepted here.
pub fn setup_s3_all(
    config: &Config,
    tags: Option<&str>,
    region: &str,
    no_test: bool,
    use_iam_profile: bool,
    reuse_user: bool,
    dry_run: bool,
) -> Result<()> {
    let discourses = selected_discourses(config, None, tags)?;
    if discourses.is_empty() {
        return Err(if tags.is_some() {
            anyhow!("no discourses configured matching the given tags")
        } else {
            anyhow!("no discourses configured")
        });
    }

    let mut failed = 0usize;
    for (index, discourse) in discourses.iter().enumerate() {
        if index > 0 {
            println!();
        }
        match setup_s3(
            config,
            &discourse.name,
            region,
            None,
            no_test,
            use_iam_profile,
            reuse_user,
            dry_run,
        ) {
            Ok(()) => {}
            Err(e) => {
                failed += 1;
                eprintln!("{}: setup-s3 failed - {e}", discourse.name);
            }
        }
    }

    if failed > 0 {
        return Err(anyhow!(
            "setup-s3 failed on {failed} of {} forum(s)",
            discourses.len()
        ));
    }
    Ok(())
}

/// Poll for a backup object for up to ~3 minutes (P14). Previously ran
/// `aws s3 ls --recursive`, which lists and buffers the entire bucket on
/// every ten-second attempt via unbounded `Command::output()` - the
/// wall-clock deadline was only checked between whole-bucket listings, so
/// one hung `aws` process could exceed it indefinitely, and nothing was
/// printed while waiting. Now each attempt walks bounded `list-objects-v2`
/// pages whose total elapsed time cannot exceed the poll budget, and stops
/// at the first archive *newer than the trigger time* (P14's
/// recommendation): pre-existing archives are skipped so a bucket already
/// holding older backups (the common `--reuse-user` re-run case) cannot be
/// mistaken for proof the newly triggered one landed.
///
/// The bound applies across the whole poll, not per poll attempt: the
/// budget handed to each page walk is the outer deadline minus elapsed
/// time, so retries cannot extend total runtime, and a page's own
/// subprocess call is separately capped by `run_aws_json`'s timeout. The
/// sleep is clamped to the remaining budget so the loop cannot oversleep
/// past the deadline, and a final elapsed check prevents a deadline-adjacent
/// sleep loop from starting one more attempt.
fn wait_for_backup_object(bucket: &str, region: &str, triggered_at: DateTime<Utc>) -> Result<bool> {
    const POLL_INTERVAL: Duration = Duration::from_secs(10);
    let deadline = Duration::from_secs(180);
    let start = Instant::now();
    let mut attempt = 0u32;
    // A transient failure (throttling, IAM propagation delay) is not fatal
    // to the poll - retry within the outer deadline, matching the previous
    // implementation's tolerance of a non-zero `aws` exit. If every attempt
    // fails, the last error is kept so the final message can say why
    // instead of a bare "nothing visible".
    let mut last_error: Option<anyhow::Error> = None;
    while start.elapsed() < deadline {
        attempt += 1;
        println!(
            "  checking s3://{bucket}/ for the new backup (attempt {attempt}, {}s elapsed)...",
            start.elapsed().as_secs()
        );
        let remaining = deadline.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            break;
        }
        match s3_newest_backup_after(bucket, region, triggered_at, remaining) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(error) => last_error = Some(error),
        }
        let sleep_for = POLL_INTERVAL.min(deadline.saturating_sub(start.elapsed()));
        if sleep_for.is_zero() {
            break;
        }
        sleep(sleep_for);
    }
    if let Some(error) = last_error {
        bail!(
            "backup did not appear in s3://{bucket}/ within the poll window; \
             last listing error: {error:#}"
        );
    }
    Ok(false)
}

/// Walk `list-objects-v2` pages for `bucket`, bounded by `budget`, returning
/// as soon as a backup archive with `LastModified` strictly after
/// `triggered_at` is seen (P14). Each page is its own byte-capped, timed
/// `aws` call via `run_aws_json`, sharing `backup`'s argument construction,
/// page parsing, and page-cap constant rather than duplicating them.
/// `MAX_S3_PAGES` (1,000) guards against a pathological continuation loop
/// the way `backup`'s full-inventory scan does; here it is also the
/// practical upper bound on objects seen, since the walk stops at the
/// first page containing a new archive. Subprocess/parse failures surface
/// as errors the poll tolerates as transient; a missing or non-boolean
/// `IsTruncated`, a truncated page without a token, or a malformed
/// `LastModified` on an archive key each end the walk as "no match seen"
/// or an error, but the poll's final message always preserves the last
/// error if every attempt failed.
fn s3_newest_backup_after(
    bucket: &str,
    region: &str,
    triggered_at: DateTime<Utc>,
    budget: Duration,
) -> Result<bool> {
    let start = Instant::now();
    let mut token: Option<String> = None;
    for _ in 0..MAX_S3_PAGES {
        let remaining = budget.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Ok(false);
        }
        let args = list_s3_args(bucket, region, None, token.as_deref());
        let page = run_aws_json(
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            None,
            None,
            MAX_S3_PAGE_BYTES,
            remaining.min(AWS_CALL_TIMEOUT),
        )?;
        if page_newest_backup_after(&page, triggered_at)? {
            return Ok(true);
        }
        if !page
            .get("IsTruncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(false);
        }
        token = page
            .get("NextContinuationToken")
            .and_then(Value::as_str)
            .map(str::to_string);
        if token.is_none() {
            return Ok(false);
        }
    }
    Ok(false)
}

/// True if a `list-objects-v2` JSON page contains a backup archive with a
/// `LastModified` strictly after `triggered_at` (P14). Keys are matched on
/// the basename, exactly as `backup health` classifies archives, so a
/// `backups/default/` prefix and non-archive objects (reports, manifests)
/// are ignored. Missing/malformed `LastModified` on an archive key is an
/// error rather than a silently skipped object.
fn page_newest_backup_after(page: &Value, triggered_at: DateTime<Utc>) -> Result<bool> {
    let Some(contents) = page.get("Contents").and_then(Value::as_array) else {
        return Ok(false);
    };
    for object in contents {
        let Some(key) = object.get("Key").and_then(Value::as_str) else {
            continue;
        };
        if !is_backup_archive(key) {
            continue;
        }
        let modified_at = object
            .get("LastModified")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("S3 object '{key}' missing LastModified"))?
            .parse::<DateTime<Utc>>()
            .with_context(|| format!("parsing S3 LastModified for {key}"))?;
        if modified_at > triggered_at {
            return Ok(true);
        }
    }
    Ok(false)
}

fn print_plan(
    forum: &str,
    names: &Names,
    region: &str,
    policy_pretty: &str,
    no_test: bool,
    use_iam_profile: bool,
    reuse_user: bool,
) {
    println!("[dry-run] S3 backup setup for {forum} (region {region})\n");
    if reuse_user {
        println!("AWS resources to reuse (--reuse-user - no bucket/policy/user created):");
        println!("  bucket  {}", names.bucket);
        println!(
            "  user    {}   (+ one freshly minted access key)\n",
            names.user
        );
    } else {
        println!("AWS resources to create:");
        println!(
            "  bucket  {}   (private; Block Public Access on; AWS-default SSE-S3)",
            names.bucket
        );
        if use_iam_profile {
            println!(
                "  (--use-iam-profile: no IAM policy/user/access-key created; \
                 the instance role must already have access to this bucket)\n"
            );
        } else {
            println!(
                "  policy  {}   (single-bucket, least privilege)",
                names.policy
            );
            println!("  user    {}   (+ one access key)\n", names.user);

            println!("IAM policy document:");
            for line in policy_pretty.lines() {
                println!("  {line}");
            }
            println!();
        }
    }

    println!("aws commands:");
    if reuse_user {
        println!("  aws iam list-access-keys --user-name {}", names.user);
        println!("  aws iam create-access-key --user-name {}", names.user);
        println!(
            "  aws iam update-access-key --user-name {} --access-key-id <old> --status Inactive",
            names.user
        );
    } else {
        println!(
            "  aws {}",
            create_bucket_args(&names.bucket, region).join(" ")
        );
        println!(
            "  aws s3api put-public-access-block --bucket {} --public-access-block-configuration {}",
            names.bucket, PUBLIC_ACCESS_BLOCK
        );
        if !use_iam_profile {
            println!(
                "  aws iam create-policy --policy-name {} --policy-document <json above>",
                names.policy
            );
            println!("  aws iam create-user --user-name {}", names.user);
            println!(
                "  aws iam attach-user-policy --user-name {} --policy-arn <policy ARN>",
                names.user
            );
            println!("  aws iam create-access-key --user-name {}", names.user);
        }
    }
    println!();

    println!("Discourse settings to set (in this order):");
    println!("  s3_backup_bucket     = {}", names.bucket);
    println!("  s3_region            = {region}");
    if use_iam_profile {
        println!("  s3_use_iam_profile   = true");
    } else {
        println!("  s3_use_iam_profile   = false");
        println!("  s3_access_key_id     = <minted at run time>");
        println!("  s3_secret_access_key = <minted at run time; never printed>");
    }
    println!("  backup_location      = s3   (enabled LAST, once the above are set)\n");

    if no_test {
        println!("Test backup: skipped (--no-test).");
    } else {
        println!(
            "Then: dsc backup create {forum}, and confirm the dump appears via \
             aws s3 ls s3://{}/ (skip with --no-test).",
            names.bucket
        );
    }
    println!("\nNothing was created or changed (--dry-run).");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_runbook_scheme() {
        let n = derive_names("myforum", None);
        assert_eq!(n.bucket, "myforum-discourse-backups");
        assert_eq!(n.policy, "s3-single-bucket-myforum-discourse-backups");
        assert_eq!(n.user, "myforum-discourse-backup-user");
    }

    #[test]
    fn bucket_override_keeps_user_forum_derived() {
        let n = derive_names("myforum", Some("custom-bucket"));
        assert_eq!(n.bucket, "custom-bucket");
        assert_eq!(n.policy, "s3-single-bucket-custom-bucket");
        assert_eq!(n.user, "myforum-discourse-backup-user");
    }

    #[test]
    fn policy_is_confined_to_the_one_bucket() {
        let p = single_bucket_policy("b");
        let stmts = p["Statement"].as_array().unwrap();
        assert_eq!(stmts[0]["Action"], "s3:ListBucket");
        assert_eq!(stmts[0]["Resource"], "arn:aws:s3:::b");
        assert_eq!(stmts[1]["Resource"][0], "arn:aws:s3:::b/*");
    }

    #[test]
    fn create_bucket_omits_location_constraint_for_us_east_1() {
        let args = create_bucket_args("b", "us-east-1");
        assert!(!args.iter().any(|a| a.contains("LocationConstraint")));
    }

    #[test]
    fn create_bucket_sets_location_constraint_elsewhere() {
        let args = create_bucket_args("b", "eu-west-2");
        assert!(args.contains(&"LocationConstraint=eu-west-2".to_string()));
    }

    #[test]
    fn key_rotation_requires_deletion_when_at_aws_key_cap() {
        let error = ensure_access_key_capacity("forum-discourse-backup-user", 2).unwrap_err();
        assert!(error.to_string().contains("including inactive keys"));
        assert!(error.to_string().contains("delete an existing key"));
    }

    #[test]
    fn page_newest_backup_after_matches_new_archives_on_the_basename() {
        let before = "2026-09-15T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        // A pre-existing archive (at or before the trigger time) is never a
        // match; the basename is checked so non-archive keys are ignored.
        let page = json!({
            "IsTruncated": false,
            "Contents": [
                {"Key": "backups/default/old.tar.gz", "LastModified": "2026-09-15T00:00:00Z", "Size": 1},
                {"Key": "backups/default/older.tar", "LastModified": "2026-09-14T00:00:00Z", "Size": 1},
                {"Key": "reports/checksums.sha256", "LastModified": "2026-09-16T00:00:00Z", "Size": 1},
            ],
        });
        assert!(!page_newest_backup_after(&page, before).unwrap());

        // Strictly-after matches a second later; the trigger time itself
        // does not.
        let page = json!({
            "IsTruncated": false,
            "Contents": [{"Key": "backups/default/new.tar.gz", "LastModified": "2026-09-15T00:00:01Z", "Size": 1}],
        });
        assert!(page_newest_backup_after(&page, before).unwrap());

        // A future-dated non-archive key does not match, and a
        // timezone-offset timestamp parses to the same instant as its Z form
        // (02:00+02:00 is 00:00:01Z's neighbour: 02:00:01+02:00 = 00:00:01Z,
        // strictly after the trigger).
        let page = json!({
            "IsTruncated": false,
            "Contents": [{"Key": "backups/default/offset.tar.gz", "LastModified": "2026-09-15T02:00:01+02:00", "Size": 1}],
        });
        assert!(page_newest_backup_after(&page, before).unwrap());
        let page = json!({
            "IsTruncated": false,
            "Contents": [{"Key": "backups/default/equal-offset.tar.gz", "LastModified": "2026-09-15T02:00:00+02:00", "Size": 1}],
        });
        assert!(!page_newest_backup_after(&page, before).unwrap());
    }

    #[test]
    fn page_newest_backup_after_handles_an_empty_page() {
        let before = "2026-09-15T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let page = json!({"IsTruncated": false, "Contents": []});
        assert!(!page_newest_backup_after(&page, before).unwrap());
        assert!(!page_newest_backup_after(&json!({}), before).unwrap());
        // A non-array Contents is treated as no contents, not an error, so
        // an empty bucket page cannot fail the poll.
        assert!(!page_newest_backup_after(&json!({"Contents": null}), before).unwrap());
    }

    #[test]
    fn page_newest_backup_after_rejects_an_archive_with_malformed_timestamp() {
        let before = "2026-09-15T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let page = json!({
            "IsTruncated": false,
            "Contents": [
                {"Key": "backups/default/other.tar.gz", "LastModified": "not-a-time", "Size": 1},
            ],
        });
        let error = page_newest_backup_after(&page, before).unwrap_err();
        assert!(error.to_string().contains("LastModified"), "{error}");

        let page = json!({
            "IsTruncated": false,
            "Contents": [{"Key": "backups/default/no-time.tar.gz", "Size": 1}],
        });
        let error = page_newest_backup_after(&page, before).unwrap_err();
        assert!(
            error.to_string().contains("missing LastModified"),
            "{error}"
        );
    }

    // Real-subprocess coverage for `s3_newest_backup_after`'s use of
    // `backup::run_aws_json` (see `tests/fixtures/fake-aws`'s header
    // comment): when given a real `--bucket <name>`, the fixture echoes it
    // into a single non-truncated page's object key. The fixture's fixed
    // `2026-01-01` LastModified only matches a trigger time before it.
    #[test]
    #[cfg(unix)]
    fn s3_newest_backup_after_finds_an_archive_newer_than_the_trigger() {
        use crate::commands::ssh::fixture_tests::FakeAwsPath;
        let _fake_aws = FakeAwsPath::install();
        let before = "2025-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let found =
            s3_newest_backup_after("some-bucket", "eu-west-2", before, Duration::from_secs(5))
                .expect("fake-aws succeeds");
        assert!(
            found,
            "fake-aws echoes a 2026 archive for a real bucket name"
        );
    }

    #[test]
    #[cfg(unix)]
    fn s3_newest_backup_after_skips_archives_older_than_the_trigger() {
        use crate::commands::ssh::fixture_tests::FakeAwsPath;
        let _fake_aws = FakeAwsPath::install();
        // The fixture's single archive is dated 2026-01-01, so a later
        // trigger time means the bucket holds only pre-existing archives -
        // exactly the false-positive the trigger-time check exists to catch.
        let after = "2027-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let found = s3_newest_backup_after("", "eu-west-2", after, Duration::from_secs(5))
            .expect("fake-aws succeeds");
        assert!(!found);
    }
}
