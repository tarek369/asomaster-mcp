# ASOMaster for Claude

Work with [ASOMaster](https://asomaster.app), the desktop App Store Optimization app for the App Store
and Google Play, from Claude Code and the Claude desktop app. Ask Claude to check where your app ranks,
find and translate keywords, read real search popularity from Apple Ads or Google Ads, run an audit of
your store listing, draft listings in every language, answer reviews or plan local prices. Every change
shows up live in the app, and your data stays on your computer.

## What you need

- ASOMaster installed: macOS from [asomaster.app](https://asomaster.app), or the Windows beta.
- An active ASOMaster license (15.99 EUR a month or 49 EUR a year). Without one, every tool answers
  `license_required`.

The app doesn't have to be open: the plugin starts it when Claude first uses a tool.

## Install

In Claude Code:

```
/plugin install asomaster --marketplace tarek369/asomaster-mcp
```

On older Claude Code versions, add the marketplace first, then install:

```
/plugin marketplace add tarek369/asomaster-mcp
/plugin install asomaster@asomaster
```

## What the plugin runs

The plugin adds one MCP server, `asomaster`. Its launcher, `bin/asomaster-mcp.sh`, looks for the
`asomaster-mcp` bridge that ships inside the ASOMaster app and runs it:

| System  | Bridge it runs |
|---------|----------------|
| macOS   | `/Applications/ASOMaster.app/Contents/MacOS/asomaster-mcp` (or the same under `~/Applications`) |
| Windows | `%LOCALAPPDATA%\ASOMaster\asomaster-mcp.exe` |

The launcher downloads nothing and changes nothing on your computer. If the app isn't installed, it
prints where to get it and stops. The bridge's source code is the `src/` folder of
[this repository](https://github.com/tarek369/asomaster-mcp) (Rust, MIT).

The bridge talks to the app over a private local connection: an owner-only Unix socket on macOS, a
per-user named pipe with a token on Windows. It opens no network connection of its own.

## What Claude can do with it

- **Read:** your apps, keyword boards per country, rank history, rank alerts, ratings, the audit
  score and rivals' listing changes.
- **Keywords:** suggest, add, translate, edit and remove keywords, and see popular App Store
  searches around a word.
- **Demand:** real popularity from Apple Ads or Google Ads Keyword Planner, kept apart from
  ASOMaster's own estimates.
- **Listings:** draft, compare and choose titles, subtitles, keyword fields and descriptions, translate
  them into every store language, review the translations and preview what a publish would change.
- **Reviews and prices:** read and draft replies to reviews, and plan local prices per country.
- **Screenshots:** edit slides, write and translate headings, render and export them.

Store searches and AI work run as background jobs in the app; Claude waits for them with
`wait_for_jobs`.

## Safety

- Anything that changes your live store page (publishing a listing, prices, screenshots or a new app
  name) needs your go-ahead twice: Claude must pass your explicit approval, and the app then shows a
  confirmation window that only you can answer.
- On the App Store nothing is submitted for review. On Google Play, changes are kept for you to send
  for review, except for apps whose changes Play always sends for review automatically.
- Store and ads credentials are passed as file paths, kept in your system keychain and never returned
  by a tool.

## Privacy Policy

The full policy is at <https://asomaster.app/privacy/>. In short, for this plugin:

- **Data collection:** the plugin and its bridge collect nothing themselves. They pass Claude's tool
  requests to the ASOMaster app on your computer and return its answers to Claude.
- **Usage and storage:** your projects, keywords, rank history, listing drafts and settings live in a
  database on your computer. Cressu OÜ has no copy. What a tool returns becomes part of your Claude
  conversation, which Anthropic handles under its own terms.
- **Third-party sharing:** the app itself contacts the services you use it with: the public App Store
  and Google Play for rank checks, App Store Connect, Google Play Console, Apple Ads and Google Ads with
  the credentials you connect, the AI provider you pick for AI tasks, and our license server to keep
  your license valid (purchase email, a random device identifier, computer name, platform and app
  version). The bridge sends nothing to these services itself.
- **Data retention:** your local data stays until you delete it in the app or remove the app. License
  and device records are kept while your license exists, then as long as accounting law requires
  (in Estonia, generally 7 years).
- **Contact:** Cressu OÜ, Uus-Sadama tn 21-207, 10120 Tallinn, Estonia, <asomaster@cressu.com>.

## Support

Docs: <https://asomaster.app/docs/mcp/>. Questions and bug reports:
<https://asomaster.app/support/> or an issue in
[tarek369/asomaster-mcp](https://github.com/tarek369/asomaster-mcp/issues).

ASOMaster is made by Cressu OÜ. This plugin is MIT licensed (see `LICENSE`).
