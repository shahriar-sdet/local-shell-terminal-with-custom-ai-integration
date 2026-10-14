# Build LocalShell with an AI agent

**For the human:** open your AI coding agent (Claude Code, Codex, Cursor or similar) in a terminal
on the Mac that should run LocalShell, and tell it:

> Read `BUILD_WITH_AI.md` in https://github.com/shahriar-sdet/warp-with-custom-local-ai (or in this
> folder, if you've already cloned it) and follow it.

The agent prepares the Mac, builds the installer and tells you what to do next. It stops and asks
you whenever a step needs your password, the App Store or a click in a macOS dialog.

**Everything below is for the agent.**

---

## Your task

Build LocalShell's installer, `target/release-lto/bundle/osx/LocalShell.dmg`, on this Mac and
verify it. Then hand over to the human for installation. [LOCALSHELL.md](LOCALSHELL.md) is the
human-facing guide to the same process; this file adds the checks, failure fixes and hand-off you
need to run it yourself.

You are done when:

1. `LocalShell.dmg` and `LocalShell.app` exist in `target/release-lto/bundle/osx/`;
2. every check in [Step 7](#step-7-verify-the-build) passes;
3. you've given the human the hand-off message from [Step 8](#step-8-hand-off).

## Rules

- **Ask, don't guess.** Stop and ask the human before anything that needs `sudo`, the App Store,
  a macOS permission dialog, installing software beyond the tools listed here, or deleting files.
  `sudo` here asks for a password you can't type: give the human the exact command to run.
- **Don't change the app's source code** to get a build through. Every failure seen so far is
  environmental and listed in [Troubleshooting](#troubleshooting). If a build still fails, report
  the error and its cause to the human instead of editing code.
- **Never weaken the privacy design.** Don't touch `crates/egress_guard`,
  `crates/http_client/src/egress.rs`, `LOCALSHELL_DISABLED_FLAGS` in
  `crates/warp_features/src/lib.rs` or `app/src/bin/oss.rs`, and don't add hosts, telemetry, crash
  reporting or update checks. LocalShell exists so the app talks only to the AI endpoint the user
  chooses.
- **Don't commit, push, publish or upload anything.** The DMG stays on this Mac.
- **Keep the legal files** (`LICENSE-AGPL`, `LICENSE-MIT`, the README notice) as they are.
- **Run long commands in the background with a log file** and check the exit code. Your shell
  state may not persist between commands, so start each one that uses Rust with
  `source "$HOME/.cargo/env"`.
- Run cargo commands one at a time: they share a build directory lock.

## Step 0. Check the Mac

```bash
uname -m                      # must print arm64
sw_vers -productVersion       # macOS version
df -h / | tail -1             # need at least 40 GB free
xcode-select -p               # should be /Applications/Xcode.app/Contents/Developer
xcodebuild -version           # Xcode version
brew --version                # Homebrew
source "$HOME/.cargo/env" 2>/dev/null; rustup --version
git lfs version
```

- **Not `arm64`:** stop. LocalShell builds only on Apple Silicon. Tell the human.
- **Less than 40 GB free:** stop and ask the human to free space.
- Note what's missing; Steps 1–5 install it.

## Step 1. Xcode

The build needs full Xcode, not just the Command Line Tools, because it compiles Metal shaders.

1. **Xcode missing** (`xcodebuild -version` fails or `/Applications/Xcode.app` doesn't exist): ask
   the human to install Xcode from the App Store, open it once, then quit it. Wait for them.
2. **Wrong developer directory** (`xcode-select -p` isn't Xcode's): ask the human to run
   `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`.
3. **License:** `xcodebuild -license check` must exit 0. Otherwise ask the human to run
   `sudo xcodebuild -license accept`.
4. **First launch:** if `xcodebuild -checkFirstLaunchStatus` exits non-zero, run
   `xcodebuild -runFirstLaunch` (no `sudo` needed).
5. **Metal compiler:** `xcrun -sdk macosx metal --version` must print a version. If it says the
   Metal toolchain is missing, run `xcodebuild -downloadComponent MetalToolchain` (about 700 MB)
   and check again.

**Pass:** the Metal command prints an `Apple metal version` line.

Xcode may be older than macOS (for example Xcode 26 on macOS 27). That's fine:
`script/macos/bundle` pins the SDK to Xcode's own. See the first
[Troubleshooting](#troubleshooting) row if you run `cargo` directly.

## Step 2. Homebrew packages and Git LFS

1. If `brew` is missing, ask the human to install Homebrew from https://brew.sh and to run the
   "Next steps" it prints.
2. Install and enable the tools:

   ```bash
   brew install protobuf create-dmg git-lfs
   git lfs install
   ```

**Pass:** `protoc --version` and `git lfs version` both print a version.

Git LFS must be on **before** the source is cloned. Without it the input-classifier models arrive as
133-byte placeholder files, the build embeds them without complaint and the app fails to load them
at runtime.

## Step 3. Rust

If `rustup` is missing:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

**Pass:** `rustup --version` prints a version.

## Step 4. Source code

If the current directory is already a LocalShell checkout (it has `LOCALSHELL.md` and
`app/src/bin/oss.rs`), use it. Otherwise:

```bash
git clone https://github.com/shahriar-sdet/warp-with-custom-local-ai.git localshell
cd localshell
```

Run everything from here on in the checkout's root.

Check the models:

```bash
ls -l crates/input_classifier/models/onnx/*.onnx
```

**Pass:** three `bert_tiny_v*.onnx` files of about 17 MB each. If they're a few hundred bytes, run
`git lfs pull` and check again.

## Step 5. Pinned Rust version and packaging tools

```bash
source "$HOME/.cargo/env"
rustup toolchain install
./script/install_cargo_bundle
./script/install_cargo_release_deps --no-build-deps
```

**Pass:** all three of these print the expected version:

```bash
cargo --version            # cargo 1.92.0
cargo-bundle --version     # cargo-bundle v0.11.0
cargo-about --version      # cargo-about 0.8.4
```

## Step 6. Build

Run in the background, logging to a file:

```bash
source "$HOME/.cargo/env"
./script/bundle --channel oss --arch aarch64 --selfsign > /tmp/localshell-build.log 2>&1
echo "BUNDLE EXIT=$?"
```

- It takes about 15 minutes to over an hour, depending on the Mac. Check progress with
  `tail /tmp/localshell-build.log`.
- `--selfsign` uses an "Apple Development" certificate if the keychain has one, otherwise an ad-hoc
  signature. Both are fine.
- Near the end, `create-dmg` may make macOS ask whether your terminal app may control Finder. Tell
  the human to click **Allow**.
- During the build the script runs the new binary once to generate a settings schema. That's
  expected.

**Pass:** `BUNDLE EXIT=0`. Otherwise look the error up in [Troubleshooting](#troubleshooting),
fix the cause and run the build again.

## Step 7. Verify the build

```bash
B=target/release-lto/bundle/osx
ls "$B"
codesign --verify --deep --strict "$B/LocalShell.app" && echo "signature OK"
codesign -dvv "$B/LocalShell.app" 2>&1 | grep -E "Identifier=|Signature|Authority"
/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$B/LocalShell.app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Print :CFBundleName" "$B/LocalShell.app/Contents/Info.plist"
ls "$B/LocalShell.app/Contents/Resources"
ls "$B/LocalShell.app/Contents/PlugIns" 2>&1
```

**Pass when all of these hold:**

| Check | Expected |
|---|---|
| `ls "$B"` | `LocalShell.dmg`, `LocalShell.app`, `localshell.dSYM` |
| Signature | `signature OK` |
| Identifier | `io.github.shahriar-sdet.LocalShell` (from both `codesign` and `PlistBuddy`) |
| Name | `LocalShell` |
| Resources | contains `LICENSE-AGPL`, `LICENSE-MIT` and `THIRD_PARTY_LICENSES.txt` |
| PlugIns | `No such file or directory` (LocalShell ships no dock-tile plugin) |

Note the first `Authority=` line. If the app was signed with a personal certificate, it shows the
certificate's name, often an email address. You'll mention it in the hand-off.

### Optional: tests

Only run these if the human asks. They take a while, and some upstream tests are known to fail.

```bash
source "$HOME/.cargo/env"
./script/install_cargo_test_deps   # installs cargo-nextest and the other test tools
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
cargo nextest run -p warp_core -p egress_guard -p local_ai -p local_agent -p http_client
```

These five crates should all pass; they include the egress-guard integration test. The `warp` and
`warp_tui` crates have pre-existing failures for features LocalShell removes, so compare against a
run on the unmodified checkout rather than expecting zero.

## Step 8. Hand-off

Tell the human, in your own words:

1. **Where the installer is:** the full path to `target/release-lto/bundle/osx/LocalShell.dmg`.
   Offer to show it in Finder (`open -R <path>`).
2. **How to install:** quit any running LocalShell, open the DMG, drag **LocalShell** onto
   **Applications** (choose **Replace** if asked) and eject the DMG. If macOS blocks the first
   launch, follow [LOCALSHELL.md](LOCALSHELL.md) Part 2 Step 2.
3. **What to expect:** it opens straight to a terminal, with no sign-in. AI stays off until they set
   it up.
4. **Next:** set up AI with [LOCALSHELL.md](LOCALSHELL.md) Part 3 (LM Studio, or an
   OpenAI-compatible endpoint), then, if their workplace needs evidence, Part 4 to verify the
   network behaviour.
5. **Before sharing the DMG:** if the `Authority=` line from Step 7 shows a personal name or email,
   anyone who receives the DMG can see it.
6. **Updating later:** `git pull` in the checkout, then repeat from Step 5 and reinstall.

Also report anything you had to fix along the way and any step the human had to do by hand.

## After installation (only if the human asks)

- **Check the app launched:** `pgrep -x localshell` lists it, and
  `grep -c panicked ~/Library/Logs/localshell.log` prints `0`.
- **Check egress:** sample LocalShell's sockets several times a second (`nettop` only shows
  connections open at that moment, so short ones are easy to miss):

  ```bash
  while true; do
    for p in $(pgrep -x localshell); do lsof -nP -a -p "$p" -i 2>/dev/null | tail -n +2; done
    sleep 0.2
  done
  ```

  With AI off there should be no outbound connections. With AI on, the only remote address should
  be the configured endpoint. Refused attempts are logged as `egress_guard: blocked` in
  `~/Library/Logs/localshell.log` and listed under **Settings → Agents → AI provider → Network**.
  A connection to any other host is a serious bug: stop and report it.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `ld: tapi error: malformed file` / `unknown architecture arm64e…`, often while building `aws-lc-sys` | Xcode is older than macOS, so `cc` picks the Command Line Tools SDK, which Xcode's linker can't read | `script/macos/bundle` handles this. For direct `cargo` commands, first run `export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"`. |
| `cannot execute tool 'metal' due to missing Metal Toolchain` | Xcode 26+ ships Metal separately | `xcodebuild -downloadComponent MetalToolchain` |
| Any other error mentioning `metal` or `metallib` | Xcode isn't selected, or its first launch didn't finish | Step 1 items 2–5 |
| `Could not find protoc` | protobuf missing | `brew install protobuf`, then a new shell |
| `no such command: bundle` / `no such command: about` | Packaging tools missing | Step 5 |
| `sudo: a password is required` | You can't type the password | Give the human the exact command to run |
| `cargo binstall … 403 Forbidden`, then `Nextest does not support being installed without --locked` | GitHub API rate limit; the source fallback needs `--locked` | Use `./script/install_cargo_test_deps` (it passes `--locked`), or `cargo install --locked cargo-nextest` |
| `missing get_cursor_shape` in `warpui` when testing a few crates | Cargo unifies a different feature set for that subset | Test with the same `-p` list every time, or add `-p warp` |
| The app logs `Failed to initialize candle inference runner` | Models are LFS placeholders | Step 2, `git lfs pull`, rebuild |
| The app quits right after opening | A crash on launch | Read the `panicked` line in `~/Library/Logs/localshell.log` and report it. To open the app with AI off meanwhile, rename `~/.localshell/settings.toml`. |
| macOS says the app "is damaged" or "can't be opened" | Quarantine flag on a self-signed app | `xattr -dr com.apple.quarantine /Applications/LocalShell.app`, or LOCALSHELL.md Part 2 Step 2 |
