# `dsc category` definition sync — `def pull/push` + per-category `show`/`get`/`set`

Spec for declarative version-control of a Discourse instance's **category
definitions** (name, slug, colour, permissions, position, description, topic
template, tag rules, …) and an imperative setter for the same fields on a
single category. Goal: make a forum's category structure reproducible from a
file under Git, the way `tag pull/push` already does for the tag taxonomy and
`setting pull/push` does for site settings.

This is distinct from the existing `category pull/push` in
[category-workflow.md](category-workflow.md), which sync **topic content**
*inside* a category (Markdown files with front-matter routing). There is
currently no way in `dsc` to read or write a category's *definition*. The gap
surfaced while configuring a brand-new Discourse (`yorkmusic.org`) from
scratch: the category layout is the product, so it must be version-controlled
and reviewable, not hand-clicked in the admin UI.

## Motivation

I am standing up `yorkmusic.org` (Discourse 2026.7.0-latest) from a near-vanilla
install. The forum's whole information architecture is its category set — seven
"X looking for Y" categories with curated colours, descriptions, topic
templates, tag rules, and per-category read permissions. The canonical source of
truth for the *site* lives in this repo (a `settings.yaml` snapshot, a
`tags.yaml` taxonomy) so that the forum is reproducible and changes are
reviewable in Git. I want the **categories** to live the same way:

```
dsc category def pull yorkmusic categories.yaml   # snapshot every category def → file
# edit + commit
dsc category def push yorkmusic categories.yaml   # apply file → server (upsert; --dry-run)
```

Today `dsc category` cannot do this. `category list -f json` returns a
`CategoryInfo` with only `name/slug/color/text_color/id/parent_category_id/
subcategory_list` — **no description, no permissions, no position, no topic
template, no tag rules**. `create_category` only sends `name/slug/color/
text_color`; there is **no `update_category`** at all. The only definition
fields reachable from `dsc` are the four copied by `category copy`. Everything
else (description, permissions, position, topic template, allowed tags, …)
requires the admin UI or raw `curl PUT /categories/{id}.json`.

Alongside the file form I need an imperative one-field setter, because editing a
single category's topic template or description is a frequent, small operation
that shouldn't require rewriting a 7-category YAML file and pushing the whole
thing — exactly as `dsc setting set` coexists with `dsc setting push`.

## Current state (as of 2026-07-01)

Tested against `yorkmusic.org` (Discourse 2026.7.0-latest). Code refs:

- `src/api/models.rs` — `CategoryInfo`:

  ```rust
  pub struct CategoryInfo {
      pub name: String,
      pub slug: String,
      #[serde(default)] pub color: Option<String>,
      #[serde(default)] pub text_color: Option<String>,
      pub id: Option<u64>,
      #[serde(default)] pub subcategory_list: Vec<CategoryInfo>,
      #[serde(default)] pub parent_category_id: Option<u64>,
  }
  ```

  No `description`, `position`, `read_restricted`, `group_permissions`,
  `topic_template`, `allowed_tags`, `allowed_tag_groups`, etc.

- `src/api/categories.rs` — `create_category()` sends only
  `name/slug/color/text_color`. There is **no** `update_category()` and **no**
  fetch of category *definitions* with permissions; `fetch_categories()` hits
  `/categories.json?include_subcategories=true` + `/site.json`, neither of
  which carries `group_permissions` (confirmed: `group_permissions: null` for
  the restricted `sysadmin` category in the response below).

- `src/commands/category.rs` — `category list -v` prints only `id - name`
  regardless of `-v`; `category list -f json` serialises the sparse
  `CategoryInfo` above. `category pull/push` operate on **topics**, not
  definitions (see category-workflow.md).

So the whole definition surface — the part of a category that makes it what it
is — is outside `dsc` today.

## Proposed CLI surface

Two related subcommand families under `dsc category`, mirroring the
`setting` (file = `pull/push`, single = `get/set`) split.

### Declarative (file, all categories)

```text
dsc category def pull <DISCOURSE> [LOCAL_PATH]   # write every category definition → file
dsc category def push <DISCOURSE> <LOCAL_PATH>    # apply file → server
```

- `def pull` serialises **all** category definitions to one file. The whole
  category set is the unit of version control (a handful of interdependent
  categories with parent/child + position ordering), so a single document is
  correct — same call as `tag pull`.
- Default `LOCAL_PATH`: `categories.yaml`. Format inferred from extension
  (`.yaml`/`.yml` default, `.json` supported). Reuse the serializers behind
  `category list -f`.
- Emit **definitions only** — exclude usage counts (`topic_count`,
  `post_count`, `topics_day/week/month/...`), `can_edit`, `notification_level`,
  and server-only booleans, so repeated pulls are diff-clean. Include `id`
  (see Match key) — it's server-specific but stable for a given install and
  makes renames safe (see Backward compatibility).
- `def push` reconciles server state toward the file.
  - **Default: upsert** — create missing categories, update changed ones, never
    delete. Match by `id` when present (stable), else by `slug`, else by
    `name`.
  - `--prune`: **off by default and dangerous.** A category deletion either
    deletes its topics or force-moves them to a target category; `--prune` must
    require an explicit `--prune-categories` confirmation flag, print the
    topic count that would be affected per category in `--dry-run`, and refuse
    without a `--move-to <category>` for any non-empty category. See Phasing —
    prune is Phase 3 (nice to have), not Phase 1.
  - Honour global `-n/--dry-run`: print the plan with `~` (change), `=`
    (unchanged), `+` (create), `-` (would-delete, prune only) sigils, exactly
    like `setting push --dry-run` / `tag push --dry-run`.
  - Idempotent: a push with no file change is a no-op (normalised compare; no
    spurious PUTs).
  - Reject unknown top-level and category-entry fields. A declarative file must
    fail loudly rather than silently ignoring a misspelt or unsupported setting.
  - Dry-run lists the exact definition fields that differ for every planned
    update, without dumping potentially long descriptions or templates.

### Imperative (single category, one field)

Verbs mirror the rest of `dsc`: `show` (like `theme show`) for the whole object,
`get`/`set` (like `setting get`/`setting set`) for one field. No `settings`
sub-noun — that would clash with the singular top-level `dsc setting` and add a
layer `setting get`/`set` don't have.

```text
dsc category show <DISCOURSE> <CATEGORY>                     # show all definition fields for one category
dsc category get  <DISCOURSE> <CATEGORY> <FIELD>            # print one field's value
dsc category set  <DISCOURSE> <CATEGORY> <FIELD> <VALUE>    # set one field
```

- `<CATEGORY>` resolves by `id`, `slug`, or `name` (reuse `resolve_category_id`).
- `<FIELD>` is one of the definition keys below (e.g. `description`,
  `topic_template`, `color`, `text_color`, `position`, `read_restricted`,
  `parent_category_id`, `allowed_tags`, `allowed_tag_groups`,
  `minimum_required_tags`, `default_view`, `sort_order`,
  `subcategory_list_style`, …). Unknown field → error listing valid fields.
- `set` is a single `PUT /categories/{id}.json` with the one field. `--dry-run`
  prints the planned payload. For list-typed fields (`allowed_tags`,
  `allowed_tag_groups`), `<VALUE>` is comma-separated and replaces the list;
  mutually exclusive `--append`/`--remove` flags edit the list additively
  instead.
- `set permissions` is the one composite field: value is a map string like
  `everyone:full,staff:full` or, for restricted categories,
  `staff:full` (staff-only). Levels: `full` | `create_post` | `readonly`.
  This is the same representation used in the file (see schema). Setting
  permissions implies `read_restricted=true` when any group other than
  `everyone` is granted, mirroring the admin UI behaviour.

Both families hit the same Discourse endpoints; `category set` is the
convenience face of the same writes `def push` does in bulk.

## File schema (the contract)

`categories.yaml` is the contract: `def pull` emits this shape, `def push`
accepts it. Field names mirror the Discourse category object (snake_case) so the
mapping is mechanical; a few are normalised for human editing (see notes).

```yaml
version: 1

categories:
  - name: Bands looking for musicians
    id: 5                      # optional; server-specific. Present from `def pull`. On push, match by id when present (makes renames safe). Portable setups may omit it.
    slug: bands-looking-for-musicians   # optional; defaults to slugify(name). Changing slug is a rename (see Backward compatibility).
    color: "BF1E2E"            # 6-hex, no '#'
    text_color: "FFFFFF"
    position: 1               # 0-based ordering within the parent (or top level)
    parent: null               # parent category slug (or name), or null for top-level
    read_restricted: false     # false = public; true = access by permission only

    description: |             # plain text shown under the category title (supports a subset of markdown)
      Watch if you're a musician wanting to join or play with a band.
      Posted by bands/projects with a vacancy.

    topic_template: |          # optional; the default body pre-filled in the composer for new topics here
      **Band:** ...
      **Genre:** [tag]
      **Role needed:** [tag]
      **Paid / unpaid:**
      **Links:**

    permissions:                # optional; map group_name -> level. Omit entirely = "default".
      everyone: full            #   levels: full | create_post | readonly
      # staff: full              #   a restricted, staff-only category = {staff: full} (and read_restricted: true)

    allowed_tags: [guitarist, bassist, drummer, vocalist]   # optional; restricts the tag picker to these
    allowed_tag_groups: [Role, Genre]                        # optional; tag groups whose members are pickable here
    minimum_required_tags: 1                                 # optional; topics must carry >= N tags
    # required_tag_groups:                                   # optional; list of {name, min_count} (Phase 2)
    # category_types: [support]                              # optional; extra enabled category type IDs (Phase 2; `discussion` is implicit)
    # custom_fields:                                         # optional; complete map, omitted = leave untouched
    #   enable_accepted_answers: "true"

    # Optional display/ordering knobs (omit to keep server defaults):
    sort_order: null
    default_view: null
    subcategory_list_style: rows_with_featured_topics
    num_featured_topics: 3
    show_subcategory_list: false
    all_topics_wiki: false
    default_latest_period_days: null
```

### Permission representation

`permissions` is a map `group_name -> level`. This is a deliberate normalisation:
the API returns `group_permissions` as an array of
`{group_id, group_name, permission_type}` where `permission_type` is an integer
(1=full, 2=create_post, 3=readonly); the file uses the readable map form.

- `read_restricted: false` + `permissions` omitted (or `everyone: full`) =
  public, anyone can read/post (the default public category).
- `read_restricted: true` + `permissions: {staff: full}` = staff-only (admins +
  moderators). The restricted `sysadmin` category in the reference response below
  is exactly this case.
- Any other group name resolves against the forum's group list; an unknown group
  name on push → error (do not silently create groups).

### Match key & renames

- **Primary match key on push = `id` when present.** A name or slug change with a
  stable `id` is a safe `PUT /categories/{id}` (renames the category, keeps its
  topics). This is the recommended workflow: keep `id` in the file (it's what
  `def pull` writes).
- **Without `id`**, match by `slug`, then `name`. A name change without `id` is
  indistinguishable from delete+create and would orphan every topic in the
  category — so without `id`, name/slug changes MUST go through a dedicated
  `dsc category rename <discourse> <category> <new-name>` (Phase 2, mirrors
  `tag rename`; uses `PUT /categories/{id}` by resolved id, preserves topics).
  `def push --dry-run` must warn loudly when a no-`id` entry's name doesn't
  match any existing category ("would CREATE — if you meant to rename, use
  `dsc category rename` to preserve topics").

## Reference: API calls observed in the field

Captured against `yorkmusic.org` (Discourse 2026.7.0-latest), admin-scope API
key, 2026-07-01. Nothing sensitive in category objects (no secrets); shown
unedited.

### Read definitions with permissions

```
GET /categories.json?show_permissions=true&include_subcategories=true
Api-Key: <redacted>
Api-Username: pacharanero

→ 200 OK
{
  "category_list": {
    "categories": [ { /* one object per category — full field set below */ } ]
  }
}
```

A single category object (`id: 4`, "General") — the definition-relevant keys:

```json
{
  "id": 4,
  "name": "General",
  "slug": "general",
  "color": "25AAE2",
  "text_color": "FFFFFF",
  "position": 3,
  "read_restricted": false,
  "parent_category_id": null,
  "description": "Create topics here that don't fit into any other existing category.",
  "description_excerpt": "Create topics here that don't fit into any other existing category.",
  "topic_template": null,
  "group_permissions": null,
  "allowed_tags": null,
  "allowed_tag_groups": null,
  "required_tag_groups": null,
  "sort_order": null,
  "show_subcategory_list": false,
  "num_featured_topics": 3,
  "default_view": null,
  "all_topics_wiki": null,
  "subcategory_list_style": "rows_with_featured_topics",
  "permission": 1,
  "default_latest_period_days": null,
  "minimum_required_trust_level": null,
  "min_personal_message_trust_level": null,
  "mailinglist_mirror": null
}
```

The restricted `sysadmin` category (`id: 3`) — note `group_permissions` is
**null even for a restricted category** through this endpoint (the staff-only
default is encoded by `read_restricted: true` + the absence of any other group
permission):

```json
{ "id": 3, "name": "sysadmin", "read_restricted": true, "permission": 1, "group_permissions": null }
```

All keys present on a category object (for completeness, to scope what `def pull`
should keep vs drop):

```
id, name, color, text_color, style_type, icon, emoji, slug, topic_count,
post_count, position, description, description_text, description_excerpt,
topic_url, read_restricted, permission, notification_level, can_edit,
topic_template, topic_title_placeholder, has_children, subcategory_count,
sort_order, sort_ascending, show_subcategory_list, num_featured_topics,
default_view, subcategory_list_style, default_top_period, default_list_filter,
minimum_required_tags, navigate_to_first_post_after_read, custom_fields,
topics_day, topics_week, topics_month, topics_year, topics_all_time,
subcategory_ids, sort_topics_by_event_start_date, disable_topic_resorting,
create_as_post_voting_default, only_post_voting_in_this_category,
uploaded_logo, uploaded_logo_dark, uploaded_background, uploaded_background_dark, topics
```

`def pull` keeps the definition keys in the schema above and drops:
`topic_count`, `post_count`, `topics_day/week/month/year/all_time`,
`topics`, `can_edit`, `notification_level`, `topic_url`, `description_excerpt`
(derived from description), `description_text` (rendered), `subcategory_ids`
(derived from `parent`), `topic_title_placeholder` (Phase 2), `style_type/icon/
emoji/uploaded_logo*/uploaded_background*` (Phase 2 — asset upload is its own
problem), `custom_fields` (Phase 2).

Note: `/c/{id}.json` returns the **topic list**, not the category definition —
do not use it for definitions. The categories list endpoint is the right read.

### Create / update (write)

```
POST /categories.json           # create
PUT  /categories/{id}.json      # update (the one that is entirely missing from dsc today)
Api-Key: <redacted>
Api-Username: <admin>
Content-Type: application/x-www-form-urlencoded

Form params (all optional except name on create):
  name, slug, color, text_color, parent_category_id,
  description, topic_template, position, read_restricted,
  permissions[<group_name>] = full | create_post | readonly,   # one per group
  allowed_tags[]=, allowed_tag_groups[]=,
  minimum_required_tags, required_tag_groups[][name]= / required_tag_groups[][min_count]=,
  category_types[]=,
  sort_order, default_view, subcategory_list_style, num_featured_topics,
  show_subcategory_list, all_topics_wiki, default_latest_period_days,
  minimum_required_trust_level, min_personal_message_trust_level, mailinglist_mirror
```

`permissions[<group_name>]` is the documented write form (a hash keyed by group
name). Equivalent JSON `group_permissions: [{group_name, permission_type}]` is
also accepted; the form form is simpler and matches `create_category`'s existing
`vec![("name", …)]` pattern.

## Phasing

### Phase 1 — blocking (unblocks yorkmusic.org config-as-code)

> **Status: implemented (unreleased).** All Phase 1 items below, in
> `src/commands/category_def.rs` + `src/api/categories.rs`/`models.rs`.
> Verified end-to-end on koloki-demo: pull, show, get, reversible set, and def
> push (create with rename-warning, update, and idempotent `= unchanged` on a
> full pull→push→pull). Unit-tested (`plan_push`, changed-field comparison, permission
> round-trip, single-field param building).
>
> Two findings baked into the implementation:
> - **`description` is read from `description_text`.** The API's `description`
>   is the *cooked* HTML excerpt of the category's auto-created "About" topic and
>   settles asynchronously after create, so reading it broke idempotency (raw
>   file text vs `<p>…</p>`). Reading the plain `description_text` and writing
>   `description` makes the round-trip clean.
> - **Parent-in-same-push limitation.** `parent` is resolved against categories
>   that already exist on the server; a brand-new parent created in the *same*
>   push isn't yet resolvable. Create the parent first (or run push twice). A
>   two-pass ordering is deferred to Phase 2.
> - **Description terminal newlines are normalised.** YAML literal blocks end in
>   a newline while Discourse strips it from category descriptions. Normalising
>   this boundary prevents a perpetual update plan for otherwise identical text.

- [x] `dsc category def pull <discourse> [categories.yaml]` — reads
  `/categories.json?show_permissions=true&include_subcategories=true`, emits the
  file schema above, definitions only, stable-sorted (position, then name).
- [x] `dsc category def push <discourse> <categories.yaml>` — upsert only (no
  prune); creates missing, updates changed by `id`/`slug`/`name`; honours
  `--dry-run` with `+`/`~`/`=` sigils and a loud rename warning for a no-`id`
  no-match entry; idempotent. Maps `permissions` map → `permissions[group]=level`
  (integer) form params; maps `parent` slug → `parent_category_id`.
- [x] `dsc category set <discourse> <category> <field> <value>` — single
  `PUT /categories/{id}` for the one field (the topic_template + description
  case that prompted this spec); `--dry-run` prints the payload.
- [x] `dsc category get <discourse> <category> <field>` and `dsc category show
  <discourse> <category>` — read from the categories list endpoint; `get` prints
  one field, `show` prints all definition fields (both honour `--format`).
- [x] Added a `CategoryDefinition` model with the definition fields and
  `update_category()` / `create_category_def()` / `fetch_category_definitions()`
  to `src/api/categories.rs`.
- [x] `def push` rejects unknown YAML keys and dry-run names every changed field,
  so incomplete schemas such as `solved_enabled` cannot be silently ignored.

### Phase 2 — iteration ergonomics

- [x] `dsc category rename <discourse> <category> <new-name>` — safe rename via
  `PUT /categories/{id}` (preserves topics); mirrors `tag rename`. The
  recommended path for no-`id` files. **Status: implemented (unreleased).**
  Resolves `<category>` by id/slug/name (same as `show`/`get`/`set`); rejects
  an unresolvable category, an empty/identical new name, and a new name
  already used by a different category; `--dry-run` prints the planned
  rename. `src/commands/category_def.rs` (`plan_rename`, `category_rename`);
  unit-tested (no live test, matching `tag rename`'s coverage).
- [x] `--append` / `--remove` for list fields on `category set` (`allowed_tags`,
  `allowed_tag_groups`). **Status: implemented (unreleased).** Mutually
  exclusive flags on `category set`; merge against the field's current
  server-side value (dedupe on append, no-op on removing an absent item) and
  send the resulting list as the field's `PUT` params, same as a full
  replace. `src/commands/category_def.rs` (`merge_list_field`,
  `list_field_edit_params`); unit-tested (no live test needed - pure list
  merge logic).
- [x] `required_tag_groups` round-trip (list of `{name, min_count}`). **Status: implemented (unreleased).** Reads the category-list response and writes `required_tag_groups[][name]` / `required_tag_groups[][min_count]` form parameters. `category set` accepts `group:min_count,...` and an empty value clears the rules.
- [x] `category_types` round-trip. **Status: implemented (unreleased).** Reads the stable type IDs beyond the built-in `discussion` type from the category-list response and writes `category_types[]` form parameters. `category set` accepts a comma-separated type list. The modern Discourse support category type is
  needed to enable accepted answers for York Music's Marketplace; it supersedes
  the invalid category-level `solved_enabled` field.
- [x] `parent` resolution by name as well as slug; validation that the parent
  exists before push (clear error instead of a 4xx from the API). **Status:
  implemented (unreleased).** `def push` (including `--dry-run`) now
  validates every file entry's `parent` reference against the server's
  current categories, by slug first then by name, before any writes happen;
  an unresolvable reference fails with one error naming every offending
  entry, rather than a 4xx partway through applying earlier entries.
  `category set parent` resolves the same way. `src/commands/category_def.rs`
  (`resolve_parent_id`, `validate_parents`); unit-tested.
- [x] `custom_fields` round-trip. **Status: implemented (unreleased).** Reads each category's complete custom-field map from `/c/{id}/show.json`; applies changes via JSON `PUT /categories/{id}.json`, using nulls to remove keys omitted from a specified file map. `category set` accepts a complete scalar-valued JSON object. Object and array values are rejected because Discourse stringifies them. Verified reversibly on koloki-demo.
- [ ] `topic_title_placeholder`, logo/background asset fields, `icon`/`emoji` — surface as `category set` fields once the asset-upload path is decided.

### Phase 3 — nice to have

- [ ] `--prune` for `def push` with hard guardrails (see above): explicit
  `--prune-categories` flag, `--move-to <category>` required for non-empty
  categories, topic-count disclosure in `--dry-run`, refuse if the file's entry
  count drops to zero. Deleting categories is destructive and rare; ship only
  if a real need appears.
- [x] `dsc category diff <discourse-a> <category-a> <discourse-b> <category-b>`
  — compare two explicitly selected live category definitions. **Status:
  implemented (unreleased).** Each category resolves independently by
  id/slug/name, so no cross-forum identity inference is needed and categories
  with different names/slugs can be compared deliberately. Reports only
  differing stable definition fields; excludes server-specific IDs and
  volatile usage counts. Text output is human-readable; JSON/YAML retain native
  booleans, numbers, lists, and permission maps. `src/commands/category_def.rs`
  (`category_diff`, `diff_entries`, `load_category_diff_side`); unit-tested and
  smoke-tested against the disposable live demo forum.
- [ ] Cross-forum `category def copy` (today `category copy` sends only the 4
  basic fields; a full-definition copy falls out of pull→push once `id` is
  stripped/portability is handled — but see Out of scope).

## Backward compatibility

- No existing command changes. `category list/copy/pull/push` keep their current
  meaning (topics for pull/push). New subcommands `def` and `show`/`get`/`set`
  are added under `category`.
- `category list -f json` currently serialises the sparse `CategoryInfo`. Adding
  fields to that struct is additive (existing consumers tolerant of new keys);
  but to avoid changing `category list`'s output shape, prefer a separate
  `CategoryDefinition` model used by `def pull`/`show`/`get`/`set` only, and leave
  `category list` as-is. (If `CategoryInfo` is extended instead, the extra
  fields are `#[serde(default)]` so parsing the old sparse `/site.json` payload
  still works.)
- `id` in the file is new and optional; push treats its absence as name/slug
  matching (the tag-sync model). Existing hand-written category files (none yet)
  need no `id`.
- Renames: with `id` present, a name change is a safe in-place update (no
  behaviour change for users who never rename). Without `id`, a name change now
  errors/warns rather than silently delete-creating — this is *more* correct
  than today's `category push` topic behaviour (which silently created
  duplicates; see category-workflow Gap 3) and is the whole point of the match-key
  design.

## Out of scope

- **Topic content** — `category pull/push` (topics) is separate; see
  category-workflow.md. This spec is about definitions only.
- **Creating or managing groups.** Permissions reference groups by name; an
  unknown group is an error, not an implicit group creation. Group management is
  its own surface (`dsc group`).
- **Asset upload** (category logos/backgrounds) — file upload is a separate
  problem (`dsc upload` + a reference); `def pull` writes `uploaded_logo` URLs
  only as read-only metadata in Phase 2, not as settable file content.
- **Cross-forum portability** of `id`. The file is per-install; `id` is
  server-specific and not meaningful across forums. A portable "template" form
  (no `id`, match by slug) is supported but renames then need `category rename`.
- **Category deletion by default.** Prune is Phase 3 and behind guardrails; the
  default is the safe upsert everyone expects from `tag push` / `setting push`.
