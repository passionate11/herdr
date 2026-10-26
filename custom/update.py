#!/usr/bin/env python3
"""Fetch this personal branch, build and rehearse, then optionally activate it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from preferences import migrate_legacy_preferences


def run(args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def output(args):
    return run(args, capture_output=True, text=True).stdout.strip()


def pane_identity(binary):
    data = json.loads(output([binary, "pane", "list"]))
    if "error" in data:
        raise RuntimeError(f"Cannot inspect active panes: {data['error']}")
    return sorted((p["pane_id"], p["tab_id"], p["workspace_id"]) for p in data["result"]["panes"])


def pane_processes(binary, panes):
    processes = {}
    for pane_id, *_ in panes:
        data = json.loads(output([binary, "pane", "process-info", "--pane", pane_id]))
        if "error" in data:
            raise RuntimeError(f"Cannot inspect {pane_id} processes: {data['error']}")
        info = data["result"]["process_info"]
        # Check persistent shell and agent processes, not a transient tool that
        # can legitimately finish while a busy agent is being handed over.
        processes[pane_id] = {
            "shell_pid": info.get("shell_pid"),
            "agent_pids": sorted(
                process["pid"]
                for process in info.get("foreground_processes", [])
                if process["name"] in {"claude", "codex"}
                or Path(process.get("argv0") or "").name in {"claude", "codex"}
            ),
        }
    return processes


def switch_current(candidate, root):
    staged = root / ("current.next-" + str(os.getpid()))
    if staged.is_symlink():
        staged.unlink()
    staged.symlink_to(candidate.parent)
    staged.replace(root / "current")


def activate(candidate, current, root):
    server = json.loads(output([current, "status", "server", "--json"]))
    if not server.get("running") or not server.get("compatible"):
        raise RuntimeError("The current executable must match the running default server")
    before_panes = pane_identity(current)
    before_processes = pane_processes(current, before_panes)
    initial_client = (root / "current").resolve() if (root / "current").is_symlink() else current
    backup_dir = root / "backups"
    backup_dir.mkdir(parents=True, exist_ok=True)
    backup = backup_dir / (time.strftime("%Y%m%d-%H%M%S") + "-herdr")
    shutil.copy2(current.resolve(), backup)
    backup.chmod(0o555)
    receipt = {
        "source": str(current),
        "source_backup": str(backup),
        "candidate": str(candidate),
        "sha256": hashlib.sha256(candidate.read_bytes()).hexdigest(),
        "previous_client": str(initial_client),
        "server_before": server,
        "panes_before": before_panes,
        "processes_before": before_processes,
    }
    record = backup.with_suffix(".json")
    config_dir = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "herdr"
    receipt["configuration_backups"] = {}
    for name in ["config.toml", "session.json"]:
        source = config_dir / name
        if source.is_file():
            saved = backup.with_name(backup.name + "-" + name)
            shutil.copy2(source, saved)
            saved.chmod(0o600)
            receipt["configuration_backups"][name] = str(saved)
    if server.get("version") == "0.7.4":
        receipt["chrome_preferences"] = migrate_legacy_preferences(backup_dir)
    record.write_text(json.dumps(receipt, indent=2) + "\n")
    print("Live handoff will detach Herdr clients. Reconnect with herdr.", flush=True)
    try:
        run([current, "server", "live-handoff", "--import-exe", candidate])
    except subprocess.CalledProcessError as error:
        receipt["handoff_error"] = str(error)
        record.write_text(json.dumps(receipt, indent=2) + "\n")
        raise
    # The replacement server is already active. Its matching client must be
    # reachable even if a subsequent verification needs manual inspection.
    receipt["handoff_complete"] = True
    record.write_text(json.dumps(receipt, indent=2) + "\n")
    try:
        switch_current(candidate, root)
    except OSError as error:
        receipt["client_switch_error"] = str(error)
        record.write_text(json.dumps(receipt, indent=2) + "\n")
        raise RuntimeError(f"Candidate server is active; reconnect with {candidate} and inspect {record}") from error
    try:
        after_panes = pane_identity(candidate)
        after_processes = pane_processes(candidate, after_panes)
        receipt.update(panes_after=after_panes, processes_after=after_processes)
        if after_panes != before_panes:
            raise RuntimeError("Workspace/tab/pane identities changed")
        for pane_id, before in before_processes.items():
            after = after_processes[pane_id]
            if (
                before["shell_pid"] != after["shell_pid"]
                or not set(before["agent_pids"]).issubset(after["agent_pids"])
            ):
                raise RuntimeError(f"Process IDs changed for {pane_id}")
        receipt["verified"] = True
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        receipt["verification_error"] = str(error)
        raise RuntimeError(f"Candidate server is active; reconnect with herdr and inspect {record}: {error}") from error
    finally:
        record.write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"Active: {candidate}; retained {len(before_panes)} panes and their process IDs")
    print(f"Local handoff receipt: {record}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-fetch", action="store_true", help="build the current local branch")
    parser.add_argument("--activate", action="store_true", help="handoff the live server after rehearsal passes")
    parser.add_argument("--current", type=Path, help="current binary; defaults to active custom build or PATH")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    os.chdir(repo)
    if output(["git", "status", "--porcelain"]):
        raise RuntimeError("Commit the custom branch before building an upgrade candidate")
    if not args.no_fetch:
        branch = output(["git", "branch", "--show-current"])
        if not branch.startswith("custom/"):
            raise RuntimeError("Run updates from the personal custom branch")
        run(["git", "fetch", "personal", branch])
        run(["git", "merge", "--ff-only", "FETCH_HEAD"])
    root = Path(
        os.environ.get("HERDR_CUSTOM_INSTALL_ROOT", Path.home() / ".local/share/herdr-custom")
    ).expanduser().resolve()
    current = args.current.expanduser() if args.current else root / "current/herdr"
    if not current.is_file():
        if args.current:
            raise RuntimeError(f"Current binary does not exist: {current}")
        found = shutil.which("herdr")
        if not found:
            raise RuntimeError("Pass --current with the running server's binary")
        current = Path(found)
        with current.open("rb") as binary:
            prefix = binary.read(1024)
        if b"No active custom Herdr build" in prefix:
            raise RuntimeError("The custom launcher has no active build; pass --current with the running server's actual binary")
    current = current.expanduser().resolve()
    if not os.access(current, os.X_OK):
        raise RuntimeError(f"Current binary is not executable: {current}")
    run(["bash", repo / "custom/build.sh"])
    commit = output(["git", "rev-parse", "--short=12", "HEAD"])
    candidate = root / f"builds/0.9.5-grouped.1-{commit}/herdr"
    if not candidate.is_file():
        raise RuntimeError("Build output does not match the current commit")
    for key in list(os.environ):
        if key.startswith("HERDR_"):
            del os.environ[key]
    run([sys.executable, repo / "custom/rehearse.py", str(current.resolve()), str(candidate)])
    if args.activate:
        activate(candidate, current.resolve(), root)
    else:
        print(f"Verified candidate: {candidate}")
        print("Use herdr-custom-update --activate to build, verify and switch the running session.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
