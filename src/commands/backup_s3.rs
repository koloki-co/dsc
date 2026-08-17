// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::commands::backup::selected_discourses;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::{Context, Result, anyhow, bail};
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

/// One-command S3 backup provisioning (spec/backup-s3-setup.md, Phase 1):
/// create a private bucket + single-bucket IAM user/policy, point Discourse at
/// it, and (unless `--no-test`) trigger a backup and confirm it lands.
pub fn setup_s3(
    config: &Config,
    discourse_name: &str,
    region: &str,
    bucket: Option<&str>,
    no_test: bool,
    use_iam_profile: bool,
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

    // 1. Private bucket + block-public-access.
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

    // 2./3. With --use-iam-profile the EC2 instance role already carries the
    // bucket permissions (provisioned outside dsc), so no dedicated IAM user
    // or policy is minted here - only the bucket exists to be granted access to.
    if use_iam_profile {
        println!(
            "  skipped IAM user/policy creation (--use-iam-profile); \
             ensure the instance role can access s3://{}",
            names.bucket
        );
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
    if wait_for_backup_object(&names.bucket)? {
        println!(
            "✓ Test backup landed in s3://{}/ - setup verified.",
            names.bucket
        );
    } else {
        println!(
            "Backup triggered, but nothing visible in s3://{}/ yet. Discourse backups run \
             asynchronously - re-check with `aws s3 ls s3://{}/ --recursive` shortly.",
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
    dry_run: bool,
) -> Result<()> {
    let discourses = selected_discourses(config, None, tags)?;
    if discourses.is_empty() {
        return Err(anyhow!("no discourses configured matching the given tags"));
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

/// Poll `aws s3 ls` for a backup object (`.tar.gz`) for up to ~3 minutes.
fn wait_for_backup_object(bucket: &str) -> Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        let output = Command::new("aws")
            .args(["s3", "ls", &format!("s3://{bucket}/"), "--recursive"])
            .output()
            .context("running `aws s3 ls`")?;
        if output.status.success() && String::from_utf8_lossy(&output.stdout).contains(".tar.gz") {
            return Ok(true);
        }
        sleep(Duration::from_secs(10));
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
) {
    println!("[dry-run] S3 backup setup for {forum} (region {region})\n");
    println!("AWS resources to create:");
    println!(
        "  bucket  {}   (private; Block Public Access on; SSE-S3)",
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

    println!("aws commands:");
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
}
