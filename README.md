# LocalShell

**A modern terminal with built-in AI, for workplaces that can't let a terminal app send code or
data to outside services.**

LocalShell is a fork of the open-source [Warp](https://github.com/warpdotdev/warp) terminal. It
keeps the terminal and its AI features, but removes the cloud. There's no account, no telemetry,
no crash reporting and no sync. The AI runs on a model **you** choose: one on your own Mac through
LM Studio, or an OpenAI-compatible endpoint you're allowed to use.

> [!NOTE]
> LocalShell is an independent project. It is not affiliated with or endorsed by Denver
> Technologies, Inc., the company behind Warp. It is an early release for **macOS on Apple
> Silicon**.

## Why this exists

Plenty of developers would like to use Warp at work and can't. Out of the box, Warp sends your
prompts, terminal output and files to Warp's servers, which pass them on to AI providers. It asks
you to sign in, sends usage telemetry and crash reports, and syncs data to Warp's cloud. Under
GDPR, ISO 27001, SOX or a company data-protection policy, every one of those is a data flow a
security team has to assess, and often the answer is no.

LocalShell removes those flows instead of asking you to trust them. The result is something a
security team can actually review: **the app talks to one host, and you choose that host.**

## What leaves your Mac

| Data | Warp | LocalShell |
|---|---|---|
| Prompts, commands, output and files sent to AI | Warp's servers, then their model providers | Only the endpoint you configure. With LM Studio it never leaves the Mac. |
| Account and sign-in | Warp account (Firebase) | None. No account exists. |
| Usage telemetry | Sent to an analytics service | None |
| Crash reports | Sent to Sentry | None |
| Settings and workflows | Synced to Warp's cloud | Stay on disk (`~/.localshell/`) |
| Warp Drive, session sharing, cloud agents, teams, billing | Warp's servers | Removed |
| Codebase indexing | Code processed by Warp's servers | Removed |
| Voice input | Audio transcribed by Warp's servers | Removed |
| Update checks | Warp's release server | None. You update by rebuilding. |
| MCP servers and third-party CLI agents | Connected or launched by Warp | Removed |

If you point LocalShell at a cloud provider such as OpenAI, your prompts go to that provider. That
is the only exception, and it is your choice.

## How the guarantee is enforced

LocalShell doesn't rely on having found and deleted every network call in a large codebase.
Instead, **the app's own network traffic** is blocked by default.

> [!IMPORTANT]
> This covers what LocalShell itself sends: AI requests, telemetry, sync, update checks and the
> like. It does **not** restrict the programs you run *in* the terminal. `ssh`, `curl`, `telnet`,
> `git`, `brew`, `npm`, `pip` and every other command reach the network exactly as they do in
> Terminal.app or iTerm, and so do commands the AI agent runs (under the permission rules you set
> in Settings → Agents). Controlling those is the job of your Mac's firewall, proxy or device
> management, just as it is for any other terminal.

How the app's own traffic is held to one host:

- **One allowlist for the whole app.** At startup, before anything can open a connection, the app
  switches into restricted mode. From then on, the only host it can reach is the AI endpoint in
  Settings. While AI is turned off, the allowlist is empty and the app makes no connections at all.
- **Checked at every layer.** Each request's destination is checked, hostnames are checked before
  they are even looked up, redirects are checked, system proxies are bypassed so the checked host
  is the host actually contacted, and websockets are refused. A code path that was missed fails
  closed, with an error that names the refused host.
- **No device metadata.** Requests to your endpoint don't carry the OS or app-version headers Warp
  normally sends.
- **Visible.** Settings → Agents → AI provider → **Network** shows the allowed host and every
  connection attempt that was blocked since launch.
- **Tested.** An automated test drives the real HTTP client: the allowed endpoint answers, while
  Warp's server and other ports are refused, and the refused port never receives a connection.

You can check all of this yourself with a firewall such as LuLu or Little Snitch: the only
outbound connections the LocalShell app makes are to your AI endpoint. (Connections from commands
you run, such as `ssh` or `git`, are attributed to those programs, not to LocalShell.)

LocalShell doesn't make your organization compliant by itself; that's an assessment your security
team makes. What it gives them is a short, checkable answer to "where does the app send data?"

## What you get

- **The full terminal:** blocks, the modern input editor, completions, themes, workflows, splits,
  tabs, keybindings and the code editor.
- **Agent Mode, powered by your model.** The agent reads and edits files, searches the code and
  runs commands, with the same approval and permission rules as Warp. File diffs, long-running
  commands and todo lists all work.
- **AI Command Search:** type `#` at the start of the input, describe what you want and press
  Enter.
- **Next-command and prompt suggestions** while you type.
- **Generated commit messages, PR titles and PR descriptions** from the code review panel.
- **Per-task model tuning.** Use a large model with a big context for the agent and a small, fast
  one with reasoning off for suggestions. Each task has its own model, context length, reasoning
  level, temperature and output limit.
- **LM Studio integration.** LocalShell lists your models and loads each one with the context
  length its task needs.

## Get started

1. **Build the app** on an Apple Silicon Mac with Xcode. The build produces the installer at
   `target/release-lto/bundle/osx/LocalShell.dmg`.
2. **Install it** and allow it in macOS (the build is self-signed, so macOS may ask once).
3. **Set up AI:** install LM Studio and pick a model, then in LocalShell open Settings → Agents →
   AI provider.

Each step is covered in **[LOCALSHELL.md](LOCALSHELL.md)**, together with verifying the network
behaviour, updating and troubleshooting.

To have an AI coding agent (Claude Code, Codex, Cursor and the like) prepare the Mac and build
for you, point it at **[BUILD_WITH_AI.md](BUILD_WITH_AI.md)**.

## Before you use LocalShell at work

These are facts to check against your own situation and your employer's policies. They are not
legal advice.

- **Licenses.** LocalShell is licensed under the [GNU AGPL v3](LICENSE-AGPL); its UI framework
  (`warpui_core`, `warpui`) is under the [MIT license](LICENSE-MIT). The AGPL's conditions apply
  to copying, modifying and conveying LocalShell itself, not to the files and code you work on
  with it. Some employers restrict AGPL software regardless, so check your company's open-source
  policy. The open-source components inside the app keep their own licenses, listed in
  `THIRD_PARTY_LICENSES.txt` in the app bundle.
- **No warranty.** LocalShell is provided as is, without warranty of any kind, as the AGPL states.
- **Trademarks.** "Warp" and the Warp logo are trademarks of Denver Technologies, Inc. LocalShell
  has its own name and icon. Warp is named in the legal notices and where something refers to
  Warp itself, such as the link to Warp's documentation; a few messages inside the app may still
  mention Warp.
- **Data stored on your Mac.** Nothing below leaves the Mac, but it is stored unencrypted unless
  the disk is (FileVault):
  - Settings, including the AI endpoint's API key in plain text: `~/.localshell/settings.toml`
  - Command history, terminal blocks, AI conversations and app state:
    `~/Library/Application Support/io.github.shahriar-sdet.LocalShell/`
  - Logs: `~/Library/Logs/localshell.log`
- **What the AI endpoint receives.** Your prompts, and whatever the agent reads to answer them:
  command output, file contents and directory listings. With LM Studio on the same Mac, that stays
  on the Mac. With a remote endpoint, it goes to that service under whatever agreement covers it.
- **LM Studio and models.** LM Studio is a separate application with its own terms, and its own
  network activity (such as model downloads and update checks) is outside LocalShell's control.
  Each model has its own license; some restrict commercial use.
- **Signing and updates.** Builds are self-signed, not notarized by Apple, which some IT policies
  don't allow. LocalShell never updates itself: security fixes from Warp or from its dependencies
  arrive only when you pull the source and rebuild.
- **Building needs the internet.** The build downloads the Rust toolchain, crates from crates.io
  and some dependencies from GitHub. The finished app doesn't.
- **Early release.** The network guard is covered by automated tests, but LocalShell hasn't had
  much real-world use yet. Verify the network behaviour on your own build (LOCALSHELL.md, Part 4)
  before relying on it.

## Limitations

- macOS on Apple Silicon only.
- How well the AI works depends on your model. Agent Mode needs a model trained for tool use;
  small models handle suggestions and command search well but struggle with long agent tasks.
- Cloud-only Warp features are gone: Warp Drive, session sharing, cloud agents, MCP servers,
  voice input and codebase indexing.

## How it's built

Warp's agent normally runs on Warp's servers: the app sends the conversation to the server, the
server talks to the model, and the app carries out the tool calls it gets back. LocalShell keeps
the app side unchanged and replaces the server with an in-process engine that speaks the same
protocol to your model. The rest of the UI doesn't know the difference.

| Piece | Where |
|---|---|
| Network allowlist, enforced in HTTP, DNS, redirects and websockets | `crates/egress_guard`, `crates/http_client/src/egress.rs`, `crates/websocket` |
| Client for OpenAI-compatible endpoints and LM Studio, with per-task settings | `crates/local_ai` |
| Warp's agent protocol, served in-process from your model | `crates/local_agent` |
| Endpoint runtime, allowlist updates, model list | `app/src/ai/local_ai_runtime.rs` |
| Command search, suggestions and commit messages on your model | `app/src/ai/local_generation.rs` |
| AI provider settings page | `app/src/settings_view/local_ai_page.rs` |
| Features removed in LocalShell | `LOCALSHELL_DISABLED_FLAGS` in `crates/warp_features/src/lib.rs` |
| App identity, disabled server addresses, restricted mode at startup | `app/src/bin/oss.rs` |

For development setup, coding style and testing, see [AGENTS.md](AGENTS.md).

## License and credits

**Modification notice.** LocalShell is a modified version of
[warpdotdev/warp](https://github.com/warpdotdev/warp), based on upstream commit `21f06832`
(12 October 2026). The modifications, made from 13 October 2026 onward, remove Warp's cloud
services, telemetry, accounts and third-party integrations, restrict the app's network access to
a user-configured AI endpoint, add an in-process AI engine for that endpoint, and give the app its
own name and icon. The full list of changes is in this repository's Git history.

Copyright © 2020–2026 Denver Technologies, Inc. Modifications copyright © 2026 shahriar-sdet.

LocalShell keeps the upstream licenses: the UI framework (`warpui_core`, `warpui`) is under the
[MIT license](LICENSE-MIT), and everything else is under the [AGPL v3](LICENSE-AGPL).

Thanks to the Warp team for open-sourcing the client, and to the projects it builds on, including
[Tokio](https://github.com/tokio-rs/tokio), [Alacritty](https://github.com/alacritty/alacritty),
[Hyper](https://github.com/hyperium/hyper), [NuShell](https://github.com/nushell/nushell),
[Fig completion specs](https://github.com/withfig/autocomplete) and
[FontKit](https://github.com/servo/font-kit).
