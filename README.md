# ASOMaster MCP server

`asomaster-mcp` connects AI agents (Claude Code, Codex, Cursor, Claude Desktop, Windsurf, OpenCode or any
stdio MCP client) to [ASOMaster](https://asomaster.app), a desktop ASO app for the App Store and Google Play.

Your agent can read keyword boards, check ranks, find and localize keywords, fetch real Apple Ads or
Google Ads popularity, watch rivals and run the listing studio, with every change showing up live in the
app. The bridge talks to the app over a private local socket (a named pipe on Windows); nothing goes
through our servers.

Full docs: <https://asomaster.app/docs/mcp/> (Markdown for agents: <https://asomaster.app/docs/mcp.md>)

## Install

`asomaster-mcp` ships inside the ASOMaster app (this repository is its source). [Download ASOMaster](https://asomaster.app) for macOS
(Windows beta), activate a license, then either:

- **From the app:** Settings › MCP clients › Install, for Claude Code, Codex, OpenCode, Cursor, Claude
  Desktop or Windsurf.
- **By hand:** point your client at the bridge.

| System  | Bridge path |
|---------|-------------|
| macOS   | `/Applications/ASOMaster.app/Contents/MacOS/asomaster-mcp` |
| Windows | `%LOCALAPPDATA%\ASOMaster\asomaster-mcp.exe` |

Claude Code:

```sh
claude mcp add --scope user asomaster -- /Applications/ASOMaster.app/Contents/MacOS/asomaster-mcp
```

Cursor, Claude Desktop, Windsurf and other JSON configs ([examples/mcp.json](examples/mcp.json)):

```json
{
  "mcpServers": {
    "asomaster": {
      "command": "/Applications/ASOMaster.app/Contents/MacOS/asomaster-mcp",
      "args": []
    }
  }
}
```

Codex (`~/.codex/config.toml`):

```toml
[mcp_servers.asomaster]
command = "/Applications/ASOMaster.app/Contents/MacOS/asomaster-mcp"
args = []
```

The app doesn't need to be open; the bridge starts it. Supported MCP protocol versions: 2024-11-05,
2025-03-26, 2025-06-18 and 2025-11-25.

## What agents can do

- **Read:** apps, keyword boards per market, keyword detail (rank history, apps ranking), rank alerts,
  audit score, ratings, rival listing changes (`get_overview`, `get_project_board`, `get_term_detail`,
  `list_rank_alerts`, `list_rival_changes`, `get_audit`, `get_ratings`).
- **Keywords:** suggest, add, localize, edit and remove keywords; popular App Store searches around a
  seed word (`add_keywords`, `edit_keyword`, `popular_searches`).
- **Demand:** real popularity from Apple Ads or Google Ads Keyword Planner, kept separate from
  ASOMaster's own estimates (`connect_popularity`, `get_popularity_status`).
- **Listings:** draft, compare, choose and localize titles, subtitles, keyword fields and descriptions;
  preview changes (`get_optimizer`, `get_publish_preview`).
- **Jobs:** store searches and AI runs are queued; `wait_for_jobs` waits until they finish.

## Safety

- Publishing to App Store Connect or Google Play needs your go-ahead twice: the agent must quote your
  approval, and the app shows a native confirmation only you can answer. Nothing is submitted for review.
- Screenshot uploads can only be started in the app.
- Credentials are passed as file paths, stored in the OS keychain and never returned by a tool.
- Local only: owner-only Unix socket on macOS, per-user named pipe with a token on Windows.

## Source, Linux build and Docker

This repository holds the bridge's source (Rust, MIT). `cargo build --release` builds it; `cargo test`
runs its tests. The [Dockerfile](Dockerfile) builds and runs it so MCP directories can check that the
server starts and lists its tools. ASOMaster itself runs on macOS and Windows; without the app the
bridge still answers `initialize` and `tools/list`, and every tool call returns `app_unavailable`.

## Requirements

ASOMaster with an active license (subscription: 15.99 EUR per month or 49 EUR per year). Without one,
tools answer `license_required`.

## Support

Questions and bug reports: open an issue here or see <https://asomaster.app/support/>.

ASOMaster is made by Cressu OÜ.
