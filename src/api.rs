// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod api_keys;
mod backup;
mod categories;
mod client;
mod emoji;
mod error;
mod explorer;
mod groups;
mod invites;
mod models;
mod notifications;
mod palettes;
mod plugins;
mod rate_limit;
mod reports;
mod search;
mod settings;
mod staff_action_logs;
mod tags;
mod themes;
mod topics;
mod uploads;
mod user_actions;
mod users;
mod webhooks;

pub use api_keys::{ApiKeySummary, CreatedApiKey};
pub use client::{DiscourseClient, VersionInfo};
pub use explorer::{
    ExplorerParamInfo, ExplorerQueryCatalogue, ExplorerQueryDetails, ExplorerQuerySummary,
    ExplorerRunResult,
};
pub use groups::AddMembersOutcome;
pub use invites::InviteResult;
pub use models::*;
pub use notifications::{Notification, NotificationFilter, NotificationPage};
pub(crate) use palettes::color_schemes;
pub use reports::{AdminReport, ReportPoint};
pub use search::SearchHit;
pub use settings::SiteSettingDetail;
pub use staff_action_logs::{StaffActionLog, StaffActionLogFilter, StaffLogUser};
pub use tags::TagGroupInfo;
pub use tags::TagInfo;
pub use topics::{DeletedTopicSummary, PmTopicSummary, PostEditOptions, PostInfo};
pub use uploads::UploadInfo;
pub use user_actions::UserAction;
pub use users::{UserDetail, UserSummary};
pub use webhooks::WebhookSummary;
pub(crate) use webhooks::redact_webhook_url;
