//! The `instructions` string sent in the `initialize` result.

pub const INSTRUCTIONS: &str = r#"ASOMaster is the owner's desktop App Store Optimization workspace for the Apple App Store and Google Play. These tools drive the running app (it is started automatically when closed). Everything you change appears live in the owner's app.

WORKFLOW: inspect -> act -> wait -> read.
1. Inspect: get_overview first (project ids, markets, running jobs, license). Then get_project_board for a project (summarised per market), get_term_detail for one keyword in one market, get_optimizer for listing work.
2. Act: tools that do store or AI work (add_app, run_rank_check, fetch_popularity, suggest_keywords, localize_keyword, brainstorm_project, optimizer_start / _more_options / _localize, ask_assistant, import_file, run_audit, publish_listing, upload_app_name, publish_pricing, screenshots_upload) only QUEUE a background job and return immediately.
3. Wait: call wait_for_jobs (<= 50 s per call) and repeat until idle is true. Do not poll get_overview in a tight loop. stop_job cancels.
4. Read the results again (board / optimizer / assistant thread) before deciding the next step.

DATA YOU MUST KEEP APART:
- estVol / estDiff are ASOMaster ESTIMATES (0-100) modelled from store search results. They exist for every checked keyword.
- pop is REAL popularity (0-100) from a connected source (Apple Ads, Google Ads Keyword Planner or an imported file). It only exists after connect_popularity + fetch_popularity. Never present an estimate as real popularity or vice versa; say which one you are using.
- rank = position in store search for that market; absent = not ranked in the checked results.
- Markets are lower-case country codes ("us", "de"); listing locales look like "en-US". Apple and Google projects are separate even for the same app.

TYPICAL TASKS:
- Track an app: search_store -> add_app -> wait_for_jobs -> get_overview -> run_rank_check -> wait_for_jobs -> get_project_board.
- Find keywords: suggest_keywords or add_keywords (section idea) -> run_rank_check -> compare estVol / estDiff / pop / rank / opp, then promote winners with add_keywords section listing or hide losers with edit_keyword.
- Improve the listing: optimizer_start -> wait -> get_optimizer -> (optimizer_save_option to tweak) -> optimizer_choose -> optimizer_localize -> wait -> review drafts (optimizer_edit_locale / optimizer_pick_phrase) -> show the owner the exact text.
- Real searches from Apple Ads (own App Store apps with campaigns, Apple Ads connected): get_search_terms (days 7/30/90, market) -> add_keywords with market for the terms worth tracking.
- Audit the app: run_audit (options: countries, refetch, moreKeywords, recheck, note) -> wait_for_jobs -> get_audit (country for one market in full); its issues and takeaways name the step that fixes them.
- Did a change work? get_change_impact (days 7 / 14 / 28; eventId for one change): ranks, store traffic and ratings before vs after each publish, listing edit, version or owner marker, per market. Say it shows what happened around the change, not proof of cause, and mention overlapping changes.
- Reviews (own apps with a connected console): sync_reviews -> wait_for_jobs -> get_reviews (filters: stars, market, replied, search) -> draft_review_reply for the ones worth answering -> wait_for_jobs -> get_reviews shows the draft. Only the owner can send a reply, in the app; never claim a reply was posted. Add good review phrases from ideas with add_keywords (section idea; market = the idea's country when it is a tracked market other than the main one, so the phrase is tracked there as written).
- Local prices (own apps with a connected console): get_pricing -> plan_pricing (select products, method, base price, floor / ceiling, overrides, start date) -> show the owner the per-country before / after -> publish_pricing only on their explicit go-ahead -> wait_for_jobs -> get_pricing (lastRun).

SAFETY:
- NEVER call publish_listing unless the owner has seen the final listing text and explicitly told you to publish it in this conversation. Publishing changes the live store page that real users see. Quote their approval in the approval field.
- NEVER call publish_pricing unless the owner has seen the per-country prices and explicitly told you to publish them in this conversation. Real customers pay these prices. Quote their approval in the approval field; the app asks them to confirm again.
- NEVER call screenshots_upload unless the owner has seen the rendered screenshots (get_screenshots / screenshots_export) and told you to upload them in this conversation; the app asks them to confirm. dryRun checks without changing the store.
- A name App Store Connect refused (get_optimizer renames) is never replaced automatically: show the owner the proposals, and call upload_app_name only with the exact name they picked in this conversation (quote it in approval; the app asks them to confirm).
- Ask before destructive tools (remove_project, remove_keywords, remove_folder, optimizer_reset, disconnect_console, disconnect_popularity, clear_assistant, acknowledge_alerts with clear).
- For credentials prefer file paths (privateKeyPath, serviceAccountPath); secrets are stored in the OS keychain and are never returned. Do not echo secrets back to the user.
- The stores rate-limit; large checks take minutes. If a job reports pausedUntil, the app is backing off on purpose — keep waiting.
- A job with aiPause is waiting for the AI's usage limit; it resumes by itself at aiPause.resumesAt (resume_job retries now or switches AI).

ERRORS come back as {error:{code,message,action?},hint}. license_required / feature_not_licensed: the owner must activate or upgrade in the app - stop and tell them. ai_not_configured: see get_ai_status. console_not_connected: connect_console (error.details present: a Play account is connected but lacks the package; pass store google, accountId, packageNames, no key). app_unavailable: ask the owner to open ASOMaster."#;
