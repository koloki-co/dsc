# Changelog

All notable changes to `dsc` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Releases are grouped from conventional-commit messages by [git-cliff](https://git-cliff.org).

## [0.18.0] - 2026-09-02

### Bug fixes

- **file**: Parse the fetch checksum from mixed ssh stderr ([26d66c3](https://github.com/koloki-co/dsc/commit/26d66c3388da7d235dbfb30ff442ba497bb1efd0))

- **update,config**: Cap parallel widths at shared fleet worker ceiling (P28) ([20e6997](https://github.com/koloki-co/dsc/commit/20e699795b5820f5851ee47e6e4f7810e4c6431b))

### Build

- **deps**: Bump sha2 from 0.10.9 to 0.11.0 ([34268e6](https://github.com/koloki-co/dsc/commit/34268e6e4525fdc742825938b35a3dd7a626d9b0))

- **deps**: Bump taiki-e/install-action ([565c66d](https://github.com/koloki-co/dsc/commit/565c66dc3820e3fa7013a73eda5ad8bdcc8ebf20))

- **deps**: Bump zensical in the routine-minor-and-patch group ([e27d1da](https://github.com/koloki-co/dsc/commit/e27d1da969c2784374061af1d93f2fd75aef7910))

- **deps**: Bump uuid in the routine-minor-and-patch group ([ec3e75e](https://github.com/koloki-co/dsc/commit/ec3e75e3351f059d39bae4f4b0219ca718f302f4))

### Documentation

- **fleet**: Correct the worker-ceiling comment to non-overridable ([4a9f847](https://github.com/koloki-co/dsc/commit/4a9f8474f3d81a9eedfb991b7048dfde6ea09517))

### Features

- **render**: Protect Markdown code fences from template substitution ([6c83196](https://github.com/koloki-co/dsc/commit/6c83196d160ca50fb860fa0ab080d4b8c196900a))

- **backup**: Add --format and a real dry-run plan to backup create ([f06ca88](https://github.com/koloki-co/dsc/commit/f06ca8838351726bece5444b36937f860bf36cbb))

- **file**: Add dsc file pull (R53 Phase 3) ([5596603](https://github.com/koloki-co/dsc/commit/5596603f94d8b86c600314f79be950051e9376c6))

- **file**: Fleet audit and push via R48 selector (R53 Phase 2) ([bf9feb5](https://github.com/koloki-co/dsc/commit/bf9feb56ac18d0b1b897fece16a766b3aea412fe))

## [0.17.0] - 2026-08-29

### Bug fixes

- **emoji,sar**: Bound memory on download and surface failed SAR fetches (R56, R57) ([c3038b1](https://github.com/koloki-co/dsc/commit/c3038b182a81318857c16abb59467ef1be265b3f))

### Documentation

- **file-transfer**: Complete Phase 0 design - host identity, fixtures, no-follow protocol ([83d0fa2](https://github.com/koloki-co/dsc/commit/83d0fa2e64f78893f17546db8997e4e0ca06834d))

- **roadmap**: Clear completed items ([40d9dc9](https://github.com/koloki-co/dsc/commit/40d9dc90b6fd5a3ee9423d82c2e2d7e0174ae17f))

- **roadmap**: Record bug triage R55-R57 from 2026-08-28 review ([e3632f2](https://github.com/koloki-co/dsc/commit/e3632f235b8998aba8a09c974660a09d57b64ad7))

- **roadmap**: Record official MCP lessons ([f30eeeb](https://github.com/koloki-co/dsc/commit/f30eeeb1a01611ad63ef0bbe1ef716cfad59f8c0))

- **roadmap**: Update R52 with P9/P10/P11 progress ([3b37f93](https://github.com/koloki-co/dsc/commit/3b37f933836f86f96337f885223db557a3012def))

### Features

- **file**: Default-on backup and host-key visibility in dry-run ([8d52083](https://github.com/koloki-co/dsc/commit/8d52083e5cd94ca7a61ec193570b82409cbec8c0))

- **file**: Single-forum file audit and push (R53 Phase 1) ([5e6f4a5](https://github.com/koloki-co/dsc/commit/5e6f4a586d0a42f3a2564255f0c1480a31fd6a20))

- **ssh**: Add bounded binary capture and pipe transport (R53 Phase 0) ([a658e40](https://github.com/koloki-co/dsc/commit/a658e40078e5c9a451d09077d79fe51063d7c18c))

- **render**: Add --render flag to topic new/push/reply and category push (R29 Phase 2) ([076d64c](https://github.com/koloki-co/dsc/commit/076d64cdd444256ac5ee6788c8a0686ff945fdbd))

### Performance

- **category-def**: Index server categories for O(1) push matching (P25) ([a3558ce](https://github.com/koloki-co/dsc/commit/a3558ce78c1b48a16f1cfbff417ebbb2e99d3d21))

- **emoji**: Bounded download pool and image size cap (R52 P11) ([3560002](https://github.com/koloki-co/dsc/commit/3560002533ad91d7b983bc34d579993ba1bb75ce))

- **sar**: Bounded post fetches and streaming JSON (R52 P9) ([83378f0](https://github.com/koloki-co/dsc/commit/83378f09d05afb2922adafa47cc88bdb199980e8))

- **topic**: Filter before detail fetch, index topic lookups (R52 P10/P15) ([50e16f9](https://github.com/koloki-co/dsc/commit/50e16f93ef6a479c1c6b42923084f7c5e77cdbb1))

- **fleet**: Parallelise title discovery (R52 P30) ([a025d01](https://github.com/koloki-co/dsc/commit/a025d01c0f5c894fc79acab0b1910ef0ec2b7c04))

- **fleet**: Shared bounded executor for read-only audits (R52 P12/P28) ([7c5b946](https://github.com/koloki-co/dsc/commit/7c5b946e68e74152b6536146f033814b5768ff6a))

- **update**: Bound SSH output retention (R52 P8) ([800cf0d](https://github.com/koloki-co/dsc/commit/800cf0dde8f1717df5cc3b6d8b588e3f25af2ff4))

### Refactor

- **file**: Unify push onto build_replace_script with verify-before-replace ([14ab786](https://github.com/koloki-co/dsc/commit/14ab7868a6b3c68c454cfa74110fb81f79d50642))

- **ssh**: Migrate app.rs and theme.rs onto shared SSH transport ([4b31e94](https://github.com/koloki-co/dsc/commit/4b31e94278fb0a86bf6cc81cc1f15dda052eb0e4))

- **ssh**: Centralize process construction for R53 ([eb0f450](https://github.com/koloki-co/dsc/commit/eb0f45026ed6e78b779ce599cd8f96a2dff06a67))

### Tests

- **perf**: Add request budget regression coverage ([dd09df7](https://github.com/koloki-co/dsc/commit/dd09df7ea2656c549a5d534cdb9affca9b883dc3))

## [0.16.0] - 2026-08-26

### Bug fixes

- **api**: Cap buffered response bodies (R50) ([85f7e64](https://github.com/koloki-co/dsc/commit/85f7e64a2099e2ea298351005ca6c304b73e202c))

- **security**: Scope API redirects to the forum's own origin (R50) ([b32ffe3](https://github.com/koloki-co/dsc/commit/b32ffe33cfacc4bd5020fe12b0a5c1f266b803d8))

- **completions**: Complete optional discourse arguments ([#110](https://github.com/koloki-co/dsc/issues/110)) ([6ce3ed0](https://github.com/koloki-co/dsc/commit/6ce3ed077edc47d630af01d664b730f11924bb4b))

### Build

- **deps**: Bump taiki-e/install-action ([#115](https://github.com/koloki-co/dsc/issues/115)) ([f59eac8](https://github.com/koloki-co/dsc/commit/f59eac885fb9a9018df4632c307ce64be2f130ef))

- **deps**: Bump zensical in the routine-minor-and-patch group ([#114](https://github.com/koloki-co/dsc/issues/114)) ([730a5de](https://github.com/koloki-co/dsc/commit/730a5dedf4614aa21d84e9f5fc5180676bae202c))

- **deps**: Bump the routine-minor-and-patch group with 2 updates ([#113](https://github.com/koloki-co/dsc/issues/113)) ([7672986](https://github.com/koloki-co/dsc/commit/7672986ac6b931de416743ad59dbd0a95caef6a0))

### Documentation

- **roadmap**: Correct release status, add R51 and R52 ([995e247](https://github.com/koloki-co/dsc/commit/995e2471d202765687ee4e6be19c983856a6d9cb))

- State alpha status on the README and docs landing page ([e4be3d2](https://github.com/koloki-co/dsc/commit/e4be3d218913d19bfe7dbf22a6b5229a9037cd60))

- Link the Meta announcement and align the security contact ([218da60](https://github.com/koloki-co/dsc/commit/218da60498a1c58955ddd25541f075ec4db12350))

- **roadmap**: Decouple the Meta announcement from v1.0.0 ([e11ecf1](https://github.com/koloki-co/dsc/commit/e11ecf14ee13987b078087934ba71a4c7822068a))

- **topic**: Specify post ownership reassignment ([3c74a82](https://github.com/koloki-co/dsc/commit/3c74a82123638975b65a23dad7a8c1326bff932e))

### Features

- **post**: Add dsc post change-owner single-post alias (R49 Phase 2) ([#112](https://github.com/koloki-co/dsc/issues/112)) ([9f38d3c](https://github.com/koloki-co/dsc/commit/9f38d3c655de6c772603898a87a6ab33fe118b4b))

- **topic**: Add dsc topic change-owner (R49 Phase 1+) ([#111](https://github.com/koloki-co/dsc/issues/111)) ([e989895](https://github.com/koloki-co/dsc/commit/e9898951703c17bea957d49f864f0070f702de1d))

- **render**: Add --strict and --list-vars to dsc render (R29 Phase 2) ([#109](https://github.com/koloki-co/dsc/issues/109)) ([a5958c8](https://github.com/koloki-co/dsc/commit/a5958c8602a2dfb83c7d8bfa7c2050ca0d0dbd67))

- **fleet**: Unify fleet selector across backup/search/user (R48) ([#108](https://github.com/koloki-co/dsc/issues/108)) ([7710472](https://github.com/koloki-co/dsc/commit/771047231576f852a1515a1079e89ebd368865df))

- **backup**: Add --reuse-user key rotation to setup-s3 (R13 Phase 2) ([#107](https://github.com/koloki-co/dsc/issues/107)) ([56b025a](https://github.com/koloki-co/dsc/commit/56b025a8ca9c88acdf4eaf79beb9d0f1427c2872))

- **render**: Add `dsc render` for template placeholder substitution (R29 Phase 1) ([#106](https://github.com/koloki-co/dsc/issues/106)) ([1ed3208](https://github.com/koloki-co/dsc/commit/1ed3208399d92c5775d79b75d4f93fc97d6ace9e))

## [0.15.0] - 2026-08-20

### Bug fixes

- **backup**: Reset IAM profile for static keys ([03198dd](https://github.com/koloki-co/dsc/commit/03198dd5aefd3c398d3e4e77d39290219747f834))

- Correct performance audit optimizations ([8f052a2](https://github.com/koloki-co/dsc/commit/8f052a25a22936ce2c38fc36c7d8c87956f3cc43))

- **update**: Make Discourse branch configurable, default to latest ([3d014d3](https://github.com/koloki-co/dsc/commit/3d014d341b9345c52d9d04da1e9c75c16db504be))

- **category**: Skip unchanged list edits ([917db41](https://github.com/koloki-co/dsc/commit/917db41d5da49d8684d38fe148c9ae278f6b9e02))

- **group**: Preserve empty notification defaults ([29268d8](https://github.com/koloki-co/dsc/commit/29268d897e586a88d78910b5832956a41ae3b9b4))

### Build

- **deps**: Bump the routine-minor-and-patch group with 3 updates ([8ea6476](https://github.com/koloki-co/dsc/commit/8ea6476bfe117e3a494936047384e409ee35d076))

- **deps**: Bump taiki-e/install-action ([8137463](https://github.com/koloki-co/dsc/commit/8137463b946552880492c2e70917e7d721ea20b5))

- **deps**: Bump taiki-e/install-action ([f09bee1](https://github.com/koloki-co/dsc/commit/f09bee1c961c2510f251ecf0f56a6a89fc01879d))

- **deps**: Bump zensical in the routine-minor-and-patch group ([60c3402](https://github.com/koloki-co/dsc/commit/60c3402b592f22b7e80d8032dff3fff2b617fe82))

- **deps**: Bump the routine-minor-and-patch group with 2 updates ([d172517](https://github.com/koloki-co/dsc/commit/d1725174d34282132a86749cf6860dc9256adbe8))

### Documentation

- Reconcile roadmap and recent CLI changes ([4c4f8b5](https://github.com/koloki-co/dsc/commit/4c4f8b5e29a92640fae16e37be78982afb19212b))

- **category**: Clarify category type sync ([6677e0f](https://github.com/koloki-co/dsc/commit/6677e0fe11d9426a6a2e7e0b15030746d11a3e6e))

- Defer P6 - per-forum GitHub fetch is intentional ([6beeae6](https://github.com/koloki-co/dsc/commit/6beeae67f039fc53ca346ab9ebcd385e67bcf122))

### Features

- **category**: Add topic_title_placeholder to category definition sync (R11) ([#104](https://github.com/koloki-co/dsc/issues/104)) ([3f57cbc](https://github.com/koloki-co/dsc/commit/3f57cbc9a83126b2f43bc4dcf4239c58cf8f6c1f))

- **update**: Add theme-derived label colour (R47 Phase 2) ([1a0a241](https://github.com/koloki-co/dsc/commit/1a0a2410163b1590897344d5e1f04b8d6231fe0c))

- **backup**: Stream endpoint-aware health checks ([b6bccfc](https://github.com/koloki-co/dsc/commit/b6bccfc4f06426229a0cca2dc9d210474abcfe92))

- **category**: Resolve parent by name and validate before push (R11) ([#100](https://github.com/koloki-co/dsc/issues/100)) ([506567b](https://github.com/koloki-co/dsc/commit/506567b9ad24d43e94ffedbb26c454b7fa084d4a))

- **backup**: Add --all/--tags fleet fan-out to setup-s3 (R13) ([#99](https://github.com/koloki-co/dsc/issues/99)) ([5e8cebe](https://github.com/koloki-co/dsc/commit/5e8cebe92bfdff7260e569510a5eda42aac11619))

- **user**: Add `dsc user find <email>` GDPR fleet lookup (R21) ([#98](https://github.com/koloki-co/dsc/issues/98)) ([d213be3](https://github.com/koloki-co/dsc/commit/d213be31c4769519ba212181719b9b377570798a))

- **category**: Sync custom fields ([#97](https://github.com/koloki-co/dsc/issues/97)) ([1f020a7](https://github.com/koloki-co/dsc/commit/1f020a715e114fa0d0b797695dffff920e763e3d))

- **backup**: Add `dsc backup create all` fleet fan-out ([#96](https://github.com/koloki-co/dsc/issues/96)) ([52e2724](https://github.com/koloki-co/dsc/commit/52e2724872247b9312fb25efb360a2cc05d012eb))

- **category**: Sync required tag groups and types ([34a4b34](https://github.com/koloki-co/dsc/commit/34a4b344eb056be130779977bea3ad12281f14f4))

- **backup**: Add --use-iam-profile to `backup setup-s3` ([f58f1dd](https://github.com/koloki-co/dsc/commit/f58f1dd2d3f04c45870389ffa728ad6dd393bac4))

- **category**: Add --append/--remove to `category set` for list fields ([8605ef9](https://github.com/koloki-co/dsc/commit/8605ef994ca98297a961a977ec51d725816eb0b9))

- **group**: Add --with-defaults to surface notification-level defaults ([836fcf9](https://github.com/koloki-co/dsc/commit/836fcf94d194403f9d5543b6b120253340baab0a))

### Performance

- Fix P7, P17, P18, P19, P21, P31 from performance audit ([fd1c3e3](https://github.com/koloki-co/dsc/commit/fd1c3e3bbddcf7e1f114def55fb0755427d38c75))

- Fix P1-P5 from performance audit ([78983dd](https://github.com/koloki-co/dsc/commit/78983dda838c3d8f60a721c38c3169dbdff600c8))

## [0.14.0] - 2026-08-07

### Bug fixes

- **completions**: Default Zsh installs to .zfunc ([32416b5](https://github.com/koloki-co/dsc/commit/32416b54242c2fae3050757cf0ceb9a371f79766))

### Build

- **deps**: Bump taiki-e/install-action ([#83](https://github.com/koloki-co/dsc/issues/83)) ([d484ce2](https://github.com/koloki-co/dsc/commit/d484ce2e8ffec68e8618f00ab9c966e5a2a45dfc))

- **deps**: Bump base64 from 0.22.1 to 0.23.0 ([#82](https://github.com/koloki-co/dsc/issues/82)) ([45f9d08](https://github.com/koloki-co/dsc/commit/45f9d08c43b07aa25539248ebcde83775adc1dc0))

- **deps**: Bump the routine-minor-and-patch group with 2 updates ([#81](https://github.com/koloki-co/dsc/issues/81)) ([59aff17](https://github.com/koloki-co/dsc/commit/59aff177e3a359ce11bbffec8c3868e595d34adb))

### CI

- Retrigger after the 2026-08-06 GitHub Actions/Pages outage ([aad01fe](https://github.com/koloki-co/dsc/commit/aad01fe3b1e16ef7657bd89a3019d51fe88c13cb))

- Repin dtolnay/rust-toolchain to master, not stable ([b27548f](https://github.com/koloki-co/dsc/commit/b27548ffbd4e6e5838613965a630806c0be66c86))

### Chores

- Ignore Playwright MCP captures ([a787198](https://github.com/koloki-co/dsc/commit/a78719899079b386dd3e0d67667f18c3529f1d43))

### Documentation

- **roadmap**: Record recent CLI work and colour design ([4f7d02c](https://github.com/koloki-co/dsc/commit/4f7d02c30082627bf67eabaaac57b2c523662f87))

### Features

- **palette**: Add push dry-run plans ([161935b](https://github.com/koloki-co/dsc/commit/161935bd6f31735aefabfad240c5226a89ed3367))

- **category**: Make definition sync strict and idempotent ([b945c6e](https://github.com/koloki-co/dsc/commit/b945c6e4967b89eddde4be874c076e10cbc4d83d))

- **post**: Add dsc post info command ([b351919](https://github.com/koloki-co/dsc/commit/b3519195970168acb0a142ce6e63c875b3edf753))

## [0.13.1] - 2026-08-03

### Bug fixes

- **update**: Recover safely from low disk space ([025678b](https://github.com/koloki-co/dsc/commit/025678bc5cfa407691fa2e239ccaef56ae411f8c))

- **webhook**: Harden webhook administration ([632cad5](https://github.com/koloki-co/dsc/commit/632cad546998e3e04e4b2fa4e39b4a4fa02898c6))

### Chores

- Ignore Playwright MCP captures ([7fdf9ce](https://github.com/koloki-co/dsc/commit/7fdf9cee2e4f5cc29735b31b1a40b1184cadbd6f))

### Features

- **webhook**: Add basic webhook admin commands (list/create/delete/ping) ([d8f979b](https://github.com/koloki-co/dsc/commit/d8f979b4f2db7d4565171289b6c4b2ff8fdd17fa))

## [0.13.0] - 2026-08-01

### Bug fixes

- Update URLs after the transfer to koloki-co ([dc205e7](https://github.com/koloki-co/dsc/commit/dc205e7caa6141ac7214aef1d4f2525055cf9222))

- Harden credential redaction ([11a26a3](https://github.com/koloki-co/dsc/commit/11a26a35409e07340f2560957b8b3cf07dc11df3))

- Harden live compatibility tests ([3151176](https://github.com/koloki-co/dsc/commit/3151176449d6ea3032bba43a1e5e6822b78f17b9))

### Build

- **deps**: Bump the routine-minor-and-patch group across 1 directory with 5 updates ([#71](https://github.com/koloki-co/dsc/issues/71)) ([9ca9332](https://github.com/koloki-co/dsc/commit/9ca933253116a5111351550846981372b4a302c5))

- **deps**: Bump actions/setup-python from 6.3.0 to 7.0.0 ([#72](https://github.com/koloki-co/dsc/issues/72)) ([d2e0425](https://github.com/koloki-co/dsc/commit/d2e0425ef1d7987399de90ed0e612652cc74f266))

### Documentation

- **spec**: Record the MCP server design and what blocks it (R24) ([2330ed9](https://github.com/koloki-co/dsc/commit/2330ed9bdbe8953510a38b941b6c5531fe296248))

- **spec**: Make the roadmap the single list of planned work ([a5d3cc5](https://github.com/koloki-co/dsc/commit/a5d3cc57cabf1683080496aa3b520722d23057cd))

- Serve the site from dsc.koloki.co ([f529966](https://github.com/koloki-co/dsc/commit/f529966ee67a9dec27f640c36b557839d141ea89))

- Narrow R28 to phase 3 ([ee9dc27](https://github.com/koloki-co/dsc/commit/ee9dc27bec5cf0ff045dd451a6eccb454656b80b))

### Features

- **explorer**: Add dsc explorer for saved Data Explorer queries (R40) ([66ab4a7](https://github.com/koloki-co/dsc/commit/66ab4a768a4d511e8be24b43fdd086c6de8cb712))

- Add app environment management ([5365f79](https://github.com/koloki-co/dsc/commit/5365f79d386f22b87b4b9fce378a9e2027f18aa2))

### Spec

- **backup**: Add fleet health command ([e1fbb12](https://github.com/koloki-co/dsc/commit/e1fbb12b62a865bcfe8539f1cf3cbdbc278ca883))

## [0.12.1] - 2026-07-27

### Bug fixes

- **category**: Compare explicit live category definitions ([a19278d](https://github.com/koloki-co/dsc/commit/a19278db0edb38546cadc2f5a84980c440080a87))

### CI

- Add Homebrew to PATH in the formula-publish step ([f643fbc](https://github.com/koloki-co/dsc/commit/f643fbc531541abdaba6afaabffd0a82a82cd979))

### Documentation

- Describe final category diff design in v0.12.1 notes ([9cbdb71](https://github.com/koloki-co/dsc/commit/9cbdb71c285d179207d5af6262a0af7d481a2496))

### Features

- **category**: Add category def diff ([cac3a34](https://github.com/koloki-co/dsc/commit/cac3a3498b64c21cfe2c9f186184a46d3244f4cd))

## [0.12.0] - 2026-07-26

### Bug fixes

- Address R40-R51 stability and security audit findings ([abebc2a](https://github.com/koloki-co/dsc/commit/abebc2a4bc4867fceb9040ed1b96beb11e43de3f))

### Build

- **deps**: Bump zensical in the routine-minor-and-patch group ([#62](https://github.com/koloki-co/dsc/issues/62)) ([74328fc](https://github.com/koloki-co/dsc/commit/74328fc08959ee8ac711c00b4aabb7fe260f156f))

### CI

- Upload only files as release assets ([7c36e72](https://github.com/koloki-co/dsc/commit/7c36e72cb9322fbef6709d9ae4e3c193c5802842))

### Features

- **category**: Add category rename command ([32296fb](https://github.com/koloki-co/dsc/commit/32296fb08885d5357c5b5a65c98a0d71bc8cb974))

## [0.11.0] - 2026-07-25

### Bug fixes

- Bound HTTP client with a connect and total request timeout ([9645af5](https://github.com/koloki-co/dsc/commit/9645af541f7daeb55a847857f876f38c9bcabb63))

- **release**: Authenticate auto-tag push ([0b15ea8](https://github.com/koloki-co/dsc/commit/0b15ea8af3a3d789144bd0a58b21a44f722db700))

### Build

- **deps**: Update dtolnay/rust-toolchain requirement to 4cda84d5c5c54efe2404f9d843567869ab1699d4 ([7ad1e1e](https://github.com/koloki-co/dsc/commit/7ad1e1ef38655ac496b5155378dc49f8de73c8c6))

- **deps**: Bump actions/checkout ([b2d023a](https://github.com/koloki-co/dsc/commit/b2d023aae9e1748323173a88d1fc9ed05d45c0ff))

- **deps**: Bump the routine-minor-and-patch group across 1 directory with 2 updates ([1d7198a](https://github.com/koloki-co/dsc/commit/1d7198a8bcdab6090c43355e76bc6f457e99b603))

### Documentation

- Complete protected release rehearsal ([3101a85](https://github.com/koloki-co/dsc/commit/3101a85867945b5820145f12f37f129a356fb5ce))

### Features

- R37 CLI ergonomics, R38 workflow security, R10 link rewriting, R36 live-test isolation ([f154bb3](https://github.com/koloki-co/dsc/commit/f154bb3f8dfa46befa5b05d3549a01ff75588a69))

- **licensing**: Record third-party asset provenance and REUSE compliance ([636bdf8](https://github.com/koloki-co/dsc/commit/636bdf801b5287e8f19137def2d7520f03ba34de))

## [0.10.32] - 2026-07-24

### Build

- **deps**: Bump zensical in the routine-minor-and-patch group ([c6b8825](https://github.com/koloki-co/dsc/commit/c6b882543808b7c09425046d8bb39876837254f3))

### CI

- Harden launch gates and retire audit ([debe518](https://github.com/koloki-co/dsc/commit/debe518911542bb04682d6a15287ab39d831ce41))

- Move releases behind protected main ([f181724](https://github.com/koloki-co/dsc/commit/f181724f0429cb511ef83eae61b8f729e732991b))

### Documentation

- Tidy release roadmap ([9cc1913](https://github.com/koloki-co/dsc/commit/9cc1913e028956419802900f53f64f617a4cf640))

- R34 - correct harden claims, safety-first quick start ([e3fbb5e](https://github.com/koloki-co/dsc/commit/e3fbb5e2cb1e9b31228eb6b2cde091092813b940))

- Record remaining R31 protection gate ([cfd3ca8](https://github.com/koloki-co/dsc/commit/cfd3ca810232be3ec9a8b93ffe4e70ab5087b7d9))

- Record main protection activation ([f9fc1ab](https://github.com/koloki-co/dsc/commit/f9fc1ab02443feed9a8908c28466a59babb0c941))

- R23 - reconcile docs and --help with actual CLI behavior ([9e39bd4](https://github.com/koloki-co/dsc/commit/9e39bd4ad2279640d0f7591ac6cf1a66bcafec2f))

- Add template rendering roadmap ([ba83aae](https://github.com/koloki-co/dsc/commit/ba83aaedb1e457b8babe35f9bbe2579e6f19a906))

- Record trusted publishing validation ([bee7ecb](https://github.com/koloki-co/dsc/commit/bee7ecbd7fe665e8a02935a82c30fc4d173d9c0c))

### Features

- **release**: Define 1.0 compatibility contract ([dd82535](https://github.com/koloki-co/dsc/commit/dd825359ac9f54fde838992ab9d6b6d1e4bc65fe))

## [0.10.31] - 2026-07-23

### Bug fixes

- Fail closed on incomplete dry runs ([935aa6a](https://github.com/koloki-co/dsc/commit/935aa6a9c3145345f201f8e762469e1bfa0a9225))

- Add local lint hook ([1cb0bec](https://github.com/koloki-co/dsc/commit/1cb0bec0c177bbca220ef4a58e58e1c4828a24f7))

### Build

- **deps**: Bump taiki-e/install-action from 2.82.11 to 2.83.2 ([#44](https://github.com/koloki-co/dsc/issues/44)) ([96a76cf](https://github.com/koloki-co/dsc/commit/96a76cfa9dfb88951d9fd0006b7b9499981b4d96))

- **deps**: Bump clap_complete from 4.6.6 to 4.6.7 ([#43](https://github.com/koloki-co/dsc/issues/43)) ([4d9b2ba](https://github.com/koloki-co/dsc/commit/4d9b2baf6379cff7f6dbd003c51f9a6158299b96))

- **deps**: Bump indicatif from 0.18.5 to 0.18.6 ([#42](https://github.com/koloki-co/dsc/issues/42)) ([fe6fd5a](https://github.com/koloki-co/dsc/commit/fe6fd5af6b96f565477d8416e97ec06c4702eed1))

- **deps**: Bump toml from 1.1.2+spec-1.1.0 to 1.1.3+spec-1.1.0 ([#41](https://github.com/koloki-co/dsc/issues/41)) ([77ff880](https://github.com/koloki-co/dsc/commit/77ff8800f0b647ab8265ab63b0154090677cccfd))

- **deps**: Bump uuid from 1.23.4 to 1.23.5 ([#40](https://github.com/koloki-co/dsc/issues/40)) ([b52d702](https://github.com/koloki-co/dsc/commit/b52d702b9d51fe7a0e3e7f1aef01ff99a3980b6e))

- **deps**: Bump actions/cache from 6.0.0 to 6.1.0 ([#39](https://github.com/koloki-co/dsc/issues/39)) ([23ad86d](https://github.com/koloki-co/dsc/commit/23ad86dd58c13420235ea854071941e60eb76d8d))

- **deps**: Bump taiki-e/install-action from 2.82.7 to 2.82.11 ([#35](https://github.com/koloki-co/dsc/issues/35)) ([f239fd2](https://github.com/koloki-co/dsc/commit/f239fd295e9a6c28ac1651f8290c0e5eb22a1cab))

- **deps**: Bump actions/cache from 5.0.5 to 6.1.0 ([#23](https://github.com/koloki-co/dsc/issues/23)) ([bd85deb](https://github.com/koloki-co/dsc/commit/bd85debfba6a4442573c4c23d77dc99d45da3d6d))

- **deps**: Bump actions/setup-python from 6.2.0 to 6.3.0 ([#22](https://github.com/koloki-co/dsc/issues/22)) ([3dac611](https://github.com/koloki-co/dsc/commit/3dac61134d497a35494dc8c24ab147f6c9dc1000))

- **deps**: Bump chrono from 0.4.43 to 0.4.45 ([#25](https://github.com/koloki-co/dsc/issues/25)) ([49aa4bf](https://github.com/koloki-co/dsc/commit/49aa4bfc03b86e4cc53f9808095657a5d72e1770))

- **deps**: Bump libc from 0.2.182 to 0.2.186 ([#34](https://github.com/koloki-co/dsc/issues/34)) ([7d086ed](https://github.com/koloki-co/dsc/commit/7d086ed6b5bdb908457ade9101791b9baea40031))

- **deps**: Bump clap from 4.5.57 to 4.6.1 ([#33](https://github.com/koloki-co/dsc/issues/33)) ([6669081](https://github.com/koloki-co/dsc/commit/66690815b25b1027df40c8bc69947c26a1565323))

- **deps**: Bump clap_complete from 4.5.65 to 4.6.6 ([#32](https://github.com/koloki-co/dsc/issues/32)) ([6d766b1](https://github.com/koloki-co/dsc/commit/6d766b1b4460f0755c2ef44481b8a0cf2240e686))

- **deps**: Bump toml from 1.0.3+spec-1.1.0 to 1.1.2+spec-1.1.0 ([#31](https://github.com/koloki-co/dsc/issues/31)) ([4f5d826](https://github.com/koloki-co/dsc/commit/4f5d8263e44bc4958947ed6915104fe06b046965))

- **deps**: Bump uuid from 1.21.0 to 1.23.4 ([#30](https://github.com/koloki-co/dsc/issues/30)) ([b7658b7](https://github.com/koloki-co/dsc/commit/b7658b7c9dab6b4ce1eff818661d0a87a8017d73))

- **deps**: Bump indicatif from 0.18.3 to 0.18.5 ([#29](https://github.com/koloki-co/dsc/issues/29)) ([767ffd6](https://github.com/koloki-co/dsc/commit/767ffd6953916b9bdaef919c5bd012f972206213))

- **deps**: Bump reqwest from 0.13.2 to 0.13.4 ([#28](https://github.com/koloki-co/dsc/issues/28)) ([35bf35b](https://github.com/koloki-co/dsc/commit/35bf35b71e039ccdd8e5a527dd82ba0b75da034d))

- **deps**: Bump tempfile from 3.26.0 to 3.27.0 ([#27](https://github.com/koloki-co/dsc/issues/27)) ([1de8cf4](https://github.com/koloki-co/dsc/commit/1de8cf4ca070b961e965e0332a0348a05729786b))

- **deps**: Bump serde_json from 1.0.149 to 1.0.150 ([#26](https://github.com/koloki-co/dsc/issues/26)) ([616ca2b](https://github.com/koloki-co/dsc/commit/616ca2bf14ed9df30b938b43bc9e6fa884adab8c))

### CI

- Adopt crates.io trusted publishing ([2502d11](https://github.com/koloki-co/dsc/commit/2502d11474f217b5d587379da360d32a948d03dc))

### Documentation

- Document s/ and wix/ naming conventions (R4) ([#37](https://github.com/koloki-co/dsc/issues/37)) ([4807a4c](https://github.com/koloki-co/dsc/commit/4807a4c6274098e995c94ccbcd431b5acf73dcda))

### Features

- Add `dsc notification list|read` for notification inspection and mark-read ([#38](https://github.com/koloki-co/dsc/issues/38)) ([12cedb6](https://github.com/koloki-co/dsc/commit/12cedb6feb17fb06b0a84b94fbce3ba1150c3fbe))

- Add `dsc log staff` for staff action log access ([#36](https://github.com/koloki-co/dsc/issues/36)) ([a2b0bab](https://github.com/koloki-co/dsc/commit/a2b0babcbd3e5acab66b556a31c39abba228a4f1))

- Convert category admonitions ([7640018](https://github.com/koloki-co/dsc/commit/764001836f5fbebfed9f7e3d5f62d2601fe2a76b))

- Improve topic lifecycle and fleet operations ([d53b732](https://github.com/koloki-co/dsc/commit/d53b73299429c679a0e09ce56d5918bc792ac50f))

## [0.10.30] - 2026-07-01

### CI

- Add cargo audit security gate (blocking, own job) ([8a8f2c8](https://github.com/koloki-co/dsc/commit/8a8f2c8096bb9a615b391536c4ef7776062eae59))

### CI / dependencies

- **deps**: Clear remaining RustSec advisories (cargo audit) ([7922727](https://github.com/koloki-co/dsc/commit/7922727b693b5cf1ef1f1ec785152dfe4a39cf5c))

- **deps**: Patch RustSec advisories in the reqwest TLS stack ([a439481](https://github.com/koloki-co/dsc/commit/a4394814b12d5156332cd30b66220a118fec7ee2))

## [0.10.29] - 2026-07-01

### Bug fixes

- **tag**: Correct delete endpoint and create-via-group ordering ([95c77b3](https://github.com/koloki-co/dsc/commit/95c77b3665d377cdd866c3528dcd16286a41673d))

### Chores

- **scripts**: Extract shared s/test-fmt-clippy gate ([bd4671e](https://github.com/koloki-co/dsc/commit/bd4671e29584108285e96bdb3facea2180bc8d84))

### Documentation

- **roadmap**: Log tag-pull group-permission id-vs-name bug ([182bea1](https://github.com/koloki-co/dsc/commit/182bea11e0c843055e65f6c090e7f595035c8dcc))

- **spec**: Reorganise into two tiers (overarching + spec/commands/) ([53fbb1f](https://github.com/koloki-co/dsc/commit/53fbb1f80cf6ac056e247e2e9705462bb5224287))

### Features

- **category**: Definition sync — def pull/push + show/get/set ([efde940](https://github.com/koloki-co/dsc/commit/efde9408e5e58e4174102972b693715eee6a618c))

- **version**: Make -v/-V/--version/-version all report the version ([aa2cc5e](https://github.com/koloki-co/dsc/commit/aa2cc5e2771c5540304b2b4e33238f5e56e596f7))

- **update**: Append-only update log + skip-recently-updated ([22e977e](https://github.com/koloki-co/dsc/commit/22e977e9073c34269c5972ce8412b53237703e43))

## [0.10.28] - 2026-07-01

### Chores

- Gitignore demo-dsc.toml ([dc191fc](https://github.com/koloki-co/dsc/commit/dc191fc386eff2e72b9f40bea196c31717e6b395))

### Features

- **update**: Leaner `-p [N]` + skip a forum that's already rebuilding ([a4d9ce9](https://github.com/koloki-co/dsc/commit/a4d9ce9af99578195833d47cafbbaaad54b65b08))

### Tests

- **update**: Update the parallel-guard test for `-p N` (was `--max`) ([ed121a7](https://github.com/koloki-co/dsc/commit/ed121a74fe7c3cdc8e917dcb34ec0f7f6d5740a2))

### Spec

- Dsc update refinements (leaner -p, rebuild-lock); prune roadmap ([b4fdb38](https://github.com/koloki-co/dsc/commit/b4fdb387f0315c67470dac79105a3b4a0a973ab7))

## [0.10.27] - 2026-07-01

### CI

- Add push/PR CI gate; commit Cargo.lock ([2d5402f](https://github.com/koloki-co/dsc/commit/2d5402f30ac3d06c4cf81f98b2e694c6fb5d824b))

### Documentation

- **spec**: Extract CLI design philosophy into spec/cli-design.md ([a2ef16a](https://github.com/koloki-co/dsc/commit/a2ef16a7377b16e08ce80a133e28d594ab217dba))

- **spec**: Refresh roadmap state + document core command patterns ([2dbbdda](https://github.com/koloki-co/dsc/commit/2dbbdda4807fb1679cb81d50ede0f62402738a08))

### Features

- **cli**: Reset SIGPIPE + structured `version --format` ([40e7f58](https://github.com/koloki-co/dsc/commit/40e7f58336b4ee2ca3581d7d98192d87af88ff90))

### Styling

- Clear clippy warnings so `--all-targets -- -D warnings` is clean ([f6732cc](https://github.com/koloki-co/dsc/commit/f6732ccb8a21d2aa29cea973e18b1b7ef6a78849))

- **theme**: Rustfmt the theme install/import code ([b8a5118](https://github.com/koloki-co/dsc/commit/b8a511888c6a2be0047f8a1325d4c101afd864f4))

## [0.10.26] - 2026-07-01

### Bug fixes

- **completions**: Accept `powershell` as the shell value ([9a202f0](https://github.com/koloki-co/dsc/commit/9a202f03e99c822f7afc853d05020c6f27901e1c))

### Build

- **deps**: Bump actions/checkout from 6.0.3 to 7.0.0 ([a237efd](https://github.com/koloki-co/dsc/commit/a237efd48abb633d0e818cb50f8de48b43e1c9a0))

### Documentation

- **roadmap**: Park `api-key create --scope` (descoped for now) ([b0db32d](https://github.com/koloki-co/dsc/commit/b0db32d719a481d0f349ee8d1a1904aa0b4e6700))

### Features

- **theme**: API install (git/bundle), delete-by-id, and asset unset ([de6c161](https://github.com/koloki-co/dsc/commit/de6c161f52a032c2c450771dc505baaa3790e492))

- **theme**: Field, asset, and update commands (Phase 2 + 3) ([e566363](https://github.com/koloki-co/dsc/commit/e566363677ded95c81cf483fafd30f26f50db35c))

- **cli**: Add completions installer ([d4d6b9e](https://github.com/koloki-co/dsc/commit/d4d6b9e9e4cf72f30c92a623ab5536d4bd2dcb95))

## [0.10.25] - 2026-06-29

### Features

- **theme**: `theme setting pull/push` for file-based component config ([c4879f3](https://github.com/koloki-co/dsc/commit/c4879f3c8d6ee93264a585cd084a42b10c63c1dd))

## [0.10.24] - 2026-06-26

### Bug fixes

- **backup**: List real backups from the bare-array API response ([4e8b079](https://github.com/koloki-co/dsc/commit/4e8b0794352e569c81559c6cbc39d981545d9282))

### Tests

- **completions**: Assert command coverage and dynamic-name injection ([67b8d79](https://github.com/koloki-co/dsc/commit/67b8d791a3fcdf1ab1bbec2221cf47f932a4afc6))

## [0.10.23] - 2026-06-26

### Bug fixes

- **backup**: Enable backup_location=s3 LAST in setup-s3 ([b9fc608](https://github.com/koloki-co/dsc/commit/b9fc608355d804d118ce26bb3af87fc009ddff76))

### Chores

- **s/docs**: Bind the first free port in 8000-8030 ([ad9cef5](https://github.com/koloki-co/dsc/commit/ad9cef5c0afd6a270c3ea6b508dc708fd0d2d80b))

## [0.10.22] - 2026-06-25

### Bug fixes

- **error**: Accurate hint for invalid/non-staff API credentials ([ea75f7b](https://github.com/koloki-co/dsc/commit/ea75f7b7bef71ead4911021bfa35488970126871))

### Features

- **config**: `config check --parallel` probes forums concurrently ([4cff576](https://github.com/koloki-co/dsc/commit/4cff57611b7ad6ddb157a74b8f72191615a1edca))

- **config**: Stream `config check` results with a progress signpost ([1b11f7d](https://github.com/koloki-co/dsc/commit/1b11f7d8d89b634412a546a447f036ce458d6a9d))

- **backup**: `dsc backup setup-s3` - provision S3 backups in one command (Phase 1) ([9eca042](https://github.com/koloki-co/dsc/commit/9eca042aed01f866b010f10fe8e3051165c584bc))

### Spec

- **backup**: Add `dsc backup setup-s3` field spec (S3 bucket + scoped IAM provisioning) ([20197b4](https://github.com/koloki-co/dsc/commit/20197b49dfaf7090c9dbe7a38976a258a7692ee0))

## [0.10.21] - 2026-06-24

### Features

- **version**: `dsc version <forum>` reports a forum's Discourse version + commit ([6176d38](https://github.com/koloki-co/dsc/commit/6176d387a9544550c27b81848d3ce9bc091891c3))

## [0.10.20] - 2026-06-24

### Bug fixes

- **topic**: Honour --dry-run on `topic reply` (preview, never post) ([e972d25](https://github.com/koloki-co/dsc/commit/e972d255a05044a91fa7b09e8c63c1dd76291ace))

- **setting**: Persist site-setting writes (form field named after the setting) ([51b8727](https://github.com/koloki-co/dsc/commit/51b8727fb1f8105a72e6bf1502a44eb76b66fe5c))

### Features

- **cli**: Sort help alphabetically, add Examples to every command, surface `setting pull` ([995a8dc](https://github.com/koloki-co/dsc/commit/995a8dc69b18502d5c2e2902001754cee7ff7fbb))

### Styling

- Cargo fmt the new site-setting regression test ([3661db8](https://github.com/koloki-co/dsc/commit/3661db8b448a51ba7a187deccf4704112f419f5e))

## [0.10.19] - 2026-06-23

### Documentation

- **readme**: Add a "What works today" capability matrix ([bc7fcc8](https://github.com/koloki-co/dsc/commit/bc7fcc8cb9ee77f11c4635cecb94bebac90e2cec))

- **roadmap**: Refresh stale test count (125 → 181) in the 1.0 bullet ([9f0741b](https://github.com/koloki-co/dsc/commit/9f0741b3645cde3373ee284acfe532dc4fb51e28))

### Features

- **sar**: One-shot Subject Access Request export (`dsc sar`, Phase 1) ([c3a1ff9](https://github.com/koloki-co/dsc/commit/c3a1ff95a1300594cb0f99e0c6d7669120605e9e))

- **setting**: Add `setting audit` - one setting across every forum ([1530f4e](https://github.com/koloki-co/dsc/commit/1530f4eda18d1fcddff9e93864c2259dfa214de8))

### Styling

- Apply clippy autofixes and cargo fmt ([6c3bc81](https://github.com/koloki-co/dsc/commit/6c3bc818558b0a0feba81aa91e9c1d1be85f9ea6))

### Spec

- **sar**: One-shot Subject Access Request export (`dsc sar`) ([57ec43a](https://github.com/koloki-co/dsc/commit/57ec43a4f3aa528cf4e5777fa7b74d5227d849a9))

## [0.10.18] - 2026-06-23

### Bug fixes

- **emoji**: Preserve hyphens in bulk-upload emoji names ([b35aac7](https://github.com/koloki-co/dsc/commit/b35aac7c556cbb4841e145f50c2e8226e825304b))

### Features

- **topic**: Add `topic title` and `topic tags` for metadata editing ([72b3e4e](https://github.com/koloki-co/dsc/commit/72b3e4e862991ee790a1be9c1898782b69cd70d1))

- **theme**: Move `palette` under `theme palette` with a deprecation alias ([63a9320](https://github.com/koloki-co/dsc/commit/63a932030ecd824fa99babfb5f1e1f142d74d083))

- **cli**: Universal --format json|yaml on single-value commands ([3b5c1b5](https://github.com/koloki-co/dsc/commit/3b5c1b5b7681bc553ca201f83bae5534ed0604da))

### Spec

- Dsc topic title and topic tags subcommands ([0f85375](https://github.com/koloki-co/dsc/commit/0f8537567bd69289cabdbec051ef5647ec47449f))

## [0.10.17] - 2026-06-22

### Features

- **theme**: Add `dsc theme show` for a richer single-theme view (theme mgmt Phase 3) ([c4b1dac](https://github.com/koloki-co/dsc/commit/c4b1dac1b09eb803dcfc2a9d5c4432d8618383d1))

- **theme**: Component settings, enable/disable, attach/detach (theme mgmt Phase 1) ([8983c04](https://github.com/koloki-co/dsc/commit/8983c04fa34e848f8cadd6ce4cfeda922174ef75))

## [0.10.16] - 2026-06-22

### Features

- **topic,category**: Add --no-bump/--skip-revision; strip front matter on topic push ([0c7e3f0](https://github.com/koloki-co/dsc/commit/0c7e3f0ab5bc954e17352e9efa31954ae96c9132))

- **category**: Route push by topic_id, honour --dry-run, add --updates-only ([705289e](https://github.com/koloki-co/dsc/commit/705289e5caf858edf7494a242cef96731e1e4db0))

- **category**: Embed YAML front matter in category pull (Gap 1, pull side) ([61cd71f](https://github.com/koloki-co/dsc/commit/61cd71faf94c80bcf4d977c15b6e062dc03523a2))

### Spec

- **category-workflow**: Add gap 5 --no-bump/--skip-revision for silent bulk edits ([86f08bc](https://github.com/koloki-co/dsc/commit/86f08bc2ddbdacde79f71ae003a3f4ca15c03266))

- **category-workflow**: Update for YAML front matter (not HTML comments); mark gaps 1-4 implemented; add gap 4 admonition/URL conversion ([2113c35](https://github.com/koloki-co/dsc/commit/2113c35fa6e0196131cf5676a8e7a545a04d7ab1))

- Category pull/push workflow gaps (field-driven, forum.rcpch.tech) ([7254a01](https://github.com/koloki-co/dsc/commit/7254a0118d34198950dce2ee5419812b5d46248d))

## [0.10.15] - 2026-06-17

### Documentation

- **spec**: Audit downstream code for negative user-id impact ([bb18cd6](https://github.com/koloki-co/dsc/commit/bb18cd6aa8d04b08f9e0b2ca0b16f1d22d73e823))

### Refactor

- **utils**: Use slug crate for slugify, handles Unicode ([690b321](https://github.com/koloki-co/dsc/commit/690b3210ad8b523172b5e706e87130027d5c988e))

## [0.10.14] - 2026-06-17

### Bug fixes

- **user**: Tolerate negative IDs for Discourse system accounts ([d3c6d55](https://github.com/koloki-co/dsc/commit/d3c6d5516ac912ebabdbcda5607e07a5c05b1988))

## [0.10.13] - 2026-06-10

### Features

- **cli**: Add 'dsc man' for generating Unix man pages ([77d20ed](https://github.com/koloki-co/dsc/commit/77d20eda89c26b3d07699f83adfa62882624094b))

## [0.10.12] - 2026-06-10

### Bug fixes

- **cli**: Bring 6 empty-list + 1 error message in line with spec ([bab77a7](https://github.com/koloki-co/dsc/commit/bab77a7536a3a8e2a42a480ede4622c471660aa3))

### Documentation

- Pre-1.0 polish batch (CHANGELOG, CONTRIBUTING, issue templates) ([038d3c7](https://github.com/koloki-co/dsc/commit/038d3c7285061af3a1c7b631842e93bf8d8a491d))

## [0.10.11] - 2026-06-10

### Documentation

- **spec**: Merge .marcus notes + integrate field-driven specs ([51b6fc7](https://github.com/koloki-co/dsc/commit/51b6fc75f5e6139311b2df60aa4fbc320250be7e))

- **roadmap**: Note git-cliff as recommended changelog tool ([0364d28](https://github.com/koloki-co/dsc/commit/0364d2869e1826351c34644863a1134fcdebe0ca))

- **roadmap**: Add pre-1.0 launch checklist ([9cacf84](https://github.com/koloki-co/dsc/commit/9cacf84121002c8490de3cafd989d0618cfe77a9))

- Rename agents.md to AGENTS.md ([ff49b3f](https://github.com/koloki-co/dsc/commit/ff49b3f246476bb42f0d3cabb30ab7e390828eca))

- Add agents.md - guide for LLMs using dsc in other sessions ([5390ccb](https://github.com/koloki-co/dsc/commit/5390ccb2ab5ec7b74141e62a88dc94f7f4673f1d))

- Accuracy pass + roadmap cleanup ([2fec564](https://github.com/koloki-co/dsc/commit/2fec56453d303e17cefbe6ddf8c104274b1d4586))

### Features

- **topic**: Add 'dsc topic pull --full' for whole-thread export ([3bda807](https://github.com/koloki-co/dsc/commit/3bda80786cab0d2e2cf9a97fcaa8eb33a3f53842))

## [0.10.10] - 2026-06-09

### Features

- **tag**: Add 'dsc tag rename' preserving topic associations ([65b2a65](https://github.com/koloki-co/dsc/commit/65b2a65d8d2f6277ccec9aeedd0c71406bb00ba5))

## [0.10.9] - 2026-06-09

### CI / dependencies

- **deps**: Bump checkout v6.0.3, upload-artifact v7.0.1, download-artifact v8.0.1 ([d66c74c](https://github.com/koloki-co/dsc/commit/d66c74cdc9ee8425de0419a3eb428c0778bbf832))

### Documentation

- **roadmap**: Mark setting-sync (Phases 1-4) as completed ([1f3a318](https://github.com/koloki-co/dsc/commit/1f3a31805fabc915244a270bdce11a025ebabdc9))

### Features

- **config**: Add $DSC_CONFIG and $DSC_CONFIG_HOME resolution ([216b848](https://github.com/koloki-co/dsc/commit/216b848b7c54be0c105a97b784b5ca91e97092e8))

- **tag**: Add declarative pull/push spec for managing tag taxonomy ([13aa024](https://github.com/koloki-co/dsc/commit/13aa02490cc7688ec6dbeb02d71ddc8d1332956a))

- **harden**: Enhance SSH algorithm checks to prevent weak crypto usage ([979c3d1](https://github.com/koloki-co/dsc/commit/979c3d1c8d1b9add9310f7e50e56644a67e0f7d7))

## [0.10.8] - 2026-06-07

### Features

- **setting**: Add 'dsc setting diff' for cross-source comparison (Phase 3) ([603a58e](https://github.com/koloki-co/dsc/commit/603a58e8968d808ca850c989287626d88bb2b5fe))

## [0.10.7] - 2026-06-07

### Features

- **setting**: Add 'dsc setting push' for idempotent apply (Phase 2) ([edaa0ad](https://github.com/koloki-co/dsc/commit/edaa0ad62c479069b1bc76666877b6d6aa48276a))

## [0.10.6] - 2026-06-07

### Features

- **setting**: Add 'dsc setting pull' for declarative snapshots (Phase 1) ([9d48885](https://github.com/koloki-co/dsc/commit/9d48885d8861bebcb142f849902e505cd6dd9e91))

## [0.10.5] - 2026-06-07

### Bug fixes

- Add cooldown configuration for dependencies in dependabot.yml ([725cf6d](https://github.com/koloki-co/dsc/commit/725cf6d0cac8584d56a797b97baad9703a894999))

- Support rootless Docker in dsc update ([c3db942](https://github.com/koloki-co/dsc/commit/c3db942a984326132581916b73f68371871edadb))

- TagInfo.id is u64, use text field for tag names ([a889d5a](https://github.com/koloki-co/dsc/commit/a889d5a148f850854ad28caf17c744035cf09314))

### Chores

- Pin GitHub Actions to commit SHAs (supply-chain security) ([b0e7823](https://github.com/koloki-co/dsc/commit/b0e782344c6f1c9dc6b581060bbdd9c8668d3629))

- Bump version to 0.10.4 ([9b2a170](https://github.com/koloki-co/dsc/commit/9b2a17079409a6558758bda29c88ad6f3873c5b2))

### Documentation

- Fix setting.md inaccuracies and reference bulk pull/push spec ([1884f52](https://github.com/koloki-co/dsc/commit/1884f52ac9fdfaa6059bfe53ffc32f26ec362df9))

- **harden**: Refine SSH configuration details and clarify algorithm policies ([01f1287](https://github.com/koloki-co/dsc/commit/01f12879468a6366722ffd935e1a37baf6afc938))

- **dsc.example.toml**: Update SSH algorithm comments for clarity and accuracy ([cad6d66](https://github.com/koloki-co/dsc/commit/cad6d66783449fbb4f7f607b78c497f307f1a5db))

- **index**: Consolidate dsc-rs naming note into the Cargo tab ([e5d67bc](https://github.com/koloki-co/dsc/commit/e5d67bc8ff85635f9b5ac5ee96fd1c67cd34f06f))

- **index**: Add platform icons to install tabs ([e7c3e0d](https://github.com/koloki-co/dsc/commit/e7c3e0d4b7479c7cf9fb98e8a4bc8ef8218cccca))

- **index**: Convert install section to content tabs ([8255a93](https://github.com/koloki-co/dsc/commit/8255a9352c19d2556290cd462b2a28ac00b03c06))

- Move top-level nav from header tabs to left sidebar ([9cef00f](https://github.com/koloki-co/dsc/commit/9cef00faa83fda17ef8132e29de937b5115080a9))

- Scheme-conditional logo + 2× size ([1d61171](https://github.com/koloki-co/dsc/commit/1d61171c39b61762edcc4321de722c1a173042ec))

- Switch to Zensical modern variant + brand-orange accent ([d277823](https://github.com/koloki-co/dsc/commit/d27782346366cb1138e89c988973232bc277dbd0))

- Add analytics + harden to nav, enable dark-mode toggle ([cfa4d30](https://github.com/koloki-co/dsc/commit/cfa4d3023914bd3f427c322dd55c402b172ed7c7))

### Features

- **setting**: Make set --tags reachable from CLI (Phase 4) ([04f3fc1](https://github.com/koloki-co/dsc/commit/04f3fc1b6cd9bde1854ec6aef26a8ee4ee9b6ea4))

- Declarative tag taxonomy pull/push, move topic tagging to dsc topic ([e61b531](https://github.com/koloki-co/dsc/commit/e61b53148a8b408f12e9cd1cbbc3862c23c06b6a))

- Enhance config command to display active config and search order ([3d53c1a](https://github.com/koloki-co/dsc/commit/3d53c1aaf18554b5383b20eb15f0ae7a5dd9fd1c))

- Harmonise post, backup, emoji with pull/push pattern ([cfc7826](https://github.com/koloki-co/dsc/commit/cfc782628938bebf39f97706c5237ffea716f181))

- **harden**: Stage 2 — sshd tightening + ssh.socket patch ([010a3d8](https://github.com/koloki-co/dsc/commit/010a3d8ee6b9046266f7c84e9b0bc043371ab0f5))

### Spec

- Setting sync (bulk pull/push) and project roadmap ([46019d8](https://github.com/koloki-co/dsc/commit/46019d8dda0ca81970f4400cf7f09fe20ccf28bb))

## [0.10.3] - 2026-04-27

### Bug fixes

- **utils**: `1m` means 1 month, not 1 minute ([1442875](https://github.com/koloki-co/dsc/commit/1442875378b69c0b4325a7c72c6e39609df62c23))

## [0.10.2] - 2026-04-27

### Features

- **analytics**: --format table + --snapshot multi-window mode ([b021c3f](https://github.com/koloki-co/dsc/commit/b021c3ffdd9c8e5d6ed1f62cda0eccf93a940df8))

## [0.10.1] - 2026-04-27

### Bug fixes

- **analytics**: Stacked-chart aggregation + new_contributors wiring ([d6c83d1](https://github.com/koloki-co/dsc/commit/d6c83d14f83697309e6de43eea375b4ed42d52de))

## [0.10.0] - 2026-04-27

### Build

- **docs**: Serve install.sh and install.ps1 proxies from the docs site ([3672df4](https://github.com/koloki-co/dsc/commit/3672df4eb26b7fbfe2f255d9a2969e7ebfdfd6bd))

### Features

- **analytics**: Implement spec/analytics.md (v1) ([0ea3407](https://github.com/koloki-co/dsc/commit/0ea34076f3dbb24c15e68eeeaf5ec8045decc999))

- **harden**: Config block + flag override, SECURITY.md, docs ([8b8a994](https://github.com/koloki-co/dsc/commit/8b8a994180121465d77e15c31d41ccd0454bf3fa))

- **harden**: Stage 1 — user creation + pubkey install + self-lockout guard ([e49ce15](https://github.com/koloki-co/dsc/commit/e49ce15ed551f5e161dce3276442e242bc40c1bc))

### Spec

- Add analytics command spec ([07a8ff0](https://github.com/koloki-co/dsc/commit/07a8ff0a95e78b2278926bba62f3ab000144d1df))

## [0.9.0] - 2026-04-21

### Features

- Complete Phase 2 — user create, password-reset, email-set ([470b1a8](https://github.com/koloki-co/dsc/commit/470b1a8590c0cc7cb7b11b64db076c4040f944db))

## [0.8.3] - 2026-04-21

### Build

- **dist**: Add Homebrew tap, PowerShell, and MSI installers ([3676476](https://github.com/koloki-co/dsc/commit/367647628c313c09ff55653b49e12050d7359b61))

### CI / dependencies

- **deps**: Bump action pins (consolidates #11, #12, #13) ([8086b26](https://github.com/koloki-co/dsc/commit/8086b26e6b5238eafe06e61ff542c47c595380d2))

### Documentation

- **s/docs**: Detect inotify saturation and print a fixable error ([5657e84](https://github.com/koloki-co/dsc/commit/5657e84e732f411d7726c3b8a8b512bbaf095016))

- Make s/docs surface the inotify gotcha before it bites ([ea39481](https://github.com/koloki-co/dsc/commit/ea39481a36fbc87b38760c0ac959489ce9966ebf))

- Add Zensical site with GitHub Pages deploy ([be4ac2a](https://github.com/koloki-co/dsc/commit/be4ac2a6c7fb6a5e8b045dbbd3dde44e2ea07bfc))

### Features

- **docs**: Add initial bash script to serve Zensical ([a8a4501](https://github.com/koloki-co/dsc/commit/a8a45017e439608a68242f1586a71918db462303))

## [0.8.2] - 2026-04-19

### Bug fixes

- Update forum references in topic commands for user activity examples ([74c6864](https://github.com/koloki-co/dsc/commit/74c6864cea5fcfd811f060084472554f59f1ee6b))

### Features

- **user activity**: Work without an API key for public forums ([182a0e4](https://github.com/koloki-co/dsc/commit/182a0e48648d112da87b469311a6bf1b4435e057))

## [0.8.1] - 2026-04-19

### Features

- Dsc user activity — archive public activity to a journal forum ([32868ac](https://github.com/koloki-co/dsc/commit/32868ac9f55a934c413334af0f189cd87838ec24))

## [0.8.0] - 2026-04-19

### Features

- Dsc pm send + list (Phase 3 starter) ([4173260](https://github.com/koloki-co/dsc/commit/4173260637049bca6d8d449e88328642fa13d0d2))

## [0.7.0] - 2026-04-19

### Features

- Dsc api-key list / create / revoke ([61870e5](https://github.com/koloki-co/dsc/commit/61870e5a5b02c311b7d1618f12dd78d7f02249bc))

## [0.6.0] - 2026-04-19

### Features

- Invites + user moderation toolkit (silence, promote, demote) ([98437ae](https://github.com/koloki-co/dsc/commit/98437ae06a6fe510599843988bd7180b6578daad))

## [0.5.0] - 2026-04-19

### Features

- Phase 2 start — dsc user list / info / suspend / unsuspend ([add8057](https://github.com/koloki-co/dsc/commit/add8057b226e8d6c67cd03dd987ad5aad2d00747))

## [0.4.0] - 2026-04-19

### Chores

- Stop vendoring generated shell completions ([c19224a](https://github.com/koloki-co/dsc/commit/c19224a6c74d055a388f5227681023de9741397c))

### Features

- Phase 1 remainder — post ops, group/user membership, full dry-run ([8b959ff](https://github.com/koloki-co/dsc/commit/8b959fffbd2292be2c0ee36d1aed83a581a83ec9))

- Add search, tag, and upload commands with documentation ([27a458a](https://github.com/koloki-co/dsc/commit/27a458aac51fb1c8342a6c31f3555b35836d504f))

## [0.3.0] - 2026-04-17

### Features

- Phase 0 — foundations, new commands, and retry/config/dry-run ([9d7999b](https://github.com/koloki-co/dsc/commit/9d7999b1176bd986ff39c30cc07fd034a43f73cf))

## [0.2.1] - 2026-04-10

### Chores

- Upgrade cargo-dist to 0.31.0 and regenerate release.yml ([40d4dfb](https://github.com/koloki-co/dsc/commit/40d4dfbbfb89f3c08143766c74a00ac5eab32717))

## [0.2.0] - 2026-04-10

### CI

- Add crates.io publish workflow ([4886464](https://github.com/koloki-co/dsc/commit/4886464a5bba2987cf63f098fc63eccdbf62b1ba))

### Chores

- Rename crate to dsc-rs and add crates.io metadata ([07d4f1e](https://github.com/koloki-co/dsc/commit/07d4f1e24c29de29fbc9a4ffe3c1b66184855dc0))

### Features

- **cli**: Add abbreviated aliases for all subcommands ([36c0846](https://github.com/koloki-co/dsc/commit/36c08462ce9883e4e8d1bfee7081a9369b92e8f2))

- Add FUNDING.yml enable GitHub Sponsors ([42c91a5](https://github.com/koloki-co/dsc/commit/42c91a5339faede92f451a9a008df76a03654264))

- Add theme management commands for pull, push, and duplicate ([70993e4](https://github.com/koloki-co/dsc/commit/70993e49b57b192b512ddcbece15e3ed43f71664))

- Enhance site settings management in dsc CLI ([695334a](https://github.com/koloki-co/dsc/commit/695334a0772ac7fa7246022a958afb9a03c2bd2f))

## [0.1.6] - 2026-03-04

### CI / dependencies

- **deps**: Bump actions/upload-artifact from 6 to 7 ([d8f15f2](https://github.com/koloki-co/dsc/commit/d8f15f2021d2e834abc3691fad2092b0d89e4cef))

- **deps**: Bump actions/download-artifact from 7 to 8 ([0caf332](https://github.com/koloki-co/dsc/commit/0caf332535ed7848eaa321fc4e8ac3b457cdd266))

- **deps**: Bump actions/download-artifact from 4 to 7 ([a9f78e7](https://github.com/koloki-co/dsc/commit/a9f78e75b1616999cea4def18af798a187502aeb))

- **deps**: Bump actions/checkout from 4 to 6 ([dc06aed](https://github.com/koloki-co/dsc/commit/dc06aed40dcaf7ac8a9e79af915d2ecde1b72cb0))

- **deps**: Bump actions/upload-artifact from 4 to 6 ([36dd5a3](https://github.com/koloki-co/dsc/commit/36dd5a3d8ea93be53368f101ec58f60bc4570203))

- **deps**: Update toml requirement from 0.9 to 1.0 ([db03400](https://github.com/koloki-co/dsc/commit/db034000e5231ae0cb1eeeefef4d5d07e63f0d59))

### Features

- Bump version to 0.1.5; enhance CLI help text for commands and flags ([2f02c7e](https://github.com/koloki-co/dsc/commit/2f02c7ec634f9ff997780c0d5deaef582fd5913c))

## [0.1.5] - 2026-03-03

### Features

- Bump version to 0.1.4 and update dependencies; enhance update command flags and documentation ([209f3d5](https://github.com/koloki-co/dsc/commit/209f3d5abf8b6c5edec27399fdc771fa26a98631))

- Add version bump script for automated tagging ([4e1c51c](https://github.com/koloki-co/dsc/commit/4e1c51c97dbe289840641c05ac177b45853d7666))

## [0.1.3] - 2026-03-03

### Chores

- Update indicatif dependency to version 0.18 ([78d86b3](https://github.com/koloki-co/dsc/commit/78d86b3584702d892b38ca454bbd522c9a59cb1f))

- Regenerate cargo-dist release workflow ([c7742ef](https://github.com/koloki-co/dsc/commit/c7742eff6e3d1ba84ac5fa987fc70d6d67cbb07c))

### Documentation

- Update README with new environment variables for `dsc update` and name recommendations ([73d15a1](https://github.com/koloki-co/dsc/commit/73d15a173f374eff42c4acf4dc514757c91131a2))

### Features

- Add --yes flag to update commands for auto-confirming changelog posts ([bc409c2](https://github.com/koloki-co/dsc/commit/bc409c201caf69820e22a01227a30226d934d707))

- Enhance update checklist with detailed versioning and disk usage information ([2d3d9b6](https://github.com/koloki-co/dsc/commit/2d3d9b6c439ebfaf5535727ea9f61e1f1f158264))

- Add dynamic discourse completion to zsh scripts and improve update command feedback ([3541c1f](https://github.com/koloki-co/dsc/commit/3541c1f10dcb3a25a1465c2efad35d31618ab30b))

## [0.1.2] - 2026-02-01

### Bug fixes

- Mark changelog path and interactive prompt decisions as complete ([b0adc9c](https://github.com/koloki-co/dsc/commit/b0adc9ce29b914cd9a52ec39008930c06db7f58e))

- Broaden emoji list parsing ([d4edc5f](https://github.com/koloki-co/dsc/commit/d4edc5f6373650ad21768f214ff8241e7f5ef36e))

- Remove duplicate config module ([8a475a9](https://github.com/koloki-co/dsc/commit/8a475a9c3290ed2755d85a4c5e28c0f9fb9457b3))

### Documentation

- Reorganize roadmap and merge todo ([b53fcbc](https://github.com/koloki-co/dsc/commit/b53fcbc9aa8d2cdb420435e702f879c593ea795a))

- Drop incorrect add prompt note ([443c0e3](https://github.com/koloki-co/dsc/commit/443c0e3767c79845101909c60dde993631e569a3))

### Features

- Remove update-all logging ([fb7f4d5](https://github.com/koloki-co/dsc/commit/fb7f4d5b84a5ce27a64f9a6416b542f8cecec8a7))

- Add inline emoji listing ([4f2abdd](https://github.com/koloki-co/dsc/commit/4f2abdd23aa00ca7942cd62f800d81956a2130ed))

- Add theme management commands ([5910532](https://github.com/koloki-co/dsc/commit/5910532ffae9c54d1e4aa7e1c2be5819a843780e))

- Add plugin management commands ([9420b3d](https://github.com/koloki-co/dsc/commit/9420b3d1555cada2fbbb9fd95ec86c11a5a50b8a))

- Add palette commands ([e112ee3](https://github.com/koloki-co/dsc/commit/e112ee3bbbc9a5a3a21fab688d2f126b8d45c414))

- Improve backup list and os update handling ([b73cecd](https://github.com/koloki-co/dsc/commit/b73cecd03585d2f0d3ecf2533db311c13ac01c55))

- Add site setting updates and format options ([4f3db77](https://github.com/koloki-co/dsc/commit/4f3db77aac555db7236940320f559f6e1bd6890c))

- Add site setting update helper ([4b8eb21](https://github.com/koloki-co/dsc/commit/4b8eb210cda74dfde593b618b09305e458d960df))

- Enhance CLI with tag filtering and emoji upload improvements ([dfc0268](https://github.com/koloki-co/dsc/commit/dfc02681f6c274da53c40723f8ad57b882961f86))

### Refactor

- Modularize cli and api code ([d02e0ff](https://github.com/koloki-co/dsc/commit/d02e0ff510989603c7f9a1f85b4bb4d93cc20a09))

### Tests

- Add common module to new tests ([b69a254](https://github.com/koloki-co/dsc/commit/b69a2544c1549cd7b32a84dae689eb9b7c8047a6))

- Add completions e2e and refresh scripts ([333d2bb](https://github.com/koloki-co/dsc/commit/333d2bb2c76de5f84c2cf838da7c25b4750fbd01))

## [0.1.1] - 2026-01-30


