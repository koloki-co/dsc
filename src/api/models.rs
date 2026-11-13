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
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub archived: bool,
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

/// One tag group whose tags are required when creating a topic in a category.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct RequiredTagGroup {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub min_count: Option<u64>,
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
    /// Category style: `square` (colour swatch, the default), `icon`, or `emoji`.
    #[serde(default)]
    pub style_type: Option<String>,
    /// FontAwesome icon name, used when `style_type` is `icon`.
    #[serde(default)]
    pub icon: Option<String>,
    /// Emoji shortcode (without colons), used when `style_type` is `emoji`.
    #[serde(default)]
    pub emoji: Option<String>,
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
    /// Placeholder text shown in the topic-title field when composing a new
    /// topic in this category (e.g. "Enter your band's genre and location").
    #[serde(default)]
    pub topic_title_placeholder: Option<String>,
    #[serde(default)]
    pub group_permissions: Option<Vec<GroupPermission>>,
    #[serde(default)]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_tag_groups: Option<Vec<String>>,
    #[serde(default)]
    pub minimum_required_tags: Option<u64>,
    #[serde(default)]
    pub required_tag_groups: Option<Vec<RequiredTagGroup>>,
    /// Registered category types keyed by type ID. The metadata values are
    /// server-defined, so definition sync preserves only their stable keys.
    #[serde(default)]
    pub category_types: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    /// Complete category custom fields, available from `/c/{id}/show.json`.
    #[serde(default)]
    pub custom_fields: Option<std::collections::BTreeMap<String, serde_json::Value>>,
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
    #[serde(default)]
    pub subcategory_list: Vec<CategoryDefinition>,
}

/// Response payload for `/categories.json?show_permissions=true`.
#[derive(Debug, Deserialize)]
pub struct CategoryDefinitionsResponse {
    pub category_list: CategoryDefinitionList,
}

/// Response payload for `/c/{id}/show.json`.
#[derive(Debug, Deserialize)]
pub struct CategoryDefinitionResponse {
    pub category: CategoryDefinition,
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
    /// True for Discourse's built-in groups (admins, staff, trust_level_*).
    /// Required for destructive-action safety, but never serialised so
    /// `group info` output is unchanged.
    #[serde(skip_serializing)]
    pub automatic: bool,
    /// Category IDs a group's members watch by default, keyed by notification level.
    /// Populated by the API for admins/owners; omitted from output unless requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watching_category_ids: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_category_ids: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watching_first_post_category_ids: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regular_category_ids: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_category_ids: Option<Vec<u64>>,
    /// Tags a group's members watch by default, keyed by notification level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watching_tags: Option<Vec<GroupNotificationTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_tags: Option<Vec<GroupNotificationTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watching_first_post_tags: Option<Vec<GroupNotificationTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regular_tags: Option<Vec<GroupNotificationTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_tags: Option<Vec<GroupNotificationTag>>,
}

impl GroupDetail {
    /// Clear the group/category/tag notification-default fields so they are omitted
    /// from serialized output unless explicitly requested (`--with-defaults`).
    pub fn clear_notification_defaults(&mut self) {
        self.watching_category_ids = None;
        self.tracking_category_ids = None;
        self.watching_first_post_category_ids = None;
        self.regular_category_ids = None;
        self.muted_category_ids = None;
        self.watching_tags = None;
        self.tracking_tags = None;
        self.watching_first_post_tags = None;
        self.regular_tags = None;
        self.muted_tags = None;
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

/// Deserialize a topic-list row's `tags` field into tag names.
///
/// Discourse has returned this field in two shapes: a legacy array of tag-name
/// strings, and (current releases, observed on `2026.11.0-latest.1`) an array
/// of tag objects such as `{"id": 1, "name": "bug", "slug": "bug"}`. Both are
/// accepted; an object contributes its `name`, falling back to `slug`. A
/// missing or `null` field is `None`. Any other shape (a non-array value, or an
/// entry that is neither a string nor an object carrying a string `name` or
/// `slug`) is rejected with a shape-only error that never echoes the payload,
/// rather than being silently dropped or coerced.
pub(crate) fn deserialize_tag_names<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;

    let Some(value) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return Err(D::Error::custom(format!(
            "unsupported topic `tags` shape: expected an array, got {}",
            json_kind(&value)
        )));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| match item {
            Value::String(name) => Ok(name.clone()),
            Value::Object(map) => ["name", "slug"]
                .iter()
                .find_map(|key| map.get(*key).and_then(Value::as_str))
                .map(str::to_string)
                .ok_or_else(|| {
                    D::Error::custom(format!(
                        "unsupported topic tag object at index {index}: expected a string `name` or `slug`"
                    ))
                }),
            other => Err(D::Error::custom(format!(
                "unsupported topic tag entry at index {index}: expected a string or tag object, got {}",
                json_kind(other)
            ))),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tag_name_tests {
    use super::deserialize_tag_names;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Row {
        #[serde(default, deserialize_with = "deserialize_tag_names")]
        tags: Option<Vec<String>>,
    }

    fn parse(json: &str) -> Result<Option<Vec<String>>, String> {
        serde_json::from_str::<Row>(json)
            .map(|row| row.tags)
            .map_err(|err| err.to_string())
    }

    #[test]
    fn accepts_legacy_string_tags() {
        assert_eq!(
            parse(r#"{"tags":["bug","ops"]}"#).unwrap(),
            Some(vec!["bug".to_string(), "ops".to_string()])
        );
    }

    #[test]
    fn accepts_current_tag_objects_preferring_name_then_slug() {
        assert_eq!(
            parse(r#"{"tags":[{"id":1,"name":"Bug","slug":"bug"},{"id":2,"slug":"ops"}]}"#)
                .unwrap(),
            Some(vec!["Bug".to_string(), "ops".to_string()])
        );
    }

    #[test]
    fn missing_null_and_empty_tags() {
        assert_eq!(parse(r#"{}"#).unwrap(), None);
        assert_eq!(parse(r#"{"tags":null}"#).unwrap(), None);
        assert_eq!(parse(r#"{"tags":[]}"#).unwrap(), Some(vec![]));
    }

    #[test]
    fn rejects_unknown_shapes_without_echoing_content() {
        let err = parse(r#"{"tags":"private-tag"}"#).unwrap_err();
        assert!(err.contains("expected an array, got a string"), "{err}");
        assert!(!err.contains("private-tag"), "{err}");

        let err = parse(r#"{"tags":[{"id":7,"label":"private-tag"}]}"#).unwrap_err();
        assert!(err.contains("tag object at index 0"), "{err}");
        assert!(!err.contains("private-tag"), "{err}");

        let err = parse(r#"{"tags":["ok",42]}"#).unwrap_err();
        assert!(err.contains("index 1"), "{err}");
        assert!(err.contains("got a number"), "{err}");
    }
}
