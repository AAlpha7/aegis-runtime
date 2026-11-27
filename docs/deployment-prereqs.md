# Deployment Prerequisites

Status: active operational setup guide.

Fresh-machine setup guide for Aegis Runtime + the multi-CC coordination layer. This
complements `docs/DEPLOYMENT.md` (which focuses on production runtime). Read
this first on a new host, then follow `docs/DEPLOYMENT.md` for service wiring.

Only §1 is needed to build and run the product. §2–§5.5 describe the author's
multi-agent (multi-CC) development loop; its hooks live under `.claude/`, which
is untracked, so a fresh clone cannot run §3–§4 as written.

## 1. Host dependencies

### Runtime + developer tools

| Package    | Why                                                          |
|------------|--------------------------------------------------------------|
| `git`      | >= 2.30 — worktree support is mandatory for the multi-CC flow |
| `bash`     | >= 4 — hook scripts use `[[ ]]`, associative arrays, `mapfile` |
| `tmux`     | Required by `/migrate` and the mobile workflow                |
| `jq`       | Used by hooks for JSON parsing                                |
| `flock`    | `util-linux` — serializes commit + deploy critical sections    |
| `shellcheck` | CI + local test gates (`test-migrate.sh` etc.)               |
| Rust       | `rustc`/`cargo` 1.98+ (`rust-toolchain.toml` pins 1.98.1; rustup installs it on first use) with the `wasm32-wasip1` target |
| Node.js    | 22 + npm — builds the web console (`cd webapp && npm ci && npm run build`); not needed at runtime |
| `ffmpeg`   | Optional — voice notes are transcoded to 16 kHz mono WAV before transcription |
| `python3`  | `make check` (`agent-harness-lint`, `agent-harness-test`)     |
| `gh`       | Optional — for push credentials under claude-safe              |

### Install

```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install -y git bash tmux jq util-linux shellcheck ffmpeg
scripts/bootstrap-system-deps.sh   # build-essential, pkg-config, libssl-dev (+ mold/clang/sccache accelerators)
# Install Node.js 22+ with npm from nodejs.org or a version manager.
# Ubuntu's nodejs/npm packages may be older; verify with `node --version`.

# macOS (Homebrew)
brew install git bash tmux jq util-linux shellcheck node ffmpeg

# Rust toolchain (rustup reads rust-toolchain.toml and installs the pinned 1.98.1)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup target add wasm32-wasip1

# GitHub CLI (optional)
# Linux:  type -p curl >/dev/null || sudo apt install curl -y; curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | sudo dd of=/etc/apt/keyrings/githubcli-archive-keyring.gpg; echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null; sudo apt update && sudo apt install gh -y
# macOS:  brew install gh
```

## 2. Git push credentials

Claude Code sub-agents run in `claude-safe` sandbox mode and cannot by default
reach the internet for `git push`. Two options:

1. **SSH keys on the host**: configure `~/.ssh/config` and add the public key
   to GitHub. The sandbox inherits SSH agent socket if the user exports
   `SSH_AUTH_SOCK` into the sandbox env.
2. **Per-repo Deploy Key** via the `/grant` slash command (preferred for
   single-repo sessions):
   ```
   /grant
   ```
   Prints the host command to add a scoped Deploy Key. Re-run as needed.

GitHub HTTPS pushes via PAT also work if `GH_TOKEN` is in the env — but prefer
the Deploy Key path so credentials are scoped to one repo.

## 3. Hook configuration verification

After cloning, verify the coord hook layer is wired correctly:

```bash
bash scripts/test-p1a.sh              # broadcasts, inbox, active-work helpers
bash scripts/test-p1c.sh              # main-edit-consent gate
bash scripts/test-p1d.sh              # pre-commit lock + review gate
bash scripts/test-p1e-integration.sh  # end-to-end peer coordination
bash scripts/test-migrate.sh          # /migrate mobile flow
```

All five should report `OK` / `N/N passed`. If any fail, stop and investigate
— the coord layer is on the commit path; a broken gate blocks all work.

## 4. Memory bootstrap (one-time per clone)

```bash
bash .claude/bootstrap-memory.sh
```

Creates a symlink from `~/.claude/projects/<slug>/memory` to the repo's
`.claude/memory/` so CC sessions read and write the canonical memory store.

## 5. tmux quickstart

If you're new to tmux, the mobile workflow relies on these bindings
(default prefix is `Ctrl-b`):

| Keys            | Action                                               |
|-----------------|------------------------------------------------------|
| `Ctrl-b c`      | Create a new window in the current session          |
| `Ctrl-b n`      | Next window                                          |
| `Ctrl-b p`      | Previous window                                      |
| `Ctrl-b w`      | Window picker (interactive list)                     |
| `Ctrl-b &`      | Kill current window (confirmation prompt)            |
| `Ctrl-b d`      | Detach session (CC keeps running in background)      |
| `tmux attach`   | Re-attach to the most recent session                 |
| `tmux ls`       | List sessions                                        |

For `/migrate`: after the command exits, hit `Ctrl-b n` to jump to the new
window, verify the resumed session is healthy, then `/exit` the old one.

## 5.5 Wake daemon (multi-CC idle wake-up, Plan §8)

The wake daemon watches `.claude/coordination/wake/*.wakereq` and pokes
idle peer CCs via `tmux send-keys` when another CC messages them. Without
it, direct messages to idle CCs sit unread until the user happens to
visit the pane. Falls back to bark push if the target is not in tmux.

Prerequisites:
- `inotify-tools` installed (for `inotifywait`). On Ubuntu:
  `sudo apt-get install -y inotify-tools`
- `tmux` running with CC sessions in named panes (standard `/migrate` flow).
- Optional: `BARK_URL` set in `/opt/aegis-runtime/shared/.env` or your shell
  env to enable push fallback for non-tmux targets.

Install as a user-level systemd unit (runs without root, scoped to your
user session):

```bash
mkdir -p ~/.config/systemd/user
cp scripts/wake-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wake-daemon
# Verify:
systemctl --user status wake-daemon
tail -f .claude/coordination/wake-daemon.log
```

If `systemctl --user` is unavailable (headless box without linger), keep
the daemon alive in its own tmux window instead:

```bash
tmux new-window -n wake-daemon 'exec scripts/wake-daemon.sh'
# Enable linger so user services survive logout:
sudo loginctl enable-linger "$USER"
```

Tunables (set in the service unit or shell env):
- `WAKE_IDLE_SECS` (default `60`) — pane idle threshold before send-keys.
- `WAKE_COOLDOWN_SECS` (default `30`) — min seconds between wakes per target.
- `AEGIS_RUNTIME_REPO_ROOT` — override canonical repo path.

## 6. Production runtime setup

For systemd units, firewalls, Cloudflare tunnel, and database placement, see
`docs/DEPLOYMENT.md`. This file covers only host prerequisites and the
multi-CC dev-loop setup.
