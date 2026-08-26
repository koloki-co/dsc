# Performance audit

Audit started 2026-08-10 against merged `main` at `db57692f177c33368cd2a9d651cccc4909c29050` (`v0.14.0` development tree). The source review runs from an isolated clean worktree so unrelated local category-definition changes do not affect the evidence.

## Scope

This audit looks for avoidable latency, unnecessary remote requests, accidental sleeps or retries, serial work that should be concurrent, repeated filesystem or subprocess work, unbounded memory growth, and algorithmic inefficiencies. It begins with simple mechanical mistakes and then moves into command-level and cross-command architecture.

The audit does not treat every blocking operation as a defect. `dsc` is a synchronous administration CLI, so a straightforward blocking request is often the clearest correct design. A finding is recorded only when the implementation adds avoidable work, scales poorly in a realistic workflow, or lacks a bound that can make latency or resource use surprising.

## Executive summary

The audit found **31 performance issues: 5 high, 16 medium, and 10 low**. The dominant problem is not CPU efficiency. It is avoidable remote work: oversized request sets, serial N+1 calls, fleet operations that do not use the good worker-pool pattern already present in `config check`, and subprocess paths without appropriate bounds.

The best immediate returns are P1 (analytics request planning), P2 (tag pull/push idempotency), P3 (the broken update scheduler), P4 (SSH liveness defaults), P5 (readiness polling), and P7 (one update-log scan). P1, P2, P5, and P7 are relatively contained changes. P3 and P4 should follow promptly because they affect long-running privileged operations. P6 is deferred: the per-forum fetch is intentional because the `latest` branch moves during a multi-hour update.

The largest scale risks are P9 (SAR), P10 (category content sync), P12 (fleet audits), and P13 (backup-health S3 aggregation). They need bounded concurrency or streaming rather than isolated micro-optimizations.

## Severity

- **High** - likely to make a supported command unusable or excessively expensive at realistic forum/fleet sizes.
- **Medium** - user-visible avoidable latency or resource growth in a common or important workflow.
- **Low** - measurable inefficiency with limited present-day impact, or a scaling risk worth fixing alongside nearby work.

## Findings

### P1 - High - Analytics fetches reports excluded by the selected section and refetches most comparison data

**Evidence:** `src/commands/analytics.rs:34-46,71-80,96,274-292,353-375`; `src/api/reports.rs:5-11,58-65,102-105`.

`populate_cache` always builds the Cartesian product of every requested window and all nine report IDs. The section filter is applied only after those requests complete. Growth uses two reports, activity uses four, and health uses four, but each single-window section currently fetches all nine. Comparison mode doubles the windows and therefore doubles every request even though each report response already includes `prev_data` for the immediately preceding equal-length window.

Concrete request counts show the scale: `--section growth` makes 9 requests instead of 2, and `--section growth --compare` makes 18 instead of 2. All-sections comparison makes 18 requests; it should need the 9 current-window reports plus at most one prior-window request for `time_to_first_response`, whose previous average is not represented by `prev_data`.

**Impact:** This directly adds request waves, server work, and rate-limit exposure to a command whose source documents an empirically chosen concurrency ceiling because higher fan-out already caused HTTP 429 responses.

**Recommendation:** Derive the required report-ID set from `SectionFilter` before populating the cache. In comparison mode, consume `AdminReport::previous_total()` and issue a previous-window request only for a metric whose required scalar is unavailable in the current response. Add deterministic request-budget tests for every section/mode combination.

### P2 - High - Tag synchronization performs redundant serial reads and unchanged writes

**Evidence:** `src/api/tags.rs:11-22,52-82`; `src/commands/tag.rs:238-258,443-488,532-545,689-704`; `tests/dry-run-mutation-test.rs:110-111`; `spec/commands/app-environment.md:45`.

`tag pull` first retrieves `TagInfo` objects from `/tags.json`; `TagInfo` already carries `description`, and the test fixture includes it. The command ignores that field and serially calls `/tag/{name}.json` for every tag. At minimum, every tag whose list result contains a description incurs a provably redundant request. A real `tag pull` is documented as hitting the admin rate limit and waiting 53 seconds between retries.

The inverse path also wastes work. `tag push` discards server descriptions and retains names only, so `plan_tags` schedules every non-null desired description. A completely unchanged taxonomy with `D` described tags therefore issues `D` PUT requests. It also unconditionally lists all tags again after group reconciliation, even when no group was created or updated.

**Impact:** Pull is a serial `1 + N` request pattern on a demonstrated rate-limit-sensitive path. Push violates the declarative sync contract by rewriting unchanged values, causing avoidable latency, server work, audit noise, and rate-limit pressure.

**Recommendation:** Use `TagInfo.description` directly and use a compatibility fallback detail request only when the list field is absent. Preserve server descriptions during push planning and compare before scheduling writes. Re-list tags only when group creation or update could have materialized tags. Add request-budget tests for unchanged pull/push fixtures.

### P3 - High - `update all --parallel` is not completion-driven and loses effective concurrency

**Evidence:** `src/commands/update.rs:87-110`.

The scheduler fills a vector of thread handles. Once it reaches the requested width, it pops and joins the most recently started thread before starting one more. It does not detect whichever worker finishes first. Completed older handles remain in the vector and continue occupying nominal slots, so after the initial batch, later updates commonly run one at a time behind the most recently spawned update.

The error path is also surprising: propagating one joined error drops the remaining `JoinHandle`s, which detaches rather than cancels their threads. The command can return while other SSH updates continue in the background.

**Impact:** Update durations naturally vary by host. With width 3, two quick workers can sit completed and unused while the scheduler waits for a slow third worker; after that, subsequent hosts can be admitted serially. Fleet updates can take hours longer than the requested parallelism implies.

**Recommendation:** Replace the handle stack with the fixed shared-queue worker-pool pattern already used by `config check`, and collect all worker outcomes before returning. Add a delayed fake-worker test proving that width `N` remains work-conserving when jobs finish out of order.

### P4 - High - Shared SSH execution lacks default connection and liveness timeouts

**Evidence:** `src/commands/update.rs:1015-1041,1237-1254,1268-1274`; compare `src/commands/config.rs:240-262`.

The shared SSH command builder used by update, app, plugin, and theme operations sets `BatchMode` and host-key behavior but no `ConnectTimeout`, `ConnectionAttempts`, `ServerAliveInterval`, or `ServerAliveCountMax`. Only the special reboot probe adds `ConnectTimeout=10`; `config check` independently uses a five-second connection timeout.

**Impact:** A black-holed host can hold a command for the operating system's TCP timeout, and a connected but stalled session has no application liveness bound. This can block a serial fleet command indefinitely from the user's perspective and can permanently consume one of the already-imperfect update worker slots.

**Recommendation:** Give the shared SSH builder conservative connection-attempt and server-alive defaults, preserving `DSC_SSH_OPTIONS` as an override. Do not impose a short absolute command deadline on legitimate long rebuilds; use SSH transport liveness settings to distinguish long work from a dead connection.

### P5 - Medium - Update readiness uses coarse fixed sleeps before it tries observable checks

**Evidence:** `src/commands/update.rs:896-952,1445-1466`.

A successful normal reboot sleeps 30 seconds before the first SSH probe and then checks only every 30 seconds. After the optional Discourse rebuild, or even when the rebuild is skipped as current, the workflow sleeps another 15 seconds before the first API check. A normal successful update therefore includes at least 45 seconds of fixed waiting, while coarse polling adds up to almost another interval of detection latency. The later version check already has bounded retries, making the unconditional API sleep partly redundant.

**Impact:** The waits multiply across sequential fleet updates and compound P3. They also make fast hosts no faster while still not proving readiness on slow hosts.

**Recommendation:** Replace fixed readiness sleeps with state-driven polling: detect the reboot transition safely, poll SSH at a shorter bounded interval, and poll a lightweight Discourse endpoint immediately with capped backoff. Preserve overall deadlines and progress messages.

### P6 - Deferred - Per-forum GitHub commit fetch is intentional, not waste

**Evidence:** `src/commands/update.rs:928-945,1469-1510`.

**Decision: not fixing.** The original recommendation was to cache the latest SHA once per update invocation. On review, this is incorrect for a moving target: the `latest` branch tracks Discourse `main` closely and can advance during a multi-hour fleet update. Caching at the start would cause later forums to compare against a stale reference and incorrectly skip rebuilds. The per-forum fetch gives each forum the most current reference point.

A deeper issue exists: the code hardcodes the `stable` branch at `src/commands/update.rs:1478`, so forums running `latest` always compare against the wrong reference and always rebuild. The real fix is making the branch configurable or detecting which branch the forum tracks, which is a design change, not a performance optimization. The duplicate GitHub requests are a minor cost compared to the correctness risk of caching.

### P7 - Medium - Recent-update filtering rereads and reparses the complete log once per forum

**Evidence:** `src/commands/update.rs:177-192`; `src/commands/update_log.rs:132-152,177-185`.

`recent_skip_set` calls `updated_within` for every updatable forum. Each call reads the entire append-only update log, allocates every `LogRecord`, reparses timestamps, and scans for one forum. The preflight is therefore `O(forums * log records)` in both parsing and repeated disk reads, and it becomes slower as the permanent log grows.

**Impact:** This is avoidable startup work before any update begins. On a long-lived large fleet, one command can reread gigabytes cumulatively even if the log itself is modest.

**Recommendation:** Scan the log once with `BufRead`, computing the set or latest successful timestamp by forum during that pass. Reuse the same streaming fold for `update log --latest`, which currently also materializes the full history.

### P8 - Medium - Long SSH commands retain unbounded output and duplicate tail-processing work

**Evidence:** `src/commands/update.rs:1019-1053,1061-1186,1210-1234`.

The streaming runner correctly drains stdout and stderr concurrently, but sends every line through an unbounded channel and appends both complete streams to unbounded strings. Successful OS update and rebuild callers discard this returned output; failures ultimately render only a bounded diagnostic tail. On every line, the runner also reconstructs the complete progress-tail message, including when the progress bar is hidden in non-interactive output. The generic SSH helper uses `Command::output()` and fully buffers both streams for other potentially long operations such as app rebuilds.

**Impact:** Verbose remote commands can consume memory without a bound, multiplied by fleet concurrency. Producer threads can also outrun the consumer and grow the unbounded channel. App rebuilds can appear silent for a long time while accumulating output.

**Recommendation:** Retain fixed-size diagnostic rings per stream and use a bounded channel or direct synchronized tail updates. Skip progress-message reconstruction when progress is hidden. Use the streaming runner for all long SSH operations and capture full output only for commands whose output is intentionally parsed and known to be small.

**Addressed 2026-08-26 (R52):** the streaming runner now retains only the last `MAX_REMOTE_DIAGNOSTIC_LINES` (20) lines per stream in `VecDeque` rings instead of unbounded `String` buffers, uses a bounded `sync_channel(64)` so a slow consumer applies backpressure to the reader threads, and skips progress-message reconstruction when the progress bar is hidden (non-interactive output). The failure path reads from the rings directly. Both production callers (OS update and Discourse rebuild) already discarded the returned `Ok(String)`, so the return type is now `Ok(())`. The generic `run_ssh_command`/`run_ssh_command_named`/`run_ssh_command_combined_named` helpers (which use `Command::output()`) are unchanged for now; the notable remaining concern is `app.rs:240-246` which runs `./launcher rebuild app` through the fully-buffering `run_ssh_command` - routing that through the streaming runner is a follow-up.

### P9 - High - SAR export combines serial N+1 requests with unbounded whole-export retention

**Evidence:** `src/commands/sar.rs:97-135,214-270`.

SAR collection paginates authored actions at roughly ten rows per request and then fetches every post body serially with one additional request per post. Likes are collected in a second pagination walk. With messages enabled, inbox and sent lists are fetched and every unique private-message topic is fetched serially. For `P` posts and `M` private-message threads, request count is approximately `ceil(P / 10) + P + likes pages + message-list pages + M`.

The command also retains all action rows, all raw post bodies in `posts_json`, and then allocates another complete pretty-printed JSON string in `write_json`. The action cap is 100,000, so memory is bounded only at a size that can still be enormous.

**Impact:** Even at 100 ms per request, 1,000 posts add more than 100 seconds of serialized network latency before server processing and rate-limit waits. Large exports can consume hundreds of megabytes because raw content coexists in model values and serialized output.

**Recommendation:** Combine action types in one pagination walk and partition locally. Fetch independent post and message details through a small bounded worker pool. Stream JSON elements directly to a private atomic output with `serde_json::Serializer` and drop each raw body after its JSON and Markdown representations have been written.

**Addressed 2026-08-26 (R52):** post body fetches now run through a bounded worker pool (`fetch_post_raws_parallel`, 6 workers) so one rate-limited response does not block the rest. `posts.json` is streamed to disk element-by-element via `serde_json::to_writer` rather than building a `Vec<Value>` and calling `to_string_pretty`, so memory holds at most one post body at a time. `post_actions` and the raw-body map are explicitly dropped before the likes walk begins, and `likes`/`likes_json` are dropped after `activity.json` is written. The two action-type pagination walks (posts then likes) remain serial since they use different filter sets; combining them is a follow-up. PM topic fetches remain serial; parallelising them is a follow-up.

### P10 - Medium - Category content sync serializes topic reads and repeats local parsing and linear scans

**Evidence:** `src/commands/category.rs:114-167,248-340,580-650`.

Category pull fetches every topic detail one at a time before writing its file. Category push similarly fetches each matched remote topic serially while planning. Under `--rewrite-links`, `local_topic_links` reads and parses every Markdown file, then the main planning loop reads and parses each file again. Topic routing and category-membership checks repeatedly linearly scan the topic vector; legacy snapshots without `topic_id` can therefore approach `O(files * topics)` comparisons.

**Impact:** Categories containing hundreds or thousands of topics pay the sum of all request round trips. Large local snapshots add duplicate disk reads and potentially tens or hundreds of millions of comparisons.

**Recommendation:** Use bounded concurrency for the read-only topic-detail phase while keeping mutations serialized. Build ID, slug, and normalized-title indexes once. Parse each local file once and retain the metadata/body needed by link discovery and planning.

**Partially addressed 2026-08-26 (R52):** topic-ID membership checks in the planning loop are now O(1) via a `HashSet<u64>` index built once after the category fetch, replacing the `topics.iter().any(|t| t.id == id)` linear scan per file. `local_topic_links` now builds a `HashMap<u64, &TopicSummary>` index for its ID lookup, replacing a second linear scan per file. Bounded concurrency for the serial topic-detail fetches in `category pull` and `category push` planning, and the double local-file parse under `--rewrite-links`, remain outstanding.

### P11 - Medium - Emoji downloads are serial, and inline rendering has no request timeout

**Evidence:** `src/commands/emoji.rs:31-104,347-379`.

Emoji pull downloads every image serially with a 30-second timeout per image. One hundred unreachable URLs can therefore consume about 50 minutes. Inline rendering also downloads serially but constructs a default HTTP client with no total request timeout, so one stalled image can block the command indefinitely from the user's perspective. Both paths fully buffer each image before writing or base64 encoding it.

**Impact:** Runtime is the sum of CDN latency and failures. The unbounded inline wait is particularly visible because the command produces no later rows while one URL is stalled.

**Recommendation:** Use a small bounded download pool, apply explicit connect and total timeouts to both paths, cap accepted image bytes, and restore deterministic display/write order after worker completion.

**Addressed 2026-08-26 (R52):** the inline rendering client now uses the same 30-second timeout as the download client, so one stalled URL can no longer block the command indefinitely. The download path now uses a bounded worker pool (4 workers) with `std::thread::scope`, and each accepted image is capped at 8 MiB so a hostile or misconfigured CDN cannot drive memory through a single oversized response. Progress remains deterministic (skipped files are counted before the pool starts). P16 (multipart upload streaming and endpoint caching) remains outstanding.

### P12 - Medium - Independent fleet audits are serial and withhold results until all hosts finish

**Evidence:** `src/commands/setting.rs:41-62,115-155`; `src/api/settings.rs:93-120`; `src/commands/app.rs:108-160`; `src/commands/backup.rs:221-271,317-397`.

`setting audit`, `app env audit`, and the forum-configuration phase of `backup health` visit forums one at a time. Each setting lookup downloads the complete site-settings catalogue because that is the available API shape. Audit commands collect every row before rendering, so completed results remain invisible behind one slow host. Cross-forum setting writes are also serial, although serialization may be the preferred safe default for mutations.

**Impact:** Wall time is the sum of all forum latency. One unreachable HTTP host can consume the request timeout and retry waits before the next forum starts; one stalled SSH host can block `app env audit` under P4.

**Recommendation:** Extract a shared bounded fleet executor based on `config check`, with deterministic result ordering and fastest-first text progress. Use it by default for read-only audits. Consider explicit bounded parallelism for writes only after preserving complete dry-run plans and clear per-forum outcomes.

**Addressed 2026-08-26 (R52):** a shared `run_fleet` executor in `src/commands/common.rs` now backs `setting audit`, `app env audit`, and `search all`, using `std::thread::scope` with a bounded shared-queue worker pool. `fleet_worker_count` centralises the width policy with an absolute ceiling of 32 (P28). The duplicated `matches_tag_filter` implementations are retired in favour of `selected_discourses`. The backup-health configuration phase remains serial for now; converting it is a follow-up since it has a two-phase structure (config discovery then S3 scan) that needs the shared executor applied to each phase separately.

### P13 - Medium - Backup health materializes whole S3 inventories and scans buckets serially

**Evidence:** `src/commands/backup.rs:226-271,405-469`.

After serial forum configuration discovery, each distinct S3 bucket is scanned serially. The AWS CLI is invoked without `--no-paginate`, so it may retrieve the full service result before Rust receives output. Rust buffers the command's complete JSON stdout, parses a full `Value`, copies every object into another vector, and retains all objects until separate count, sum, and newest-archive passes finish. Only object count, total bytes, and one newest archive are needed.

**Impact:** Runtime is additive across buckets, and large buckets can occupy hundreds of megabytes across AWS CLI output, Rust stdout bytes, the JSON DOM, and the copied object vector. No rows are emitted until all scans finish.

**Recommendation:** Use explicit one-service-page AWS calls, fold count/bytes/newest archive as each page arrives, and discard the page immediately. Scan independent buckets through a bounded pool and preserve bucket deduplication, which is already implemented correctly.

### P14 - Medium - S3 setup polling repeatedly performs a full recursive listing

**Evidence:** `src/commands/backup_s3.rs:240-271`.

The test-backup verifier starts `aws s3 ls --recursive` every ten seconds for up to about three minutes. Every poll can list and buffer the entire bucket. The deadline is checked only between subprocesses, so one hung AWS process can exceed it indefinitely, and the command provides no attempt or elapsed-time progress while waiting.

**Impact:** Verification repeats process startup and increasingly expensive bucket listings up to about eighteen times. A pre-existing large bucket makes every poll slower even though the command only needs evidence of one newly triggered archive.

**Recommendation:** Record the trigger time and poll a paged `list-objects-v2` query using a narrow prefix or bounded result set, with an explicit subprocess timeout and visible attempt/elapsed progress.

### P15 - Medium - Deleted-topic search performs a serial detail request for every row before applying the query

**Evidence:** `src/api/topics.rs:225-307`; `src/commands/topic.rs:356-403`.

`topic list --deleted <query>` paginates all deleted-topic summaries, fetches every topic detail serially to verify deletion/category state, and only then tests whether the summary matches the query. A query matching one topic still pays one detail request for every deleted topic.

**Impact:** Forums with a long deletion history incur `D` additional serial requests plus list pages. The safety verification is useful, but it is unnecessarily applied to rows that will not be returned.

**Recommendation:** Apply title/slug filtering before detail verification. Preserve verification for candidates and run those independent reads through a small bounded pool.

**Addressed 2026-08-26 (R52):** the query filter (`deleted_topic_matches`) now runs before the detail fetch, so a query matching one topic no longer pays a detail request for every other deleted topic on the page. The safety verification (deleted-state and category check) still runs, but only on candidates that passed the filter. Bounded concurrency for the remaining candidate fetches is a follow-up.

### P16 - Medium - Multipart uploads reread and buffer complete files on retries, and emoji endpoint negotiation repeats per file

**Evidence:** `src/api/uploads.rs:30-58`; `src/api/themes.rs:134-160`; `src/api/emoji.rs:13-83`; `src/commands/emoji.rs:173-201`.

Upload form closures call `fs::read` and create an in-memory multipart part on every attempt. A 429 retry therefore rereads and reallocates the complete upload, potentially six times. Theme bundles and general uploads can be large. Emoji upload also tries up to three endpoint variants, rereading and retransmitting the file for each 404. Bulk upload repeats the same known-failing endpoint probes for every file on older Discourse versions.

**Impact:** Large files multiply memory, disk I/O, and retransmission costs under rate limiting. One hundred emojis on an older endpoint can cause 200-300 upload requests instead of roughly 100.

**Recommendation:** Build multipart bodies from streaming file handles reopened per retry. Cache the first successful emoji upload endpoint for the remainder of the invocation and probe fallbacks only while endpoint capability is unknown.

### P17 - Medium - Version-only metadata stamps always make an unnecessary homepage request

**Evidence:** `src/api/client.rs:191-249`; callers at `src/commands/setting.rs:327` and `src/commands/theme.rs:716`.

`fetch_version_info` always requests `/about.json` and then `/`; the HTML request exists to extract the commit. `fetch_version` delegates to that full method and discards the commit, so setting and theme-setting pulls pay a second serial request and parse the homepage merely to stamp an optional version. Errors are discarded by callers, but timeouts and rate-limit waits occur before `.ok()` can discard them.

**Impact:** Informational metadata can materially delay an otherwise-successful pull when the homepage is slow, protected differently, or rate-limited.

**Recommendation:** Implement version-only lookup as one `/about.json` request. Reserve homepage parsing for callers that explicitly need the commit.

### P18 - Medium - Batch topic deletion performs a display-only GET before every DELETE

**Evidence:** `src/commands/topic.rs:268-317,406-425`.

Every topic ID is fetched before deletion to print its title and post count. Dry-run correctly requires this preflight detail; live deletion passes `required=false`, suppressing fetch errors but still paying their latency. A live batch therefore performs `2N` serial requests.

**Impact:** Large moderation batches take approximately twice the necessary request time and have twice the rate-limit exposure.

**Recommendation:** Fetch topic briefs only for dry-run or an explicit verbose mode. Live deletion can report the successfully deleted ID without an extra GET.

### P19 - Low - Common lookup paths fetch complete catalogues they do not need, sometimes twice

**Evidence:** category copy at `src/commands/category.rs:73-104,451-466` with `src/api/categories.rs:62-90`; group commands at `src/commands/group.rs:49-60,77-87,111-159` with `src/api/groups.rs:34-69`.

Slug-based category copy calls `fetch_categories` to resolve an ID and immediately calls it again to retrieve the category. Each catalogue fetch requests both `/categories.json` and best-effort `/site.json`, yielding four source GETs before creation. Numeric `group info`, `group members`, and `group copy` first list every group to obtain a name even though their API methods try the numeric-ID endpoint first.

**Impact:** These are small per-invocation penalties but common, simple examples of hidden duplicate remote work.

**Recommendation:** Resolve and retain the category object from one catalogue fetch. For numeric groups, try the ID route first and fetch the catalogue/name only after an ID-route 404 requires compatibility fallback.

### P20 - Low - Data Explorer CSV download buffers and copies the complete result

**Evidence:** `src/api/explorer.rs:234-245,302-312`; `src/commands/explorer.rs:129-133`.

The response is fully buffered as `Bytes`, explicitly copied into a `Vec<u8>`, and then copied to an atomic file. Results are row-limited upstream, but cells can still contain large text bodies.

**Impact:** Peak memory is multiple times the CSV size for no semantic benefit.

**Recommendation:** Stream the response directly into `AtomicOutput` using the existing backup-download pattern. Buffer only bounded error bodies for non-success responses.

### P21 - Low - Site-setting parsing clones complete JSON subtrees unnecessarily

**Evidence:** `src/api/settings.rs:55-90,93-120`.

The settings catalogue is first parsed into a `serde_json::Value`. Detailed listing then clones each entry before typed deserialization, while single-setting lookup clones the complete settings array before scanning it. The endpoint's all-settings response is unavoidable with the current API, but these post-parse copies are not.

**Impact:** Large settings payloads coexist as response text, a full JSON DOM, cloned subtrees, and final values. The effect is limited by ordinary catalogue size but is repeated in every forum of setting audits and backup configuration discovery.

**Recommendation:** Deserialize a typed response envelope directly. At minimum, scan the borrowed array for a single setting and avoid `.cloned()`.

### P22 - Low - `user activity` retains an unlimited history before rendering

**Evidence:** `src/commands/user.rs:440-538`.

When neither `--since` nor `--limit` is supplied, the command intentionally requests all available history and retains every action before producing output. Text, Markdown, and CSV could be emitted page by page; JSON and YAML currently allocate another complete serialization after collection.

**Impact:** Highly active long-lived users can cause unbounded command duration and memory growth, with no progress while pages are collected.

**Recommendation:** Stream pages into text/Markdown/CSV renderers and stream a JSON sequence where practical. Preserve all-history semantics but show page/item progress and add a repeated-page or total-budget guard.

### P23 - Low - Per-file atomic durability can dominate bulk pulls on slow filesystems

**Evidence:** `src/utils.rs:114-155,242-259`; bulk callers include category, SAR, and emoji output loops.

Every atomic output flushes and calls `sync_all` before rename. This is a deliberate safety property and should not be removed casually, but hundreds or thousands of small files create the same number of serial durability barriers. Network filesystems and slow journaled storage can make this locally dominant after download concurrency is improved.

**Impact:** Currently network latency often masks the cost. It may become visible after fixing P9-P11 or when operating on remote/cached storage.

**Recommendation:** Measure before changing semantics. If significant, consider a documented bulk durability strategy that writes all temporary files, synchronizes them in a controlled phase, and commits atomically per file, rather than silently weakening crash safety.

### P24 - Low - Config-free commands still resolve and parse configuration

**Evidence:** `src/main.rs:95-131,1558-1568`; `src/commands/completions.rs:40-59`.

After the own-version special case, configuration is resolved and parsed before dispatching commands including completion generation and man-page generation, which do not consume it. Startup also collects all arguments for prechecks before clap reads them again; completion generation constructs the full clap tree again.

**Impact:** This is normally milliseconds, but it makes shell-completion installation and packaging commands dependent on unrelated config size and validity.

**Recommendation:** Dispatch known config-free commands before config resolution, using the existing own-version early path as the pattern. Avoid optimizing the tiny argument copy unless startup profiling shows it matters.

### P25 - Low - Category-definition planning uses repeated linear matching

**Evidence:** `src/commands/category_def.rs:289-325,391-419`.

Each file category scans the server category list by ID, then slug, then name. List comparisons clone and sort both sides on every comparison. Complexity is roughly `O(file categories * server categories)` plus repeated list allocations.

**Impact:** Real forums usually have few categories, so this is not currently a user-visible bottleneck. It is a scale risk for generated multi-forum definitions and a useful benchmark fixture.

**Recommendation:** Build ID/slug/name indexes once and compare normalized sets without repeated cloning if scale measurements justify the extra structure.

### P26 - Low - Several complete-list loops have no total page or item budget

**Evidence:** examples include category pagination in `src/api/categories.rs`, fallback group pagination in `src/api/groups.rs`, deleted-topic pagination in `src/api/topics.rs`, private-message pagination in `src/api/topics.rs`, and all-history user activity in `src/commands/user.rs`.

These paths detect repeated continuation URLs, preventing simple cycles, but a server can return indefinitely many unique continuation paths. Per-request timeouts do not bound total command duration or retained vectors. Complete export is often the intended behavior, so a low arbitrary truncation would also be wrong.

**Impact:** A buggy or unexpectedly large configured forum can make a command consume resources without a command-level ceiling.

**Recommendation:** Add shared configurable page/item budgets with clear errors and explicit `--all` behavior where completeness is intentional. Stream outputs when whole-result retention is unnecessary.

### P27 - Medium - SSH-heavy workflows repeatedly establish independent sessions

**Evidence:** update stages throughout `src/commands/update.rs:751-988`; hardening preflight at `src/commands/harden.rs:156-194` and SSH runner at `src/commands/harden.rs:616-648`.

A typical update starts separate SSH processes for rebuild detection, OS details, disk checks, package update, reboot, readiness probes, Discourse rebuild, cleanup, and final disk usage. Hardening opens four independent read-only preflight sessions before its first mutation and many more afterward. Unless the operator has configured SSH multiplexing externally, every process repeats startup, authentication, key exchange, and TCP latency.

**Impact:** Handshake overhead adds seconds per host and becomes substantial across a fleet. Most stage separation is valuable for diagnostics and safety, so concatenating all remote operations into one opaque shell command would be the wrong optimization.

**Recommendation:** Support safe connection multiplexing or document and validate a `ControlMaster`/`ControlPersist` profile for dsc. Independently combine read-only hardening preflights into one structured remote probe where failure attribution remains clear. Measure before changing mutation-stage boundaries.

### P28 - Low - Explicit parallel widths have no local resource ceiling

**Evidence:** `src/cli.rs:82-100,470-483`; `src/commands/update.rs:234-237`; `src/commands/config.rs:187-193`.

User-supplied update/config worker counts are capped by forum count but not by a defensible absolute maximum or local CPU/file-descriptor budget. Each update worker can add an SSH process and two output-reader threads.

**Impact:** Defaults of 3 and 8 are safe, but `-p 1000` on a sufficiently large fleet can create hundreds or thousands of threads/processes and exhaust local resources.

**Recommendation:** Apply a documented hard ceiling or require an explicit unsafe override above it. A shared fleet executor should own this policy.

### P29 - Low - `list --open` waits for each browser opener process serially

**Evidence:** `src/commands/list.rs:214-219`; `src/commands/common.rs:124-152`.

Each URL launches an opener with `Command::status()` and waits for it to exit before launching the next. Normal platform openers usually detach quickly, but a blocking custom `DSC_BROWSER_OPENER` prevents all later URLs from opening.

**Impact:** Process startup is repeated per forum, and behavior depends on the opener's lifetime semantics.

**Recommendation:** Spawn normal openers without serial waiting, or add a bounded opener mode that collects exit statuses. Preserve useful errors for immediate launch failures.

### P30 - Medium - Bulk import and config tidy discover site titles serially

**Evidence:** `src/commands/import.rs:29-90`; `src/commands/list.rs:72-102`; `src/commands/common.rs:99-121`; `src/api/client.rs:156-189`.

Each imported URL, and each tidy entry missing `fullname`, constructs a temporary client and performs title discovery before moving to the next. Discovery tries `/site.json` and falls back to `/`, so one unreachable URL can pay two connection failures before the next URL starts.

**Impact:** Bulk onboarding runtime is the sum of every site's latency. Fifty unreachable URLs can consume many minutes even with the ten-second connection timeout.

**Recommendation:** Resolve independent titles through a bounded worker pool, then restore input/config order before writing. Continue treating title lookup as best effort.

**Addressed 2026-08-26 (R52):** `fetch_fullnames` in `src/commands/common.rs` runs title discovery through a bounded worker pool (same pattern as `run_fleet`). `import` (text and CSV modes) and `list tidy` now batch URL discovery through it, restoring input order before writing. One unreachable URL no longer blocks the rest.

### P31 - Medium - Some DELETE operations bypass the shared 429 retry path

**Evidence:** `src/api/client.rs:103-115`; direct users at `src/api/tags.rs:136-145,186-199` and `src/api/themes.rs:57-65`.

Most mutations build a fresh request through `send_retrying`, but `DiscourseClient::delete` calls `.send()` directly. Tag and tag-group pruning therefore fails immediately on HTTP 429. A partly completed bulk prune must be rerun, repeating planning and earlier request work, precisely on a path where tag synchronization has already caused real rate limiting.

**Impact:** This does not make successful requests slower, but turns temporary rate pressure into failed commands and costly whole-command retries.

**Recommendation:** Remove the non-retrying delete shortcut and route these callers through `delete_builder` plus `send_retrying`, retaining idempotency and bounded waits.

## Positive observations

- `DiscourseClient` owns a reusable `reqwest::blocking::Client`; ordinary requests within a command reuse its connection pool, and analytics clones share that underlying client rather than rebuilding it.
- Routine HTTP requests have connection and total-request timeouts. HTTP 429 retries and server-directed waits are bounded. The main concern is avoidable request volume triggering those waits, not an infinite retry loop.
- Analytics uses bounded concurrency rather than unbounded request fan-out. P1 concerns the oversized task set, not the worker count.
- `config check` already contains a suitable fixed worker-pool implementation that can be reused conceptually for P3 and other fleet operations.
- Category, deleted-topic, group, private-message, Data Explorer, webhook, and search pagination paths have loop detection or endpoint-specific caps in important places. P26 concerns total budgets, not a complete absence of pagination defenses.
- Full-topic retrieval avoids a per-post N+1 by fetching missing post-stream entries in batches.
- Large backup downloads stream directly into an atomic output rather than buffering the archive; this is the model for P20 and other large downloads.
- Backup health deduplicates shared `(bucket, region)` probes across forums before contacting AWS.
- SSH stdout and stderr are drained concurrently, avoiding pipe deadlock. P8 concerns retention and queue bounds, not pipe handling correctness.
- The release profile inherits Rust's optimized release settings and enables thin LTO. No unusual build-profile choice is suppressing runtime optimization.
- `cargo clippy --all-targets -- -W clippy::perf` completed without warnings at the audited commit.

## Recommended order

### Phase 1 - Request-count and idempotency fixes

1. ~~P2 - remove tag pull N+1 reads and unchanged push writes.~~ (done)
2. ~~P1 - restrict analytics tasks by section and consume `prev_data`.~~ (done)
3. ~~P7 - scan the update log once per command.~~ (done)
4. ~~P17, P18, P19, P21, and P31 - remove straightforward extra requests/copies and normalize DELETE retries.~~ (done)

Add request-budget tests with each change. These tests should assert exact or maximum route counts, not wall-clock time.

### Phase 2 - Long-running orchestration

1. ~~P3 - replace update's handle stack with a real worker pool and collect all outcomes.~~ (done)
2. ~~P4 - apply shared SSH connection/liveness defaults.~~ (done)
3. ~~P5 - replace fixed readiness sleeps with bounded observable polling.~~ (done)
4. P8 - bound SSH output retention and use the streaming runner consistently.
5. P12, P27, P28, and P30 - introduce a shared bounded fleet executor and decide SSH multiplexing policy.

Use delayed fake workers and fake subprocesses to test concurrency deterministically. Do not add flaky elapsed-time assertions to ordinary CI.

### Phase 3 - Bulk streaming and scale

1. P9 - stream SAR serialization and bound detail-fetch concurrency.
2. P10 and P15 - index topic lookups and bound category/deleted-topic detail reads.
3. P11 and P16 - bound and stream image/file transfer, with cached endpoint capability.
4. P13 and P14 - page and fold S3 data without whole-bucket materialization.
5. P20, P22, P23, P25, and P26 - complete lower-priority streaming, durability measurement, indexing, and budget work.

## Method and progress

- [x] Establish a clean baseline at merged `main`.
- [x] Check explicit sleeps, retries, polling, subprocess spawning, and repeated client construction.
- [x] Check serial per-resource network loops and pagination behavior.
- [x] Check repeated parsing, cloning, allocation, and asymptotically poor collection operations.
- [x] Check filesystem buffering and whole-response/whole-file memory use.
- [x] Run available static performance lints and inspect release-profile choices.
- [x] Rank findings, identify measurement gaps, and recommend an implementation order.

## Measurement gaps

This was a static source audit. It did not run against production credentials or infer production timing from one forum. The following missing evidence should be addressed before and alongside larger changes:

- There is no `benches/` tree, benchmark dependency, command latency budget, request-count budget, or memory budget.
- CI runs formatting, Clippy, functional tests, MSRV, dependency audit, and workflow security, but no performance or request-budget checks.
- The existing mock Discourse records every request method but uses that record only to prove dry-run commands do not mutate. It could also enforce GET budgets, duplicate-read absence, and unchanged-write counts without adding timing flakiness.
- The shared HTTP boundary reports 429 waits but not redacted route, total duration, response bytes, attempt count, or pagination page/item totals.
- There are no delayed-mock tests proving that concurrency remains work-conserving when jobs finish out of order.
- There are no scale fixtures for settings catalogues, category file/topic sets, update logs, S3 pages, SAR exports, or large activity histories.

Highest-value coverage:

1. Request-budget tests for analytics mode/section combinations, unchanged tag pull/push, version-only pulls, category/group lookup, and live topic delete.
2. A delayed-worker test for update parallelism and shared fleet execution.
3. Allocation/throughput benchmarks for SAR serialization, S3 aggregation, category indexing, and site-setting transformation.
4. Opt-in stderr telemetry at the HTTP boundary: redacted route template and method, status, attempts, wait duration, response bytes, elapsed duration, and pagination counts. Never include query secrets, API headers, post bodies, or credential-bearing URLs.

## Hypotheses requiring measurement

These mechanisms are plausible costs but are not promoted to findings without runtime evidence:

- Concurrent requests use independent deterministic 429 sleeps. Analytics workers receiving the same `Retry-After` can wake together and retry as a herd because there is no shared per-host cooldown or jitter. Fixing P1 may reduce this enough that shared throttling is unnecessary.
- `fetch_categories` always merges `/categories.json` with `/site.json`. Compatibility telemetry should record how often `/site.json` contributes a missing ID on supported Discourse versions before considering removal of that request.
- P23's per-file `sync_all` may dominate bulk writes on slow filesystems, but weakening durability without measurements would be a regression.
- SSH connection multiplexing should reduce P27 substantially, but socket lifecycle, host-key isolation, concurrent update behavior, and operator SSH configuration need a focused prototype.
- Most ordinary API methods buffer response text before deserialization. Payloads are usually modest; route-level response-size telemetry should identify exceptions before a broad transport refactor.

## Audit conclusion

The codebase is not suffering from generalized inefficient Rust. Collection choices and local CPU work are mostly reasonable for a network administration CLI, and compiler performance lints are clean. The significant slowdowns come from command orchestration: asking remote systems for more data than the selected operation needs, performing independent requests serially, redoing capability/catalogue work, and retaining large remote output longer than necessary. Request-budget tests and one reusable bounded fleet executor would prevent several classes of recurrence.
