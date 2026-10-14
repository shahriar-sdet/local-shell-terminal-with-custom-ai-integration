# LocalShell: build, install and set up

This guide has four parts. Do them in order; each part only depends on the ones before it.

1. **Build the app** on an Apple Silicon Mac.
2. **Install it.**
3. **Set up AI.**
4. **Verify the network behaviour.** Optional, for you or your security team.

Then come updating, uninstalling and troubleshooting. What LocalShell is and why it exists is in
the [README](README.md).

---

## Part 1 — Build the app

**You need:**

- A Mac with Apple Silicon (M1 or newer). Intel Macs can't build or run this version.
- At least 40 GB of free disk space and an administrator account.
- About two hours, most of it waiting on the first build.

### Step 1. Install Xcode

1. Install **Xcode** from the App Store. The Command Line Tools alone aren't enough: the build
   needs Xcode's Metal shader compiler.
2. Open Xcode once, accept the license and let it install its components, then quit Xcode.
3. In Terminal, make Xcode the active developer directory and finish its setup:

   ```bash
   sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
   sudo xcodebuild -license accept
   xcodebuild -runFirstLaunch
   ```

4. Check that the Metal compiler is available:

   ```bash
   xcrun -sdk macosx metal --version
   ```

   If this reports that the Metal toolchain is missing (Xcode 26 and later ship it as a separate
   download), install it and run the check again:

   ```bash
   xcodebuild -downloadComponent MetalToolchain
   ```

### Step 2. Install Homebrew and the build tools

1. If `brew --version` doesn't work, install Homebrew from [brew.sh](https://brew.sh) and follow
   the "Next steps" it prints to add `brew` to your shell.
2. Install the protocol-buffer compiler and the DMG tool:

   ```bash
   brew install protobuf create-dmg
   ```

3. Check: `protoc --version` prints a version number.

### Step 3. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

Check: `rustup --version` prints a version number.

### Step 4. Get the source code

```bash
git clone https://github.com/shahriar-sdet/warp-with-custom-local-ai.git localshell
cd localshell
```

Every command from here on runs inside this `localshell` directory.

### Step 5. Install the pinned Rust version and the packaging tools

```bash
rustup toolchain install
./script/install_cargo_bundle
./script/install_cargo_release_deps --no-build-deps
```

- `rustup toolchain install` reads `rust-toolchain.toml` and installs the exact Rust version the
  project uses.
- `install_cargo_bundle` installs the tool that packages the `.app`.
- `install_cargo_release_deps --no-build-deps` installs `cargo-about`, which writes
  `THIRD_PARTY_LICENSES.txt` into the app: the license notices of the open-source components
  inside it.

Check: `cargo --version` prints `cargo 1.92.0`.

### Step 6. Build

```bash
./script/bundle --channel oss --arch aarch64 --selfsign
```

- `--channel oss` builds LocalShell.
- `--arch aarch64` builds for Apple Silicon only.
- `--selfsign` signs the app with an "Apple Development" certificate if your keychain has one,
  and with an ad-hoc signature otherwise. No Apple Developer account is needed.

The first build can take an hour or more. Near the end, macOS may ask to let Terminal control
Finder; allow it, because the DMG tool uses it to arrange the DMG window.

### Step 7. Check the result

```bash
ls target/release-lto/bundle/osx/
codesign -dv target/release-lto/bundle/osx/LocalShell.app 2>&1 | grep -E "Identifier|Signature|Authority"
```

You should see `LocalShell.dmg` and `LocalShell.app`, with
`Identifier=io.github.shahriar-sdet.LocalShell`. For an ad-hoc build the signature shows as
`Signature=adhoc`; with a certificate, `Authority` names it.

---

## Part 2 — Install the app

### Step 1. Copy the app into Applications

Open `target/release-lto/bundle/osx/LocalShell.dmg` and drag **LocalShell** onto the
**Applications** folder. Then eject the DMG.

### Step 2. Allow the first launch

The app is self-signed rather than notarized by Apple, so macOS may block the first launch. If it
does, use **one** of these:

- **Terminal (works on every macOS version):**

  ```bash
  xattr -dr com.apple.quarantine /Applications/LocalShell.app
  ```

- **System Settings (macOS 15 Sequoia and later):** open LocalShell from Applications. When
  macOS says it can't be verified, click **Done**. Go to **System Settings → Privacy & Security**,
  scroll to **Security**, click **Open Anyway** next to the message about LocalShell, and confirm.
- **Finder (macOS 14 and earlier):** Control-click LocalShell in Applications, choose **Open**,
  then **Open** again.

### Step 3. Open LocalShell

Open LocalShell from Applications. It starts straight into a terminal: there is no account to
create and nothing to sign in to. AI features stay off until Part 3.

LocalShell keeps its settings in `~/.localshell/` and doesn't touch an existing Warp installation.

---

## Part 3 — Set up AI

Choose **A** to keep everything on your Mac, or **B** to use an OpenAI-compatible service.

### A. LM Studio (everything stays on your Mac)

**Step 1. Install LM Studio** from [lmstudio.ai](https://lmstudio.ai) and open it.

**Step 2. Download a model trained for tool use.** Agent Mode needs a model that can call tools.
As a starting point: on a 16 GB Mac, try a 4–8B model; with 32 GB or more, a 20–30B coding model
(for example gpt-oss-20b or Qwen3-Coder-30B) does much better on agent tasks. You don't need to
load it; LocalShell loads models when they're needed.

**Step 3. Start LM Studio's server.** In LM Studio's **Developer** tab, start the server (or run
`lms server start` in a terminal). It listens on port 1234 by default.

**Step 4. Open the AI settings in LocalShell.** Press **⌘,** to open Settings, then choose
**Agents → AI provider**. (Or press **⌘P** and run **Open Settings: AI provider**.)

**Step 5. Connect.**

1. Turn on **AI features**.
2. Set **Provider** to **LM Studio**.
3. Leave **Endpoint URL** as `http://localhost:1234/v1` unless you changed LM Studio's port.
   After editing any text field, press **Enter** to save it.
4. Click **Refresh models**. The status line reads "Connected. N chat models available."

**Step 6. Choose the agent's model.** Under **Agent**, open **Model** and pick the model from
Step 2. AI features are now on.

### B. OpenAI or another OpenAI-compatible service

1. Do Step 4 above to open **Agents → AI provider**.
2. Turn on **AI features** and set **Provider** to **OpenAI-compatible**.
3. Enter the **Endpoint URL**, for example `https://api.openai.com/v1` or your gateway's URL, and
   press **Enter**.
4. Enter the **API key** and press **Enter**.
5. Click **Refresh models**, then pick the **Agent** model.

### Try it

- Type `#` at the start of the terminal input, then describe a command ("find files larger than
  100 MB").
- Press **⌘P** and run **New Agent Tab**, then ask the agent to do something in a project folder.

### Tune each task (optional)

The same page has a section per task. Each has its own **Model**, **Reasoning**, **Context
length**, **Temperature** and **Max output tokens**; leave a field empty to use the default.

| Task | Used for | Starts with |
|---|---|---|
| Agent | Agent Mode conversations | 32K context, medium reasoning |
| Command generation | `#` command search and AI answers | 4K context, reasoning off |
| Suggestions | Next-command and prompt suggestions while typing | 4K context, 128 output tokens, reasoning off |
| Titles & summaries | Commit messages, PR titles and descriptions | 4K context, reasoning off |

Tasks without their own model use the Agent's model. Fast tasks feel best with a small model and
reasoning off. With LM Studio and **Load models for each task** turned on, LocalShell loads each
task's model with that task's context length, and reuses an already-loaded copy that has at least
that much.

All of this is stored in `~/.localshell/settings.toml` and can be edited there:

```toml
[agents.local_ai]
provider = "lm_studio"            # or "open_ai_compatible"
base_url = "http://localhost:1234/v1"
api_key = ""
manage_model_loading = true

[agents.local_ai.agent]
model = "qwen/qwen3-coder-30b"
context_length = 32768
reasoning = "medium"              # model_default, off, low, medium or high

[agents.local_ai.suggestions]
model = "qwen/qwen3-4b"
context_length = 4096
max_output_tokens = 128
reasoning = "off"
```

---

## Part 4 — Verify the network behaviour

1. In LocalShell, open **Settings → Agents → AI provider** and look at **Network**. It lists the
   one host that can be reached and every connection attempt that was blocked since launch.
2. Turn **AI features** off. The Network row now says no outbound connections are allowed.
3. To check from outside the app, use a firewall such as LuLu or Little Snitch, or watch live
   connections in Terminal:

   ```bash
   nettop -p "$(pgrep -x localshell)"
   ```

   The only remote address should be your AI endpoint. Connections made by commands you run in
   the terminal belong to those commands (for example `git`), not to LocalShell.

---

## Updating

LocalShell doesn't update itself. To update:

1. In the `localshell` directory, run `git pull`.
2. Repeat Part 1 from Step 5.
3. Quit LocalShell, replace LocalShell in Applications with the new build (Part 2 Step 1), and
   repeat Part 2 Step 2 if macOS asks.

Settings and history are kept.

## Uninstalling

Quit LocalShell, then delete:

- `/Applications/LocalShell.app`
- `~/.localshell` (settings, including the AI endpoint and API key)
- `~/Library/Application Support/io.github.shahriar-sdet.LocalShell` (history and app state)
- `~/Library/Logs/localshell.log*` (logs)

## Troubleshooting

| Symptom | Fix |
|---|---|
| Build: `Could not find protoc` | Part 1 Step 2: `brew install protobuf`, then open a new terminal. |
| Build: an error mentioning `metal` or `metallib` | Part 1 Step 1: make sure Xcode is selected and the Metal toolchain check passes. |
| Build: `no such command: bundle` | Part 1 Step 5: run `./script/install_cargo_bundle`. |
| Build: `no such command: about` | Part 1 Step 5: run `./script/install_cargo_release_deps --no-build-deps`. |
| macOS says the app "is damaged" or "can't be opened" | Part 2 Step 2: remove the quarantine flag with the `xattr` command. |
| "Could not reach the AI endpoint" | Make sure LM Studio's server is running and the endpoint URL ends in `/v1`. |
| "No models found" | Download a model in LM Studio, then click **Refresh models**. |
| The agent describes commands but never runs them | The model isn't trained for tool use; choose another Agent model. |
| "Input exceeded context window limit" | Raise **Context length** for that task, or start a new conversation. |
| Suggestions are slow | Give **Suggestions** a smaller model and set its **Reasoning** to **Off**. |
| AI menu items are missing | AI turns on only after an endpoint and an Agent model are set (Part 3). |

Logs are in `~/Library/Logs/localshell.log`. They stay on your Mac.
