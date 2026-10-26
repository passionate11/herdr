# Personal Herdr Build

Based on upstream `v0.9.5`, maintained on `custom/grouped-v0.9.5` in
https://github.com/passionate11/herdr.

The custom client adds titled space boxes, untitled boxes for multiple agents
sharing a tab, and subtle cards with half-row gaps. Grouping uses endpoint,
workspace and tab identities. Priority sorting and custom agent views retain
their upstream order. Rendering, mouse targets and scrolling share one layout.
The server protocol is unchanged.

```toml
[ui.sidebar.agents]
group_boxes = true
cards = true
row_gap = 1
rows = [["state_icon", "workspace", "tab", "agent"]]
```

Cards mix the host terminal background toward its foreground by 3.5%, or 9%
for the focused agent. A single agent in a split tab gets no subgroup. Small
viewports fall back to the simpler layout. Scrolled groups close both borders.

## Build And Validate

Install the Rust version pinned in `rust-toolchain.toml` (currently 1.96.1)
through rustup, including `clippy` and `rustfmt`. Build checks also require
`just`, `cargo-nextest`, Bun, Python 3.11 or later, Git with
`diff --default-prefix` support, GNU `sha256sum`, and Zig 0.16.0. On macOS,
Homebrew's `rustup`, `python@3.13`, `git`, and `coreutils` provide the corresponding
tools. The script discovers existing installations under `/opt/homebrew` and
`/usr/local` without changing shell configuration. It uses the already installed
pinned Rust toolchain and reports missing tools; it does not install toolchains.
GNU coreutils are exposed only to maintenance tests, preserving the native macOS
tools used by Rust tests and the release build.

Create the updater environment with the installation commands below. It needs
Python 3.9 or later, independently of the build checks' Python requirement.
Pillow enables optional screenshot artifacts. Set `ZIG` if Zig 0.16.0 is not on
PATH. The scripts also find a separately installed Zig under
`~/.local/share/herdr-custom/toolchains/zig-aarch64-macos-0.16.0/`.

```sh
bash custom/build.sh
~/.local/share/herdr-custom/venv/bin/python custom/rehearse.py /path/to/current/herdr /path/to/candidate/herdr
```

The build runs formatting and Clippy, debug-mode nextest as upstream CI does,
maintenance and architecture checks, integration asset tests, documentation
contract tests, and custom updater tests before packaging the release binary.
On macOS, nextest excludes `live_handoff`
as upstream CI does; the separate rehearsal verifies the actual cross-version
handoff. This macOS customization does not qualify Windows builds or run
`just windows-lint`; it does not download the Windows SDK.

The rehearsal uses synthetic configuration, temporary sockets and mock agents.
It never reads private conversations. It verifies pane/tab identities, process
survival, input/output, grouped rendering and mouse targeting across handoff.
Read its `--help` for artifact and keep options. Rehearsal artifacts and live
session receipts belong outside Git.

## Install And Update

Each candidate lives in its own version and commit directory under
`~/.local/share/herdr-custom/builds/`. `current` selects the active directory.
Install `custom/herdr-launcher` as `~/.local/bin/herdr`, and link
`~/.local/bin/herdr-custom-update` to this checkout's updater wrapper. Run these
from the checkout after a candidate has been built and rehearsed:

```sh
python3 -m venv ~/.local/share/herdr-custom/venv
~/.local/share/herdr-custom/venv/bin/pip install -r custom/requirements.txt
mkdir -p ~/.local/bin
install -m 755 custom/herdr-launcher ~/.local/bin/herdr
chmod +x custom/herdr-custom-update
ln -s "$PWD/custom/herdr-custom-update" ~/.local/bin/herdr-custom-update
```

The wrapper always uses that environment's Python, including for rehearsal.
`HERDR_CUSTOM_INSTALL_ROOT` overrides the build, update and runtime installation
root consistently; create `venv` under that root when using the override.

```sh
herdr-custom-update
herdr-custom-update --activate
```

The first command fetches the personal branch, builds, tests and rehearses.
`--activate` also backs up the old executable and hands the running server to the
candidate. It switches `current` to the matching client immediately after the
handoff, then verifies pane/tab identities and existing process IDs. Clients
detach once; reconnect with `herdr`. The update command targets the default
local session after clearing inherited `HERDR_*` socket and session variables.
The first 0.7.4 upgrade copies sidebar width, section split, and collapsed groups
into the new client preference file. Existing client preferences take priority.
Executable, configuration, session, and preference backups stay local.

The upstream `herdr update` installer is disabled in this custom build. Its
availability check remains available. Homebrew cannot overwrite the separate
custom build directory. Keep the old executable for recovery, and rehearse any
downgrade too; saved sessions may evolve across upstream versions.

## Track New Upstream Versions

Keep `origin` as the upstream remote and `personal` as this fork. Start a new
custom branch at the selected stable tag and carry the custom commits forward.
Resolve API/layout changes, update the base version in these scripts, run checks,
and rehearse the exact currently installed executable to the candidate. Publish
the custom branch only after reviewing the diff for private data. Do not run
upstream release workflows or push to the canonical repository.

`legacy/` records the prior 0.7.4 implementation as a patch for recovery. The
current client implementation is maintained as normal source commits.

## Candidate Validation

On macOS arm64, the native debug suite passed 3,832 tests, alongside Clippy,
formatting, maintenance, integration, documentation, and custom migration checks.
Isolated 0.7.4-to-0.9.5 and 0.9.5-to-0.9.5 activations preserved eight mock agent
PIDs, pane identities, terminal contents, input, sidebar preferences, and grouped
mouse/scroll behavior.

Fixed-geometry grouped sidebar rendering took approximately 3.1 / 21.5 / 26.5
microseconds for 1 / 15 / 150 rows. The main render scaling profile completed;
the aggregate `just bench-render-scale` command also selects a native-image CoW
benchmark that requires Linux source-file cloning and fails on macOS's supported
decoded fallback. This optional image benchmark is outside the sidebar change.
