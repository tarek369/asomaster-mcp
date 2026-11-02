//! The `instructions` string sent in the `initialize` result.

pub const INSTRUCTIONS: &str = r#"ASOMaster is the owner's desktop App Store Optimization workspace for the Apple App Store and Google Play. These tools drive the running app (it is started automatically when closed). Everything you change appears live in the owner's app.

WORKFLOW: inspect -> act -> wait -> read.
1. Inspect: get_overview first (project ids, markets, running jobs, license). Then get_project_board for a project (summarised per market), get_term_detail for one keyword in one market, get_optimizer for listing work.
2. Act: tools that do store or AI work (add_app, run_rank_check, fetch_popularity, suggest_keywords, localize_keyword, brainstorm_project, optimizer_start / _more_options / _localize, ask_assistant, import_file, publish_listing) only QUEUE a background job and return immediately.
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

SAFETY:
- NEVER call publish_listing unless the owner has seen the final listing text and explicitly told you to publish it in this conversation. Publishing changes the live store page that real users see. Quote their approval in the approval field.
- Ask before destructive tools (remove_project, remove_keywords, remove_folder, optimizer_reset, disconnect_console, disconnect_popularity, clear_assistant, acknowledge_alerts with clear).
- For credentials prefer file paths (privateKeyPath, serviceAccountPath); secrets are stored in the OS keychain and are never returned. Do not echo secrets back to the user.
- The stores rate-limit; large checks take minutes. If a job reports pausedUntil, the app is backing off on purpose — keep waiting.

ERRORS come back as {error:{code,message,action?},hint}. license_required / feature_not_licensed: the owner must activate or upgrade in the app - stop and tell them. ai_not_configured: see get_ai_status. console_not_connected: connect_console. app_unavailable: ask the owner to open ASOMaster."#;
