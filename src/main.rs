// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Result, anyhow};
use clap::{CommandFactory, Parser};
use dsc::cli::*;
use dsc::commands;
use dsc::commands::analytics::SectionFilter;
use dsc::commands::user::{ActivityFormat, Role};

fn map_section(s: SectionArg) -> SectionFilter {
    match s {
        SectionArg::All => SectionFilter::All,
        SectionArg::Growth => SectionFilter::Growth,
        SectionArg::Activity => SectionFilter::Activity,
        SectionArg::Health => SectionFilter::Health,
    }
}
use dsc::config::{
    ConfigSource, ENV_CONFIG, ENV_CONFIG_HOME, config_search_paths, load_config,
    resolve_config_source, save_config,
};

fn map_role(role: RoleArg) -> Role {
    match role {
        RoleArg::Admin => Role::Admin,
        RoleArg::Moderator => Role::Moderator,
    }
}

fn map_activity_format(f: ActivityFormatArg) -> ActivityFormat {
    match f {
        ActivityFormatArg::Text => ActivityFormat::Text,
        ActivityFormatArg::Json => ActivityFormat::Json,
        ActivityFormatArg::Yaml => ActivityFormat::Yaml,
        ActivityFormatArg::Markdown => ActivityFormat::Markdown,
        ActivityFormatArg::Csv => ActivityFormat::Csv,
    }
}

/// Map the CLI's symbolic content type to Discourse's integer encoding.
fn map_webhook_content_type(content_type: WebhookContentTypeArg) -> u8 {
    match content_type {
        WebhookContentTypeArg::Json => 1,
        WebhookContentTypeArg::Form => 2,
    }
}

/// Dispatch a palette subcommand. Shared by the top-level `dsc palette`
/// (deprecated) and the canonical `dsc theme palette`.
fn run_palette(config: &dsc::config::Config, command: PaletteCommand) -> Result<()> {
    match command {
        PaletteCommand::List {
            discourse,
            format,
            verbose,
        } => commands::palette::palette_list(config, &discourse, format, verbose),
        PaletteCommand::Pull {
            discourse,
            palette_id,
            local_path,
            force,
        } => commands::palette::palette_pull(
            config,
            &discourse,
            palette_id,
            local_path.as_deref(),
            force,
        ),
        PaletteCommand::Push {
            discourse,
            local_path,
            palette_id,
        } => commands::palette::palette_push(config, &discourse, &local_path, palette_id),
    }
}

/// Restore the default SIGPIPE disposition on Unix so piping `dsc` into
/// `head`, `less`, or `diff <(dsc …) …` terminates cleanly instead of
/// panicking from `println!` on a closed stdout. Rust's runtime ignores
/// SIGPIPE by default, which turns every broken-pipe write into a panic -
/// fine for a long-lived service, wrong for a CLI.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: single FFI call with well-defined semantics, called once at startup.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() -> Result<()> {
    reset_sigpipe();
    // Any obvious attempt to get the version should be helpful. clap wires
    // `-V` / `--version` (via `#[command(version)]`); also treat a lone
    // lowercase `-v` / `-version` / `--v` as a version request rather than
    // letting it fall through to an "unexpected argument" error.
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    }
    if args.len() == 2 && matches!(args[1].as_str(), "-v" | "-version" | "--v") {
        println!("dsc {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let cli = Cli::parse();
    if cli.dry_run
        && let Some(command) = cli.command.dry_run_refusal_reason()
    {
        return Err(anyhow!(
            "[dry-run] {command}: a complete dry-run plan is not implemented; refusing before configuration or side effects.\nhint: remove --dry-run only after reviewing this command's documentation and inputs"
        ));
    }

    if let Commands::Version {
        discourse: None,
        format,
    } = &cli.command
    {
        return commands::version::own_version(*format);
    }

    let config_source = resolve_config_source(cli.config)?;
    let config_path = config_source.path().to_path_buf();
    let mut config = load_config(&config_path)?;
    let dry_run = cli.dry_run;

    match cli.command {
        Commands::List {
            command: Some(ListCommand::Tidy),
            tags,
            open,
            verbose,
            ..
        } => {
            if verbose {
                return Err(anyhow!("--verbose is not supported with 'dsc list tidy'"));
            }
            if open {
                return Err(anyhow!("--open is not supported with 'dsc list tidy'"));
            }
            match tags {
                Some(_) => Err(anyhow!("--tags is not supported with 'dsc list tidy'")),
                None => commands::list::list_tidy(&config_path, &mut config),
            }
        }

        Commands::List {
            format,
            tags,
            open,
            verbose,
            ..
        } => commands::list::list_discourses(&config, format, tags.as_deref(), open, verbose),

        Commands::Add { names, interactive } => {
            commands::add::add_discourses(&mut config, &names, interactive)?;
            save_config(&config_path, &config)
        }

        Commands::Import { path } => {
            commands::import::import_discourses(&mut config, path.as_deref())?;
            save_config(&config_path, &config)
        }

        Commands::Update {
            command,
            name,
            parallel,
            post_changelog,
            yes,
            force,
            skip_recent,
        } => match command {
            Some(UpdateCommand::Log {
                latest,
                since,
                format,
            }) => {
                let since = since
                    .map(|s| commands::update_log::parse_duration(&s))
                    .transpose()?;
                commands::update_log::render(latest, since, format)
            }
            None => {
                let skip_recent = skip_recent
                    .map(|s| commands::update_log::parse_duration(&s))
                    .transpose()?;
                match name.as_deref() {
                    None => Err(anyhow!(
                        "specify a discourse name or 'all' (or `dsc update log`)"
                    )),
                    Some("all") if parallel == Some(0) => {
                        Err(anyhow!("--parallel width must be at least 1"))
                    }
                    Some("all") => commands::update::update_all(
                        &config,
                        parallel,
                        post_changelog,
                        yes,
                        force,
                        skip_recent,
                    ),
                    Some(_) if parallel.is_some() => {
                        Err(anyhow!("--parallel only applies to 'dsc update all'"))
                    }
                    Some(n) => commands::update::update_one(
                        &config,
                        n,
                        post_changelog,
                        yes,
                        force,
                        skip_recent,
                    ),
                }
            }
        },

        Commands::Emoji {
            command:
                EmojiCommand::Pull {
                    discourse,
                    output_dir,
                },
        } => commands::emoji::pull_emojis(&config, &discourse, &output_dir),

        Commands::Emoji {
            command:
                EmojiCommand::Push {
                    discourse,
                    emoji_path,
                    emoji_name,
                },
        } => commands::emoji::add_emoji(&config, &discourse, &emoji_path, emoji_name.as_deref()),

        Commands::Emoji {
            command:
                EmojiCommand::List {
                    discourse,
                    format,
                    verbose,
                    inline,
                },
        } => commands::emoji::list_emojis(&config, &discourse, format, verbose, inline),

        Commands::Topic { command } => match command {
            TopicCommand::Pull {
                discourse,
                topic_id,
                local_path,
                full,
                force,
            } => commands::topic::topic_pull(
                &config,
                &discourse,
                topic_id,
                local_path.as_deref(),
                full,
                force,
            ),

            TopicCommand::Push {
                discourse,
                local_path,
                topic_id,
                no_bump,
                skip_revision,
            } => commands::topic::topic_push(
                &config,
                &discourse,
                topic_id,
                &local_path,
                dry_run,
                dsc::api::PostEditOptions {
                    no_bump,
                    skip_revision,
                },
            ),

            TopicCommand::Sync {
                discourse,
                topic_id,
                local_path,
                yes,
            } => commands::topic::topic_sync(&config, &discourse, topic_id, &local_path, yes),

            TopicCommand::List {
                discourse,
                deleted,
                query,
                format,
            } => {
                commands::topic::topic_list(&config, &discourse, deleted, query.as_deref(), format)
            }

            TopicCommand::Reply {
                discourse,
                topic_id,
                local_path,
                format,
            } => commands::topic::topic_reply(
                &config,
                &discourse,
                topic_id,
                local_path.as_deref(),
                dry_run,
                format,
            ),

            TopicCommand::New {
                discourse,
                category_id,
                title,
                local_path,
                format,
            } => commands::topic::topic_new(
                &config,
                &discourse,
                category_id,
                &title,
                local_path.as_deref(),
                dry_run,
                format,
            ),

            TopicCommand::Delete {
                discourse,
                topic_id,
                mut topic_ids,
                purge,
            } => {
                topic_ids.insert(0, topic_id);
                commands::topic::topic_delete(&config, &discourse, &topic_ids, dry_run, purge)
            }

            TopicCommand::Restore {
                discourse,
                topic_id,
            } => commands::topic::topic_restore(&config, &discourse, topic_id, dry_run),

            TopicCommand::Tag {
                discourse,
                topic_id,
                tag,
            } => commands::tag::tag_apply(&config, &discourse, topic_id, &tag, dry_run),

            TopicCommand::Untag {
                discourse,
                topic_id,
                tag,
            } => commands::tag::tag_remove(&config, &discourse, topic_id, &tag, dry_run),

            TopicCommand::Title {
                discourse,
                topic_id,
                title,
            } => commands::topic::topic_title(&config, &discourse, topic_id, &title, dry_run),

            TopicCommand::Tags {
                discourse,
                topic_id,
                tags,
            } => commands::topic::topic_tags(&config, &discourse, topic_id, &tags, dry_run),
        },

        Commands::Category { command } => match command {
            CategoryCommand::List {
                discourse,
                format,
                verbose,
                tree,
            } => commands::category::category_list(&config, &discourse, format, verbose, tree),

            CategoryCommand::Copy {
                discourse,
                target,
                category,
            } => commands::category::category_copy(
                &config,
                &discourse,
                target.as_deref(),
                &category,
                dry_run,
            ),

            CategoryCommand::Pull {
                discourse,
                category,
                local_path,
                convert_admonitions,
                rewrite_links,
                force,
            } => commands::category::category_pull(
                &config,
                &discourse,
                &category,
                local_path.as_deref(),
                convert_admonitions,
                rewrite_links,
                force,
            ),

            CategoryCommand::Push {
                discourse,
                local_path,
                category,
                convert_admonitions,
                rewrite_links,
                updates_only,
                no_bump,
                skip_revision,
            } => commands::category::category_push(
                &config,
                &discourse,
                &category,
                &local_path,
                dry_run,
                commands::category::CategoryPushOptions {
                    updates_only,
                    edit: dsc::api::PostEditOptions {
                        no_bump,
                        skip_revision,
                    },
                    admonition_style: convert_admonitions,
                    rewrite_links,
                },
            ),

            CategoryCommand::Def { command } => match command {
                CategoryDefCommand::Pull {
                    discourse,
                    local_path,
                    force,
                } => commands::category_def::category_def_pull(
                    &config,
                    &discourse,
                    local_path.as_deref(),
                    force,
                ),
                CategoryDefCommand::Push {
                    discourse,
                    local_path,
                } => commands::category_def::category_def_push(
                    &config,
                    &discourse,
                    &local_path,
                    dry_run,
                ),
            },

            CategoryCommand::Diff {
                discourse_a,
                category_a,
                discourse_b,
                category_b,
                format,
            } => commands::category_def::category_diff(
                &config,
                &discourse_a,
                &category_a,
                &discourse_b,
                &category_b,
                format,
            ),

            CategoryCommand::Show {
                discourse,
                category,
                format,
            } => commands::category_def::category_show(&config, &discourse, &category, format),

            CategoryCommand::Get {
                discourse,
                category,
                field,
                format,
            } => {
                commands::category_def::category_get(&config, &discourse, &category, &field, format)
            }

            CategoryCommand::Set {
                discourse,
                category,
                field,
                value,
            } => commands::category_def::category_set(
                &config, &discourse, &category, &field, &value, dry_run,
            ),
            CategoryCommand::Rename {
                discourse,
                category,
                new_name,
            } => commands::category_def::category_rename(
                &config, &discourse, &category, &new_name, dry_run,
            ),
        },

        Commands::Group { command } => match command {
            GroupCommand::List {
                discourse,
                format,
                verbose,
            } => commands::group::group_list(&config, &discourse, format, verbose),
            GroupCommand::Info {
                discourse,
                group,
                format,
            } => commands::group::group_info(&config, &discourse, group, format),
            GroupCommand::Members {
                discourse,
                group,
                format,
            } => commands::group::group_members(&config, &discourse, group, format),

            GroupCommand::Copy {
                discourse,
                target,
                group,
            } => {
                commands::group::group_copy(&config, &discourse, target.as_deref(), group, dry_run)
            }

            GroupCommand::Add {
                discourse,
                group,
                local_path,
                notify,
            } => commands::group::group_add(
                &config,
                &discourse,
                group,
                local_path.as_deref(),
                notify,
                dry_run,
            ),
        },

        Commands::Pm { command } => match command {
            PmCommand::Send {
                discourse,
                recipients,
                title,
                local_path,
            } => commands::pm::pm_send(
                &config,
                &discourse,
                &recipients,
                &title,
                local_path.as_deref(),
                dry_run,
            ),
            PmCommand::List {
                discourse,
                username,
                direction,
                format,
            } => commands::pm::pm_list(&config, &discourse, &username, &direction, format),
        },

        Commands::Log { command } => match command {
            LogCommand::Staff {
                discourse,
                action,
                acting_user,
                target_user,
                subject,
                since,
                limit,
                format,
            } => commands::log::log_staff(
                &config,
                &discourse,
                commands::log::StaffLogOptions {
                    action: action.as_deref(),
                    acting_user: acting_user.as_deref(),
                    target_user: target_user.as_deref(),
                    subject: subject.as_deref(),
                    since: since.as_deref(),
                    limit,
                    format,
                },
            ),
        },

        Commands::Notification { command } => match command {
            NotificationCommand::List {
                discourse,
                filter,
                r#type,
                limit,
                format,
            } => commands::notification::notification_list(
                &config,
                &discourse,
                commands::notification::NotificationListOptions {
                    filter: filter.as_deref(),
                    types: r#type.as_deref(),
                    limit,
                    format,
                },
            ),
            NotificationCommand::Read {
                discourse,
                id,
                r#type,
                all,
            } => commands::notification::notification_read(
                &config,
                &discourse,
                id,
                r#type.as_deref(),
                all,
                dry_run,
            ),
        },

        Commands::ApiKey { command } => match command {
            ApiKeyCommand::List { discourse, format } => {
                commands::api_key::api_key_list(&config, &discourse, format)
            }
            ApiKeyCommand::Create {
                discourse,
                description,
                username,
                format,
            } => commands::api_key::api_key_create(
                &config,
                &discourse,
                &description,
                username.as_deref(),
                format,
                dry_run,
            ),
            ApiKeyCommand::Revoke { discourse, key_id } => {
                commands::api_key::api_key_revoke(&config, &discourse, key_id, dry_run)
            }
        },

        Commands::Webhook { command } => match command {
            WebhookCommand::List { discourse, format } => {
                commands::webhook::webhook_list(&config, &discourse, format)
            }
            WebhookCommand::Create {
                discourse,
                payload_url,
                content_type,
                secret_stdin,
                active,
                verify_certificate,
                format,
            } => commands::webhook::webhook_create(
                &config,
                &discourse,
                commands::webhook::WebhookCreateOptions {
                    payload_url: &payload_url,
                    content_type: map_webhook_content_type(content_type),
                    secret_from_stdin: secret_stdin,
                    active,
                    verify_certificate,
                    format,
                    dry_run,
                },
            ),
            WebhookCommand::Delete {
                discourse,
                webhook_id,
                format,
            } => {
                commands::webhook::webhook_delete(&config, &discourse, webhook_id, format, dry_run)
            }
            WebhookCommand::Ping {
                discourse,
                webhook_id,
                format,
            } => commands::webhook::webhook_ping(&config, &discourse, webhook_id, format, dry_run),
        },

        Commands::Invite { command } => match command {
            InviteCommand::Send {
                discourse,
                email,
                group,
                topic,
                message,
            } => commands::invite::invite_one(
                &config,
                &discourse,
                &email,
                &group,
                topic,
                message.as_deref(),
                dry_run,
            ),
            InviteCommand::Bulk {
                discourse,
                local_path,
                group,
                topic,
                message,
            } => commands::invite::invite_bulk(
                &config,
                &discourse,
                local_path.as_deref(),
                &group,
                topic,
                message.as_deref(),
                dry_run,
            ),
        },

        Commands::User { command } => match command {
            UserCommand::List {
                discourse,
                listing,
                page,
                format,
            } => commands::user::user_list(&config, &discourse, &listing, page, format),
            UserCommand::Info {
                discourse,
                username,
                format,
            } => commands::user::user_info(&config, &discourse, &username, format),
            UserCommand::Suspend {
                discourse,
                username,
                until,
                reason,
            } => commands::user::user_suspend(
                &config, &discourse, &username, &until, &reason, dry_run,
            ),
            UserCommand::Unsuspend {
                discourse,
                username,
            } => commands::user::user_unsuspend(&config, &discourse, &username, dry_run),
            UserCommand::Silence {
                discourse,
                username,
                until,
                reason,
            } => commands::user::user_silence(
                &config, &discourse, &username, &until, &reason, dry_run,
            ),
            UserCommand::Unsilence {
                discourse,
                username,
            } => commands::user::user_unsilence(&config, &discourse, &username, dry_run),
            UserCommand::Promote {
                discourse,
                username,
                role,
            } => commands::user::user_promote(
                &config,
                &discourse,
                &username,
                map_role(role),
                dry_run,
            ),
            UserCommand::Demote {
                discourse,
                username,
                role,
            } => {
                commands::user::user_demote(&config, &discourse, &username, map_role(role), dry_run)
            }
            UserCommand::Create {
                discourse,
                email,
                username,
                name,
                password_stdin,
                approve,
            } => commands::user::user_create(
                &config,
                &discourse,
                &email,
                &username,
                name.as_deref(),
                password_stdin,
                approve,
                dry_run,
            ),
            UserCommand::PasswordReset {
                discourse,
                username,
            } => commands::user::user_password_reset(&config, &discourse, &username, dry_run),
            UserCommand::EmailSet {
                discourse,
                username,
                email,
            } => commands::user::user_email_set(&config, &discourse, &username, &email, dry_run),
            UserCommand::Activity {
                discourse,
                username,
                since,
                types,
                limit,
                format,
            } => {
                let names: Vec<String> = vec![types];
                commands::user::user_activity(
                    &config,
                    &discourse,
                    &username,
                    &names,
                    since.as_deref(),
                    limit,
                    map_activity_format(format),
                )
            }
            UserCommand::Groups { command } => match command {
                UserGroupsCommand::List {
                    discourse,
                    username,
                    format,
                } => commands::user::user_groups_list(&config, &discourse, &username, format),
                UserGroupsCommand::Add {
                    discourse,
                    username,
                    group_id,
                    notify,
                } => commands::user::user_groups_add(
                    &config, &discourse, &username, group_id, notify, dry_run,
                ),
                UserGroupsCommand::Remove {
                    discourse,
                    username,
                    group_id,
                } => commands::user::user_groups_remove(
                    &config, &discourse, &username, group_id, dry_run,
                ),
            },
        },

        Commands::Backup { command } => match command {
            BackupCommand::Create { discourse } => {
                commands::backup::backup_create(&config, &discourse)
            }

            BackupCommand::List {
                discourse,
                format,
                verbose,
            } => commands::backup::backup_list(&config, &discourse, format, verbose),

            BackupCommand::Pull {
                discourse,
                backup_filename,
                local_path,
                force,
            } => commands::backup::backup_pull(
                &config,
                &discourse,
                &backup_filename,
                local_path.as_deref(),
                force,
            ),

            BackupCommand::Push {
                discourse,
                backup_path,
            } => commands::backup::backup_restore(&config, &discourse, &backup_path, dry_run),

            BackupCommand::SetupS3 {
                discourse,
                region,
                bucket,
                no_test,
            } => commands::backup_s3::setup_s3(
                &config,
                &discourse,
                &region,
                bucket.as_deref(),
                no_test,
                dry_run,
            ),

            BackupCommand::Health {
                discourse,
                tags,
                max_age,
                format,
            } => commands::backup::backup_health(
                &config,
                discourse.as_deref(),
                tags.as_deref(),
                max_age,
                format,
            ),
        },

        Commands::App { command } => match command {
            AppCommand::Env { command } => match command {
                AppEnvCommand::List { discourse, format } => {
                    commands::app::app_env_list(&config, &discourse, format)
                }
                AppEnvCommand::Get {
                    discourse,
                    key,
                    show_secret,
                    format,
                } => commands::app::app_env_get(&config, &discourse, &key, show_secret, format),
                AppEnvCommand::Audit { key, tags, format } => {
                    commands::app::app_env_audit(&config, &key, tags.as_deref(), format)
                }
                AppEnvCommand::Set {
                    discourse,
                    key,
                    value,
                    rebuild,
                    backup,
                    yes,
                } => commands::app::app_env_set(
                    &config,
                    &discourse,
                    &key,
                    &value,
                    commands::app::AppEnvChangeOptions {
                        rebuild,
                        backup,
                        dry_run,
                        yes,
                    },
                ),
                AppEnvCommand::Unset {
                    discourse,
                    key,
                    rebuild,
                    backup,
                    yes,
                } => commands::app::app_env_unset(
                    &config,
                    &discourse,
                    &key,
                    commands::app::AppEnvChangeOptions {
                        rebuild,
                        backup,
                        dry_run,
                        yes,
                    },
                ),
            },
        },

        Commands::Palette { command } => {
            eprintln!(
                "note: `dsc palette` is deprecated and will move under `dsc theme palette`; \
                 please use `dsc theme palette` instead."
            );
            run_palette(&config, command)
        }

        Commands::Plugin { command } => match command {
            PluginCommand::List {
                discourse,
                format,
                verbose,
            } => commands::plugin::plugin_list(&config, &discourse, format, verbose),
            PluginCommand::Install { discourse, url } => {
                commands::plugin::plugin_install(&config, &discourse, &url, dry_run)
            }
            PluginCommand::Remove { discourse, name } => {
                commands::plugin::plugin_remove(&config, &discourse, &name, dry_run)
            }
        },

        Commands::Theme { command } => match command {
            ThemeCommand::List {
                discourse,
                format,
                verbose,
            } => commands::theme::theme_list(&config, &discourse, format, verbose),
            ThemeCommand::Install {
                discourse,
                source,
                branch,
            } => commands::theme::theme_install(
                &config,
                &discourse,
                &source,
                branch.as_deref(),
                dry_run,
            ),
            ThemeCommand::Remove { discourse, name } => {
                commands::theme::theme_remove(&config, &discourse, &name, dry_run)
            }
            ThemeCommand::Delete {
                discourse,
                theme_id,
            } => commands::theme::theme_delete(&config, &discourse, theme_id, dry_run),
            ThemeCommand::Pull {
                discourse,
                theme_id,
                local_path,
                force,
            } => commands::theme::theme_pull(
                &config,
                &discourse,
                theme_id,
                local_path.as_deref(),
                force,
            ),
            ThemeCommand::Push {
                discourse,
                local_path,
                theme_id,
                yes,
            } => commands::theme::theme_push(
                &config,
                &discourse,
                &local_path,
                theme_id,
                dry_run,
                yes,
            ),
            ThemeCommand::Duplicate {
                discourse,
                theme_id,
                format,
            } => commands::theme::theme_duplicate(&config, &discourse, theme_id, format),
            ThemeCommand::Show {
                discourse,
                theme_id,
                format,
            } => commands::theme::theme_show(&config, &discourse, theme_id, format),
            ThemeCommand::Setting { command } => match command {
                ThemeSettingCommand::List {
                    discourse,
                    theme_id,
                    format,
                } => commands::theme::theme_setting_list(&config, &discourse, theme_id, format),
                ThemeSettingCommand::Get {
                    discourse,
                    theme_id,
                    key,
                    format,
                } => {
                    commands::theme::theme_setting_get(&config, &discourse, theme_id, &key, format)
                }
                ThemeSettingCommand::Set {
                    discourse,
                    theme_id,
                    key,
                    value,
                } => commands::theme::theme_setting_set(
                    &config, &discourse, theme_id, &key, &value, dry_run,
                ),
                ThemeSettingCommand::Pull {
                    discourse,
                    theme_id,
                    local_path,
                    force,
                } => commands::theme::theme_setting_pull(
                    &config,
                    &discourse,
                    theme_id,
                    local_path.as_deref(),
                    force,
                ),
                ThemeSettingCommand::Push {
                    discourse,
                    theme_id,
                    local_path,
                } => commands::theme::theme_setting_push(
                    &config,
                    &discourse,
                    theme_id,
                    &local_path,
                    dry_run,
                ),
            },
            ThemeCommand::Enable {
                discourse,
                theme_id,
            } => commands::theme::theme_set_enabled(&config, &discourse, theme_id, true, dry_run),
            ThemeCommand::Disable {
                discourse,
                theme_id,
            } => commands::theme::theme_set_enabled(&config, &discourse, theme_id, false, dry_run),
            ThemeCommand::Attach {
                discourse,
                parent_id,
                component_id,
            } => commands::theme::theme_set_child(
                &config,
                &discourse,
                parent_id,
                component_id,
                true,
                dry_run,
            ),
            ThemeCommand::Detach {
                discourse,
                parent_id,
                component_id,
            } => commands::theme::theme_set_child(
                &config,
                &discourse,
                parent_id,
                component_id,
                false,
                dry_run,
            ),
            ThemeCommand::Field { command } => match command {
                ThemeFieldCommand::List {
                    discourse,
                    theme_id,
                    format,
                } => commands::theme::theme_field_list(&config, &discourse, theme_id, format),
                ThemeFieldCommand::Pull {
                    discourse,
                    theme_id,
                    field,
                    local_path,
                    force,
                } => commands::theme::theme_field_pull(
                    &config,
                    &discourse,
                    theme_id,
                    &field,
                    local_path.as_deref(),
                    force,
                ),
                ThemeFieldCommand::Push {
                    discourse,
                    theme_id,
                    field,
                    local_path,
                    yes,
                } => commands::theme::theme_field_push(
                    &config,
                    &discourse,
                    theme_id,
                    &field,
                    &local_path,
                    dry_run,
                    yes,
                ),
            },
            ThemeCommand::Asset { command } => match command {
                ThemeAssetCommand::List {
                    discourse,
                    theme_id,
                    format,
                } => commands::theme::theme_asset_list(&config, &discourse, theme_id, format),
                ThemeAssetCommand::Set {
                    discourse,
                    theme_id,
                    name,
                    file,
                } => commands::theme::theme_asset_set(
                    &config, &discourse, theme_id, &name, &file, dry_run,
                ),
                ThemeAssetCommand::Unset {
                    discourse,
                    theme_id,
                    name,
                } => commands::theme::theme_asset_unset(
                    &config, &discourse, theme_id, &name, dry_run,
                ),
            },
            ThemeCommand::Update {
                discourse,
                theme_id,
                check,
            } => commands::theme::theme_update(&config, &discourse, theme_id, check, dry_run),
            ThemeCommand::Palette { command } => run_palette(&config, command),
        },

        Commands::Setting {
            command:
                SettingCommand::Set {
                    discourse,
                    setting,
                    value,
                    tags,
                },
        } => {
            // When --tags is provided, the user passes only `<setting> <value>`;
            // clap fills the first two positionals (discourse, setting) and leaves
            // `value` as None. Shift the arguments so the command layer sees the
            // correct values.
            let (discourse_arg, setting_arg, value_arg) = if tags.is_some() {
                if value.is_some() {
                    return Err(anyhow::anyhow!(
                        "cannot pass <discourse> together with --tags; specify either a single discourse or a tag filter"
                    ));
                }
                let shifted_setting =
                    discourse.ok_or_else(|| anyhow::anyhow!("missing <setting> argument"))?;
                let shifted_value =
                    setting.ok_or_else(|| anyhow::anyhow!("missing <value> argument"))?;
                (None, shifted_setting, shifted_value)
            } else {
                let d = discourse.ok_or_else(|| {
                    anyhow::anyhow!("missing <discourse> argument (or pass --tags)")
                })?;
                let s = setting.ok_or_else(|| anyhow::anyhow!("missing <setting> argument"))?;
                let v = value.ok_or_else(|| anyhow::anyhow!("missing <value> argument"))?;
                (Some(d), s, v)
            };
            commands::setting::set_site_setting(
                &config,
                discourse_arg.as_deref(),
                &setting_arg,
                &value_arg,
                tags.as_deref(),
                dry_run,
            )
        }

        Commands::Setting {
            command:
                SettingCommand::Get {
                    discourse,
                    setting,
                    format,
                },
        } => commands::setting::get_site_setting(&config, &discourse, &setting, format),

        Commands::Setting {
            command:
                SettingCommand::List {
                    discourse,
                    format,
                    verbose,
                },
        } => commands::setting::list_site_settings(&config, &discourse, format, verbose),

        Commands::Setting {
            command:
                SettingCommand::Pull {
                    discourse,
                    local_path,
                    changed_only,
                    category,
                    force,
                },
        } => commands::setting::pull_settings(
            &config,
            &discourse,
            &local_path,
            changed_only,
            category.as_deref(),
            force,
        ),

        Commands::Setting {
            command:
                SettingCommand::Push {
                    discourse,
                    local_path,
                    reset_unlisted,
                    yes,
                },
        } => commands::setting::push_settings(
            &config,
            &discourse,
            &local_path,
            reset_unlisted,
            dry_run,
            yes,
        ),

        Commands::Setting {
            command:
                SettingCommand::Diff {
                    source,
                    target,
                    changed_only,
                    category,
                    format,
                },
        } => commands::setting::diff_settings(
            &config,
            &source,
            &target,
            changed_only,
            category.as_deref(),
            format,
        ),

        Commands::Setting {
            command:
                SettingCommand::Audit {
                    setting,
                    tags,
                    format,
                },
        } => commands::setting::audit_site_setting(&config, &setting, tags.as_deref(), format),

        Commands::Sar {
            discourse,
            user,
            output,
            messages,
        } => commands::sar::sar(
            &config,
            &discourse,
            &user,
            output.as_deref(),
            messages,
            dry_run,
        ),

        Commands::Open { discourse } => commands::open::open_discourse(&config, &discourse),

        Commands::Harden {
            host,
            ssh_user,
            new_user,
            ssh_port,
            pubkey_file,
        } => commands::harden::harden(
            &config.harden,
            &host,
            &ssh_user,
            new_user.as_deref(),
            ssh_port,
            &pubkey_file,
            dry_run,
        ),

        Commands::Search {
            discourse,
            query,
            format,
        } => commands::search::search(&config, &discourse, &query, format),

        Commands::Analytics {
            discourse,
            since,
            compare,
            snapshot,
            periods,
            section,
            format,
        } => commands::analytics::analytics(
            &config,
            &discourse,
            &since,
            compare,
            snapshot,
            periods.as_deref(),
            map_section(section),
            format,
        ),

        Commands::Explorer { command } => match command {
            ExplorerCommand::List {
                discourse,
                filter,
                order,
                ascending,
                format,
            } => commands::explorer::explorer_list(
                &config,
                &discourse,
                commands::explorer::ExplorerListOptions {
                    filter: filter.as_deref(),
                    order: order.map(ExplorerOrderArg::as_str),
                    ascending,
                    format,
                },
            ),
            ExplorerCommand::Show {
                discourse,
                query_id,
                export,
                format,
            } => commands::explorer::explorer_show(
                &config,
                &discourse,
                query_id,
                export.as_deref(),
                format,
            ),
            ExplorerCommand::Run {
                discourse,
                query_id,
                params,
                params_file,
                csv,
                explain,
                limit,
                format,
            } => commands::explorer::explorer_run(
                &config,
                &discourse,
                query_id,
                commands::explorer::ExplorerRunOptions {
                    params: params.as_deref(),
                    params_file: params_file.as_deref(),
                    csv: csv.as_deref(),
                    explain,
                    limit,
                    format,
                },
                dry_run,
            ),
        },

        Commands::Upload {
            discourse,
            file,
            upload_type,
            format,
        } => commands::upload::upload(&config, &discourse, &file, &upload_type, format),

        Commands::Post { command } => match command {
            PostCommand::Pull {
                discourse,
                post_id,
                local_path,
                force,
            } => commands::post::post_pull(
                &config,
                &discourse,
                post_id,
                local_path.as_deref(),
                force,
            ),
            PostCommand::Push {
                discourse,
                post_id,
                local_path,
            } => commands::post::post_edit(
                &config,
                &discourse,
                post_id,
                local_path.as_deref(),
                dry_run,
            ),
            PostCommand::Delete { discourse, post_id } => {
                commands::post::post_delete(&config, &discourse, post_id, dry_run)
            }
            PostCommand::Move {
                discourse,
                post_id,
                to_topic,
            } => commands::post::post_move(&config, &discourse, post_id, to_topic, dry_run),
        },

        Commands::Tag { command } => match command {
            TagCommand::List { discourse, format } => {
                commands::tag::tag_list(&config, &discourse, format)
            }
            TagCommand::Pull {
                discourse,
                local_path,
                force,
            } => commands::tag::tag_pull(&config, &discourse, &local_path, force),
            TagCommand::Push {
                discourse,
                local_path,
                prune,
                yes,
            } => commands::tag::tag_push(&config, &discourse, &local_path, prune, dry_run, yes),
            TagCommand::Rename {
                discourse,
                old_name,
                new_name,
            } => commands::tag::tag_rename(&config, &discourse, &old_name, &new_name, dry_run),
        },

        Commands::Config {
            command:
                Some(ConfigCommand::Check {
                    format,
                    skip_ssh,
                    parallel,
                    max,
                }),
        } => commands::config::config_check(&config, format, skip_ssh, parallel, max),

        Commands::Doctor {
            format,
            skip_ssh,
            parallel,
            max,
        } => commands::config::config_check(&config, format, skip_ssh, parallel, max),

        Commands::Config { command: None } => {
            let candidates = config_search_paths();
            // Show env-var overrides up front so the user can see why a
            // path outside the standard hierarchy is active.
            let env_config = std::env::var(ENV_CONFIG).ok();
            let env_config_home = std::env::var(ENV_CONFIG_HOME).ok();
            println!(
                "${}: {}",
                ENV_CONFIG,
                env_config.as_deref().unwrap_or("(unset)")
            );
            println!(
                "${}: {}",
                ENV_CONFIG_HOME,
                env_config_home.as_deref().unwrap_or("(unset)")
            );
            println!();
            println!(
                "Active config: {} ({})",
                config_path.display(),
                config_source.label()
            );
            // Only show the discovered hierarchy when no explicit
            // selector overrode it; otherwise the list is misleading.
            let from_hierarchy = matches!(
                config_source,
                ConfigSource::Discovered(_) | ConfigSource::Default(_)
            );
            if from_hierarchy {
                println!();
                println!("Search order:");
                for (i, path) in candidates.iter().enumerate() {
                    let exists = path.exists();
                    let marker = if path == &config_path {
                        " <-- active"
                    } else {
                        ""
                    };
                    println!(
                        "  {}. {}{}{}",
                        i + 1,
                        path.display(),
                        if exists && marker.is_empty() {
                            " (exists)"
                        } else {
                            ""
                        },
                        marker
                    );
                }
            }
            Ok(())
        }

        Commands::Completions {
            command,
            shell,
            dir,
        } => commands::completions::run(command, shell, dir.as_deref()),

        Commands::Man { dir } => commands::manpages::write_manpages(&dir),

        Commands::Version { discourse, format } => {
            commands::version::version(&config, discourse.as_deref(), format)
        }
    }
}
