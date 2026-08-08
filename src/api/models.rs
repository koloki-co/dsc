// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use serde::{Deserialize, Serialize};

/// Response payload for site.json.
#[derive(Debug, Deserialize)]
pub struct SiteResponse {
    pub site: SiteInfo,
}

/// Site metadata.
#[derive(Debug, Deserialize)]
pub struct SiteInfo {
    pub title: String,
}

/// Response payload for about.json.
#[derive(Debug, Deserialize)]
pub struct AboutResponse {
    pub about: AboutInfo,
}

/// About metadata.
#[derive(Debug, Deserialize)]
pub struct AboutInfo {
    pub version: Option<String>,
    pub installed_version: Option<String>,
}

/// Response payload for topic JSON.
#[derive(Debug, Deserialize)]
pub struct TopicResponse {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub posts_count: Option<u64>,
    #[serde(default)]
    pub category_id: Option<u64>,
    #[serde(default)]
    pub deleted_at: Option<String>,
    pub post_stream: PostStream,
}

/// Topic post stream.
#[derive(Debug, Deserialize, Default)]
pub struct PostStream {
    #[serde(default)]
    pub posts: Vec<Post>,
    /// Flat array of every post ID in the topic. Discourse includes this
    /// on the first-page response only. Used to paginate the rest of the
    /// thread via the batch-fetch endpoint.
    #[serde(default)]
    pub stream: Vec<u64>,
}

/// Topic post.
#[derive(Debug, Deserialize)]
pub struct Post {
    pub id: u64,
    #[serde(default)]
    pub post_number: Option<u64>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CustomEmoji {
    pub name: String,
    pub url: String,
}

/// Response payload for category JSON.
#[derive(Debug, Deserialize)]
pub struct CategoryResponse {
    #[serde(default)]
    pub category: Option<CategoryInfo>,
    pub topic_list: TopicList,
}

/// Category metadata.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CategoryInfo {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub text_color: Option<String>,
    pub id: Option<u64>,
    #[serde(default)]
    pub subcategory_list: Vec<CategoryInfo>,
    #[serde(default)]
    pub parent_category_id: Option<u64>,
}

/// One group's permission on a category, as returned in `group_permissions`.
/// `permission_type`: 1 = full, 2 = create_post, 3 = readonly.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroupPermission {
    #[serde(default)]
    pub group_name: Option<String>,
    pub permission_type: u8,
}

/// The full definition of a category (from `/categories.json?show_permissions=true`).
///
/// Distinct from the sparse [`CategoryInfo`] used by `category list`: this carries
/// the definition surface (description, permissions, topic template, tag rules,
/// ordering) that `category def pull/push` and `category show/get/set` operate on.
/// Every field beyond `name` is optional so partial payloads still deserialize.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CategoryDefinition {
    #[serde(default)]
    pub id: Option<u64>,
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub text_color: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub parent_category_id: Option<u64>,
    #[serde(default)]
    pub read_restricted: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
    /// Plain-text form of `description`. `description` itself is the *cooked*
    /// excerpt of the category's auto-created "About" topic (HTML, and settles
    /// asynchronously after creation), so definition sync reads this instead for
    /// a stable, idempotent round-trip.
    #[serde(default)]
    pub description_text: Option<String>,
    #[serde(default)]
    pub topic_template: Option<String>,
    #[serde(default)]
    pub group_permissions: Option<Vec<GroupPermission>>,
    #[serde(default)]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_tag_groups: Option<Vec<String>>,
    #[serde(default)]
    pub minimum_required_tags: Option<u64>,
    #[serde(default)]
    pub sort_order: Option<String>,
    #[serde(default)]
    pub default_view: Option<String>,
    #[serde(default)]
    pub subcategory_list_style: Option<String>,
    #[serde(default)]
    pub num_featured_topics: Option<u64>,
    #[serde(default)]
    pub show_subcategory_list: Option<bool>,
}

/// Response payload for `/categories.json?show_permissions=true`.
#[derive(Debug, Deserialize)]
pub struct CategoryDefinitionsResponse {
    pub category_list: CategoryDefinitionList,
}

/// Category definition listing.
#[derive(Debug, Deserialize)]
pub struct CategoryDefinitionList {
    pub categories: Vec<CategoryDefinition>,
}

/// Response payload for categories.json.
#[derive(Debug, Deserialize)]
pub struct CategoriesResponse {
    pub category_list: CategoryList,
}

/// Category listing.
#[derive(Debug, Deserialize)]
pub struct CategoryList {
    pub categories: Vec<CategoryInfo>,
}

/// Topic list for a category.
#[derive(Debug, Deserialize)]
pub struct TopicList {
    #[serde(default)]
    pub topics: Vec<TopicSummary>,
    #[serde(default)]
    pub more_topics_url: Option<String>,
}

/// Topic summary.
#[derive(Debug, Deserialize, Serialize)]
pub struct TopicSummary {
    pub id: u64,
    pub title: String,
    pub slug: String,
}

/// Group summary.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroupSummary {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub full_name: Option<String>,
}

/// Response payload for groups.json.
#[derive(Debug, Deserialize)]
pub struct GroupsResponse {
    pub groups: Vec<GroupSummary>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroupMember {
    pub id: u64,
    pub username: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GroupMembersResponse {
    pub members: Vec<GroupMember>,
}

/// Response payload for group detail.
#[derive(Debug, Deserialize)]
pub struct GroupDetailResponse {
    pub group: GroupDetail,
}

/// Group details with settings used for deep-copy.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroupDetail {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub full_name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub grant_trust_level: Option<u64>,
    #[serde(default)]
    pub visibility_level: Option<u64>,
    #[serde(default)]
    pub mentionable_level: Option<u64>,
    #[serde(default)]
    pub messageable_level: Option<u64>,
    #[serde(default)]
    pub default_notification_level: Option<u64>,
    #[serde(default)]
    pub members_visibility_level: Option<u64>,
    #[serde(default)]
    pub primary_group: Option<bool>,
    #[serde(default)]
    pub public_admission: Option<bool>,
    #[serde(default)]
    pub public_exit: Option<bool>,
    #[serde(default)]
    pub allow_membership_requests: Option<bool>,
    #[serde(default)]
    pub automatic_membership_email_domains: Option<String>,
    #[serde(default)]
    pub automatic_membership_retroactive: Option<bool>,
    #[serde(default)]
    pub membership_request_template: Option<String>,
    #[serde(default)]
    pub flair_icon: Option<String>,
    #[serde(default)]
    pub flair_upload_id: Option<u64>,
    #[serde(default)]
    pub flair_color: Option<String>,
    #[serde(default)]
    pub flair_background_color: Option<String>,
    #[serde(default)]
    pub bio_raw: Option<String>,
    /// Category IDs a group's members watch by default, keyed by notification level.
    /// Populated by the API for admins/owners; omitted from output unless requested.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watching_category_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracking_category_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watching_first_post_category_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regular_category_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted_category_ids: Vec<u64>,
    /// Tags a group's members watch by default, keyed by notification level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watching_tags: Vec<GroupNotificationTag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracking_tags: Vec<GroupNotificationTag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watching_first_post_tags: Vec<GroupNotificationTag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regular_tags: Vec<GroupNotificationTag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted_tags: Vec<GroupNotificationTag>,
}

impl GroupDetail {
    /// Clear the group/category/tag notification-default fields so they are omitted
    /// from serialized output unless explicitly requested (`--with-defaults`).
    pub fn clear_notification_defaults(&mut self) {
        self.watching_category_ids.clear();
        self.tracking_category_ids.clear();
        self.watching_first_post_category_ids.clear();
        self.regular_category_ids.clear();
        self.muted_category_ids.clear();
        self.watching_tags.clear();
        self.tracking_tags.clear();
        self.watching_first_post_tags.clear();
        self.regular_tags.clear();
        self.muted_tags.clear();
    }
}

/// A tag referenced by a group's tag notification defaults.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroupNotificationTag {
    pub id: u64,
    pub name: String,
    pub slug: String,
}

/// Response payload for creating a post/topic.
#[derive(Debug, Deserialize)]
pub struct CreatePostResponse {
    pub id: u64,
    pub topic_id: u64,
}

/// Response payload for creating a category.
#[derive(Debug, Deserialize)]
pub struct CreateCategoryResponse {
    pub category: CreatedCategory,
}

/// Created category payload.
#[derive(Debug, Deserialize)]
pub struct CreatedCategory {
    pub id: u64,
}

/// Response payload for creating a group.
#[derive(Debug, Deserialize)]
pub struct CreateGroupResponse {
    pub group: CreatedGroup,
}

/// Created group payload.
#[derive(Debug, Deserialize)]
pub struct CreatedGroup {
    pub id: u64,
}
