#!/usr/bin/env python3
"""Rehearse an OLD -> NEW Herdr handoff using only disposable mock sessions.

Requires a C compiler and pyte; --png also requires Pillow. No user configuration,
session transcript, credentials, or default Herdr socket is read or copied.

    python3 custom/rehearse.py /path/to/old/herdr /path/to/new/herdr --keep
    python3 custom/rehearse.py OLD NEW --output /tmp/herdr-check --png
"""

import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time

from preferences import client_preferences_path
from update import activate


CONFIG = """onboarding = false
[theme]
name = "vesper"
[terminal]
default_shell = "/bin/sh"
shell_mode = "non_login"
[update]
version_check = false
manifest_check = false
[ui]
sidebar_width = 32
agent_panel_sort = "spaces"
mobile_width_threshold = 40
[ui.sidebar.agents]
rows = [["state_icon", "pane", "agent"]]
row_gap = 1
group_boxes = true
cards = true
[session]
resume_agents_on_restore = false
"""

MOCK_SOURCE = r"""#include <stdio.h>
#include <unistd.h>
#include <limits.h>
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    char path[PATH_MAX];
    snprintf(path, sizeof(path), "%s/%s.pid", argv[2], argv[1]);
    FILE *file = fopen(path, "w");
    if (!file) return 3;
    fprintf(file, "%ld\n", (long)getpid());
    fclose(file);
    setvbuf(stdout, NULL, _IONBF, 0);
    printf("MOCK:%s:PID:%ld\n", argv[1], (long)getpid());
    char line[1024];
    while (fgets(line, sizeof(line), stdin)) {
        printf("ACK:%s:%s", argv[1], line);
    }
    return 0;
}
"""

FG = (205, 214, 244)
BG = (30, 30, 46)
SPLIT_TOP = re.compile("\u2502 +\u256d\u2500+\u256e")


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def eventually(check, description, timeout=20):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (OSError, RuntimeError, subprocess.SubprocessError) as error:
            last_error = error
        time.sleep(0.15)
    detail = f": {last_error}" if last_error else ""
    raise RuntimeError(f"timed out waiting for {description}{detail}")


@contextlib.contextmanager
def isolated_environment(environment):
    original = dict(os.environ)
    try:
        os.environ.clear()
        os.environ.update(environment)
        yield
    finally:
        os.environ.clear()
        os.environ.update(original)


def terminal_screen(pyte, cols, rows, write):
    class ReplyScreen(pyte.Screen):
        def write_process_input(self, data):
            write(data.encode())

        def report_device_status(self, mode=0, private=False, **_kwargs):
            if not private:
                return super().report_device_status(mode)
            if mode == 6:
                self.write_process_input(f"\x1b[?{self.cursor.y + 1};{self.cursor.x + 1}R")
            elif mode == 5:
                self.write_process_input("\x1b[?0n")

    return ReplyScreen(cols, rows)


class Client:
    """One bounded PTY client; terminal queries receive deterministic replies."""

    def __init__(self, binary, env, cwd, cols, rows, pyte):
        self.cols, self.rows = cols, rows
        self.master, slave = os.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.screen = terminal_screen(pyte, cols, rows, self.send)
        self.stream = pyte.ByteStream(self.screen)
        self.query_tail = b""
        self.process = subprocess.Popen(
            [str(binary)], stdin=slave, stdout=slave, stderr=slave,
            cwd=cwd, env=env, start_new_session=True,
        )
        os.close(slave)

    def pump(self, seconds=0.5):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([self.master], [], [], min(0.1, max(0, deadline - time.monotonic())))
            if not ready:
                continue
            try:
                data = os.read(self.master, 65536)
            except OSError:
                break
            if not data:
                break
            queries = self.query_tail + data
            for code, rgb in [(10, FG), (11, BG)]:
                if f"\x1b]{code};?".encode() in queries:
                    value = "/".join(f"{part:02x}{part:02x}" for part in rgb)
                    self.send(f"\x1b]{code};rgb:{value}\x1b\\".encode())
            if b"\x1b[>c" in queries or b"\x1b[>0c" in queries:
                self.send(b"\x1b[>1;4000;0c")
            self.query_tail = data[-12:]
            self.stream.feed(data)

    def send(self, data):
        os.write(self.master, data)

    def click(self, x, y):
        self.send(f"\x1b[<0;{x + 1};{y + 1}M\x1b[<0;{x + 1};{y + 1}m".encode())
        self.pump()

    def drag(self, x, start_y, end_y):
        self.send(f"\x1b[<0;{x + 1};{start_y + 1}M".encode())
        self.pump(0.1)
        self.send(f"\x1b[<32;{x + 1};{end_y + 1}M".encode())
        self.pump(0.2)
        self.send(f"\x1b[<0;{x + 1};{end_y + 1}m".encode())
        self.pump()

    def wheel(self, x, y, down):
        code = 65 if down else 64
        self.send(f"\x1b[<{code};{x + 1};{y + 1}M".encode())
        self.pump(0.2)

    def text(self, width=None):
        return "\n".join(line[:width].rstrip() for line in self.screen.display)

    def save(self, output, name, png=False):
        (output / f"{name}.txt").write_text(self.text() + "\n", encoding="utf-8")
        if png:
            self.save_png(output / f"{name}.png")

    def save_png(self, path):
        from PIL import Image, ImageDraw, ImageFont

        font_paths = [
            "/System/Library/Fonts/Menlo.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ]
        font = next((ImageFont.truetype(p, 16) for p in font_paths if Path(p).exists()), None)
        require(font is not None, "PNG output needs Menlo or DejaVu Sans Mono")
        image = Image.new("RGB", (self.cols * 10, self.rows * 22), BG)
        draw = ImageDraw.Draw(image)

        def color(value, fallback):
            if isinstance(value, str) and re.fullmatch("[0-9a-fA-F]{6}", value):
                return tuple(int(value[offset:offset + 2], 16) for offset in (0, 2, 4))
            return fallback

        for y in range(self.rows):
            for x in range(self.cols):
                cell = self.screen.buffer[y][x]
                fg, bg = color(cell.fg, FG), color(cell.bg, BG)
                if cell.reverse:
                    fg, bg = bg, fg
                left, top = x * 10, y * 22
                draw.rectangle((left, top, left + 9, top + 21), fill=bg)
                if cell.data == "\u2580":
                    draw.rectangle((left, top, left + 9, top + 10), fill=fg)
                elif cell.data.strip():
                    draw.text((left, top + 1), cell.data, fill=fg, font=font)
        image.save(path)

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        os.close(self.master)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


class Rehearsal:
    def __init__(self, args, pyte):
        self.args, self.pyte = args, pyte
        self.root = Path(tempfile.mkdtemp(prefix="herdr-rehearse-", dir="/tmp")).resolve()
        self.output = args.output.resolve() if args.output else self.root / "artifacts"
        self.output.mkdir(parents=True, exist_ok=True)
        self.socket = self.root / "config/herdr/herdr.sock"
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("HERDR_")}
        self.env.update({
            "XDG_CONFIG_HOME": str(self.root / "config"),
            "XDG_STATE_HOME": str(self.root / "state"),
            "XDG_CACHE_HOME": str(self.root / "cache"),
            "XDG_DATA_HOME": str(self.root / "data"),
            "XDG_RUNTIME_DIR": str(self.root / "run"),
            "HERDR_SOCKET_PATH": str(self.socket),
            "HERDR_CLIENT_SOCKET_PATH": str(self.root / "config/herdr/herdr-client.sock"),
            "TERM": "xterm-256color", "COLORTERM": "truecolor", "SHELL": "/bin/sh",
        })
        for name in ["config/herdr", "state", "cache", "data", "run", "bin", "pids"]:
            (self.root / name).mkdir(parents=True, exist_ok=True)
        (self.root / "config/herdr/config.toml").write_text(CONFIG, encoding="ascii")
        self.server = None
        self.server_log = None
        self.expected = {}
        self.success = False

    def isolated_socket(self):
        expected = self.root / "config/herdr/herdr.sock"
        client = expected.with_name("herdr-client.sock")
        return (
            self.socket == expected
            and Path(self.env.get("HERDR_SOCKET_PATH", "")) == expected
            and Path(self.env.get("HERDR_CLIENT_SOCKET_PATH", "")) == client
            and self.socket.resolve().is_relative_to(self.root)
            and client.resolve().is_relative_to(self.root)
        )

    def cli(self, binary, *args, timeout=15):
        require(self.isolated_socket(), "socket escaped isolation directory")
        result = subprocess.run(
            [str(binary), *args], cwd=self.root, env=self.env,
            text=True, capture_output=True, timeout=timeout,
        )
        require(result.returncode == 0, f"{binary.name} {' '.join(args)}: {result.stderr or result.stdout}")
        return result.stdout

    def result(self, binary, *args):
        response = json.loads(self.cli(binary, *args))
        require("error" not in response, f"Herdr rejected {' '.join(args)}: {response}")
        return response["result"]

    def api(self, method, params):
        require(self.isolated_socket(), "socket escaped isolation directory")
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(5)
            connection.connect(str(self.socket))
            request = {"id": "rehearse:" + method, "method": method, "params": params}
            connection.sendall(json.dumps(request).encode() + b"\n")
            response = json.loads(connection.makefile("r").readline())
        require("error" not in response, f"{method} failed: {response}")
        return response["result"]

    def panes(self, binary):
        return self.result(binary, "pane", "list")["panes"]

    def tabs(self, binary):
        return self.result(binary, "tab", "list")["tabs"]

    def read(self, binary, pane):
        return self.cli(binary, "pane", "read", pane, "--source", "recent", "--lines", "100")

    def probe_input(self, binary, phase):
        for tag, info in self.expected.items():
            marker = f"{phase}-{tag}"
            self.cli(binary, "pane", "send-text", info["pane_id"], marker)
            self.cli(binary, "pane", "send-keys", info["pane_id"], "Enter")
            eventually(
                lambda: f"ACK:{tag}:{marker}" in self.read(binary, info["pane_id"]),
                f"{tag} stdin echo ({phase})", self.args.timeout,
            )

    def build_mocks(self):
        source = self.root / "mock.c"
        source.write_text(MOCK_SOURCE, encoding="ascii")
        for agent in ["claude", "codex"]:
            result = subprocess.run(
                [self.args.cc, "-O0", "-o", str(self.root / "bin" / agent), str(source)],
                text=True, capture_output=True, timeout=60,
            )
            require(result.returncode == 0, f"mock compilation failed: {result.stderr}")

    def create_workspace(self, label):
        old = self.args.old
        before = {pane["workspace_id"] for pane in self.panes(old)}
        self.cli(old, "workspace", "create", "--label", label, "--cwd", str(self.root))
        created = [pane for pane in self.panes(old) if pane["workspace_id"] not in before]
        require(len(created) == 1, f"workspace {label} did not create exactly one pane")
        return created[0]

    def populate(self):
        old = self.args.old
        alpha = self.create_workspace("alpha")
        self.cli(old, "tab", "rename", alpha["tab_id"], "review")
        for direction in ["right", "down"]:
            self.cli(old, "pane", "split", alpha["pane_id"], "--direction", direction)
        self.cli(old, "tab", "create", "--workspace", alpha["workspace_id"], "--label", "solo")
        beta = self.create_workspace("beta")
        self.cli(old, "tab", "rename", beta["tab_id"], "review")
        self.cli(old, "pane", "split", beta["pane_id"], "--direction", "right")
        self.cli(old, "tab", "create", "--workspace", beta["workspace_id"], "--label", "review")
        self.create_workspace("gamma")
        panes = self.panes(old)
        require(len(panes) == 8, f"expected 8 mock panes, got {len(panes)}")
        for index, pane in enumerate(panes, 1):
            tag = f"p{index:02}"
            agent = "claude" if index % 2 else "codex"
            self.cli(old, "pane", "rename", pane["pane_id"], tag)
            command = "exec " + shlex.join([str(self.root / "bin" / agent), tag, str(self.root / "pids")])
            self.cli(old, "pane", "run", pane["pane_id"], command)
            self.expected[tag] = {**pane, "agent": agent}
        self.cli(old, "workspace", "focus", alpha["workspace_id"])
        eventually(
            lambda: len(self.panes(old)) == 8 and all(p.get("agent") in {"claude", "codex"} for p in self.panes(old)),
            "eight detected mock agents", self.args.timeout,
        )
        for tag, pane in self.expected.items():
            pid_file = self.root / "pids" / f"{tag}.pid"
            eventually(pid_file.exists, f"{tag} PID file", self.args.timeout)
            pane["pid"] = int(pid_file.read_text(encoding="ascii").strip())
            eventually(
                lambda: f"MOCK:{tag}:PID:{pane['pid']}" in self.read(old, pane["pane_id"]),
                f"{tag} initial screen", self.args.timeout,
            )

    def dump(self, name, value):
        (self.output / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def snapshot(self, binary, name, cols=120, rows=64):
        with Client(binary, self.env, self.root, cols, rows, self.pyte) as client:
            client.pump(3)
            client.save(self.output, name, self.args.png)
            return client.text(32)

    def prepare_sidebar_preferences(self, legacy):
        with Client(self.args.old, self.env, self.root, 120, 64, self.pyte) as client:
            client.pump(3)
            client.drag(7, 32, 16)
            client.save(self.output, "before-wide", self.args.png)
        source = (
            self.root / "config/herdr/session.json" if legacy else
            client_preferences_path(self.root / "config/herdr", self.root / "state/herdr")
        )
        group_key = "collapsed_space_keys" if legacy else "collapsed_groups"

        def saved():
            if not source.exists():
                return False
            snapshot = json.loads(source.read_text(encoding="utf-8"))
            return (
                snapshot.get("sidebar_width", 32 if not legacy else None) == 32
                and snapshot.get("sidebar_section_split") == 0.25
                and (not legacy or group_key in snapshot)
            )

        eventually(saved, "sidebar width and dragged split saved", self.args.timeout)
        snapshot = json.loads(source.read_text(encoding="utf-8"))
        if not legacy and "sidebar_width" not in snapshot:
            # Make the config-derived width explicit before changing the default.
            snapshot["sidebar_width"] = 32
            source.write_text(json.dumps(snapshot, indent=2) + "\n", encoding="utf-8")
        return {
            "sidebar_width": snapshot["sidebar_width"],
            "sidebar_section_split": snapshot["sidebar_section_split"],
            "collapsed_groups": snapshot.get(group_key, []),
        }

    def activate_candidate(self, preferences, legacy):
        require(self.isolated_socket(), "socket escaped isolation directory")
        install = self.root / "install"
        target = client_preferences_path(self.root / "config/herdr", self.root / "state/herdr")
        previous = target.read_bytes() if target.exists() else None
        with isolated_environment(self.env):
            activate(self.args.new, self.args.old, install)
        receipts = list((install / "backups").glob("*-herdr.json"))
        require(len(receipts) == 1, "activation should produce one handoff receipt")
        receipt = json.loads(receipts[0].read_text(encoding="utf-8"))
        require(receipt.get("verified"), "updater did not verify isolated handoff")
        require((install / "current").resolve() == self.args.new.parent, "client link was not switched")
        if legacy:
            migration = receipt.get("chrome_preferences", {})
            require(migration.get("status") == "migrated", "updater did not migrate legacy preferences")
            require(Path(migration["target"]) == target, "updater migrated to the wrong client preference file")
        else:
            require("chrome_preferences" not in receipt, "updater attempted legacy migration for a new server")
            require(target.read_bytes() == previous, "updater changed existing client preferences")
        require(target.resolve().is_relative_to(self.root), "preferences escaped isolation directory")
        stored = json.loads(target.read_text(encoding="utf-8"))
        require(stored.get("sidebar_width") == preferences["sidebar_width"], "sidebar width not preserved")
        require(stored.get("sidebar_section_split") == preferences["sidebar_section_split"], "sidebar split not preserved")
        require(stored.get("collapsed_groups", []) == preferences["collapsed_groups"], "collapsed groups not preserved")
        self.dump("activation-receipt.json", receipt)
        return receipt

    def check_client_preferences(self, preferences):
        # A different config default proves that the new client loaded the file.
        config = self.root / "config/herdr/config.toml"
        config.write_text(CONFIG.replace("sidebar_width = 32", "sidebar_width = 26"), encoding="ascii")
        with Client(self.args.new, self.env, self.root, 120, 64, self.pyte) as client:
            client.pump(3)
            width = preferences["sidebar_width"]
            require(
                all(client.screen.buffer[row][width - 1].data == "\u2502" for row in range(64)),
                "replacement client did not load migrated sidebar width",
            )
            dividers = [
                row for row, line in enumerate(client.screen.display)
                if line[:width - 1] == "\u2500" * (width - 1)
                and "agents" in client.screen.display[min(row + 1, 63)][:width - 1]
            ]
            expected_divider = max(3, min(61, int(64 * preferences["sidebar_section_split"] + 0.5)))
            require(dividers == [expected_divider], "replacement client did not load migrated sidebar split")
            require(SPLIT_TOP.search(client.text(32)), "replacement client has no untitled subgroup")
            client.save(self.output, "after-wide", self.args.png)
            return {"sidebar_width": width, "section_divider_row": dividers[0], "config_default_width": 26}

    def check_identity(self, before_panes, before_tabs):
        after = self.panes(self.args.new)
        self.dump("after-panes.json", after)
        self.dump("after-tabs.json", self.tabs(self.args.new))
        identity = lambda panes: sorted((p["workspace_id"], p["tab_id"], p["pane_id"], p.get("agent")) for p in panes)
        require(identity(before_panes) == identity(after), "workspace/tab/pane/agent identity changed")
        tab_identity = lambda tabs: sorted((t["workspace_id"], t["tab_id"], t["label"], t["pane_count"]) for t in tabs)
        require(tab_identity(before_tabs) == tab_identity(self.tabs(self.args.new)), "tab identity or split count changed")
        for tag, info in self.expected.items():
            os.kill(info["pid"], 0)
            require(int((self.root / "pids" / f"{tag}.pid").read_text()) == info["pid"], f"{tag} restarted")
            text = self.read(self.args.new, info["pane_id"])
            require(f"ACK:{tag}:before-{tag}" in text, f"{tag} pre-handoff screen content was lost")

    def check_click_and_scroll(self):
        new = self.args.new
        with Client(new, self.env, self.root, 120, 64, self.pyte) as client:
            client.pump(3)
            sidebar = client.text(32)
            require(len(SPLIT_TOP.findall(sidebar)) >= 2, "expected separate untitled subgroups for alpha and beta")
            target = list(self.expected)[1]
            other = list(self.expected)[0]
            self.api("pane.focus", {"pane_id": self.expected[other]["pane_id"]})
            client.pump(0.5)
            rows = [y for y, line in enumerate(client.screen.display) if target in line[:32]]
            require(len(rows) == 1, f"sidebar should show exactly one {target} card")
            client.click(7, rows[0])
            pane_id = self.expected[target]["pane_id"]
            eventually(
                lambda: any(p["pane_id"] == pane_id and p["focused"] for p in self.panes(new)),
                "split card click to focus its pane", self.args.timeout,
            )
            client.save(self.output, "after-click", self.args.png)
        with Client(new, self.env, self.root, 72, 24, self.pyte) as client:
            client.pump(3)
            initial = client.text(32)
            require(SPLIT_TOP.search(initial), "narrow client is missing its untitled subgroup")
            first_group = next(y for y, line in enumerate(client.screen.display) if SPLIT_TOP.search(line[:32]))
            for _ in range(3):
                client.wheel(7, min(first_group + 2, 22), down=True)
            client.pump(1)
            require(client.text(32) != initial, "sidebar wheel did not change the narrow viewport")
            require("\u2570" in client.text(32), "scrolled sidebar has no closing group border")
            client.save(self.output, "after-narrow-scrolled", self.args.png)
            for _ in range(8):
                client.wheel(7, min(first_group + 2, 22), down=False)

    def run(self):
        print(f"isolated directory: {self.root}", flush=True)
        self.build_mocks()
        self.server_log = (self.root / "server.stdout").open("wb")
        self.server = subprocess.Popen(
            [str(self.args.old), "server"], cwd=self.root, env=self.env,
            stdin=subprocess.DEVNULL, stdout=self.server_log, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        eventually(lambda: self.socket.exists() and self.result(self.args.old, "pane", "list"), "isolated server", self.args.timeout)
        self.populate()
        self.probe_input(self.args.old, "before")
        before_server = json.loads(self.cli(self.args.old, "status", "server", "--json"))
        legacy = before_server.get("version") == "0.7.4"
        preferences = self.prepare_sidebar_preferences(legacy)
        before_panes, before_tabs = self.panes(self.args.old), self.tabs(self.args.old)
        self.dump("before-panes.json", before_panes)
        self.dump("before-tabs.json", before_tabs)
        print("handoff: old -> new (isolated socket only)", flush=True)
        activation = self.activate_candidate(preferences, legacy)
        eventually(
            lambda: len(self.panes(self.args.new)) == 8
            and all(p.get("agent") in {"claude", "codex"} for p in self.panes(self.args.new)),
            "replacement server and detected agents", self.args.timeout,
        )
        self.check_identity(before_panes, before_tabs)
        self.probe_input(self.args.new, "after")
        client_preferences = self.check_client_preferences(preferences)
        self.check_click_and_scroll()
        self.dump("result.json", {"passed": True, "mock_agents": 8, "pids": sorted(p["pid"] for p in self.expected.values()),
                                  "checks": ["identity", "pids", "screen content", "stdin", "subgroups", "click", "narrow scroll",
                                             "updater activation", "legacy preference migration" if legacy else "client preference preservation",
                                             "client preference geometry"],
                                  "sidebar_preferences": preferences, "client_preferences": client_preferences,
                                  "activation": activation})
        self.success = True
        print("PASS: 8 mock agents kept their PID, pane/tab identity, screen content and input; subgroup/click/scroll checks passed", flush=True)

    def cleanup(self):
        if self.success and self.args.keep:
            print(f"kept: {self.root}\nartifacts: {self.output}\nAPI socket: {self.socket}", flush=True)
            return
        # Connect to this exact temporary socket; never invoke a default stop path.
        if self.isolated_socket():
            with contextlib.suppress(OSError):
                with socket.socket(socket.AF_UNIX) as connection:
                    connection.settimeout(2)
                    connection.connect(str(self.socket))
                    request = {"id": "rehearse:stop", "method": "server.stop", "params": {}}
                    connection.sendall(json.dumps(request).encode() + b"\n")
                    connection.recv(65536)
        if self.server is not None:
            with contextlib.suppress(subprocess.TimeoutExpired):
                self.server.wait(timeout=3)
            if self.server.poll() is None:
                self.server.terminate()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    self.server.wait(timeout=3)
                if self.server.poll() is None:
                    self.server.kill()
                    self.server.wait(timeout=3)
        # The import daemon is not our Popen child. Its argv includes this
        # run's unique handoff socket, so a failed API stop can still be cleaned.
        for force in [False, True]:
            with contextlib.suppress(OSError, subprocess.SubprocessError):
                listing = subprocess.check_output(["ps", "-axo", "pid=,command="], text=True)
                for line in listing.splitlines():
                    fields = line.strip().split(None, 1)
                    if len(fields) != 2:
                        continue
                    pid, command = fields
                    if "server --handoff-import" in command and str(self.root) + "/" in command:
                        with contextlib.suppress(OSError):
                            os.kill(int(pid), signal.SIGKILL if force else signal.SIGTERM)
            if not force:
                time.sleep(0.2)
        # Only the recorded mock PIDs are eligible; verify the unique argv path first.
        for file in (self.root / "pids").glob("*.pid"):
            with contextlib.suppress(OSError, ValueError, subprocess.SubprocessError):
                pid = int(file.read_text().strip())
                command = subprocess.check_output(["ps", "-p", str(pid), "-o", "command="], text=True)
                if str(self.root / "bin") in command:
                    os.kill(pid, signal.SIGTERM)
        if self.server_log is not None:
            self.server_log.close()
        if self.args.output:
            for log in self.root.rglob("*.log"):
                shutil.copy2(log, self.output / f"log-{log.name}")
            if (self.root / "server.stdout").exists():
                shutil.copy2(self.root / "server.stdout", self.output / "server.stdout")
        shutil.rmtree(self.root)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("old", type=Path, help="currently installed build to import from")
    parser.add_argument("new", type=Path, help="candidate build with subgroup support")
    parser.add_argument("--keep", action="store_true", help="keep the isolated server and files after a successful rehearsal")
    parser.add_argument("--output", type=Path, help="artifact directory outside the disposable server directory")
    parser.add_argument("--png", action="store_true", help="also render terminal captures as PNG using Pillow")
    parser.add_argument("--cc", default="cc", help="C compiler used to build the two mock agent executables")
    parser.add_argument("--timeout", type=float, default=20, help="per-check timeout in seconds")
    args = parser.parse_args()
    for name in ["old", "new"]:
        path = getattr(args, name).expanduser().resolve()
        require(path.is_file() and os.access(path, os.X_OK), f"{name} binary is not executable: {path}")
        setattr(args, name, path)
    try:
        import pyte
        if args.png:
            import PIL.Image
    except ImportError as error:
        parser.error(f"missing {error.name}; install pyte (and Pillow for --png) in a disposable virtual environment")
    require(args.timeout > 0, "--timeout must be positive")

    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f"received signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    rehearsal = Rehearsal(args, pyte)
    try:
        rehearsal.run()
    except KeyboardInterrupt:
        print("FAIL: interrupted; cleaning isolated session", file=sys.stderr)
        return 130
    except (OSError, RuntimeError, subprocess.SubprocessError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        if args.output:
            rehearsal.dump("failure.json", {"passed": False, "error": str(error)})
        return 1
    finally:
        rehearsal.cleanup()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
