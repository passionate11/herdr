"""Carry legacy default-session sidebar preferences into the client shell."""

import json
import math
import os
from pathlib import Path
import shutil
import tempfile


def _herdr_directories():
    def directory(xdg, windows, windows_suffix, home_suffix, temporary):
        if xdg in os.environ:
            return Path(os.environ[xdg]) / "herdr"
        if os.name == "nt":
            if windows in os.environ:
                return Path(os.environ[windows]) / "herdr"
            if "USERPROFILE" in os.environ:
                return Path(os.environ["USERPROFILE"]) / windows_suffix / "herdr"
        if "HOME" in os.environ:
            return Path(os.environ["HOME"]) / home_suffix / "herdr"
        return Path(tempfile.gettempdir()) / temporary

    config = directory(
        "XDG_CONFIG_HOME", "APPDATA", "AppData/Roaming", ".config", "herdr"
    )
    state = directory(
        "XDG_STATE_HOME", "LOCALAPPDATA", "AppData/Local", ".local/state", "herdr-state"
    )
    return config, state


def client_preferences_path(config_dir, state_dir):
    """Match ClientChromePreferences::path_for_local_endpoint in release builds."""
    client_socket = Path(config_dir) / "herdr-client.sock"
    socket_bytes = os.fsencode(client_socket).decode("utf-8", "replace").encode("utf-8")
    hashed = 0xCBF29CE484222325
    for byte in socket_bytes:
        hashed = ((hashed ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return Path(state_dir) / "client-shell" / f"local-{hashed:016x}.json"


def _legacy_fields(snapshot):
    fields = {}
    width = snapshot.get("sidebar_width")
    if width is not None:
        if type(width) is not int or not 0 <= width <= 65535:
            raise RuntimeError("Invalid legacy sidebar_width")
        fields["sidebar_width"] = width
    split = snapshot.get("sidebar_section_split")
    if split is not None:
        if type(split) not in (int, float) or not math.isfinite(split):
            raise RuntimeError("Invalid legacy sidebar_section_split")
        fields["sidebar_section_split"] = split
    groups = snapshot.get("collapsed_space_keys")
    if groups is not None:
        if not isinstance(groups, list) or any(not isinstance(group, str) for group in groups):
            raise RuntimeError("Invalid legacy collapsed_space_keys")
        fields["collapsed_groups"] = groups
    return fields


def _read_object(path, description):
    try:
        with path.open(encoding="utf-8") as stream:
            value = json.load(stream)
    except (UnicodeError, ValueError) as error:
        raise RuntimeError(f"Invalid {description} at {path}: {error}") from error
    if not isinstance(value, dict):
        raise RuntimeError(f"{description} at {path} must be a JSON object")
    return value


def migrate_legacy_preferences(backup_dir):
    """Copy missing chrome fields; the caller checks the old server's version."""
    config_dir, state_dir = _herdr_directories()
    source = config_dir / "session.json"
    target = client_preferences_path(config_dir, state_dir)
    receipt = {
        "status": "source_missing",
        "source": str(source),
        "target": str(target),
        "migrated_fields": [],
        "backup": None,
    }
    if not source.exists():
        return receipt
    snapshot = _read_object(source, "legacy session metadata")
    fields = _legacy_fields(snapshot)
    existing = target.exists()
    preferences = {}
    if existing:
        preferences = _read_object(target, "client preferences")
    missing = {key: value for key, value in fields.items() if key not in preferences}
    receipt["status"] = "no_changes"
    if not missing:
        return receipt

    if existing:
        backup_dir = Path(backup_dir)
        backup_dir.mkdir(parents=True, exist_ok=True)
        backup_fd, backup_name = tempfile.mkstemp(
            prefix=target.stem + ".", suffix=".json", dir=backup_dir
        )
        os.close(backup_fd)
        shutil.copy2(target, backup_name)
        receipt["backup"] = backup_name

    preferences.update(missing)
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", prefix=target.name + ".tmp-",
            dir=target.parent, delete=False,
        ) as stream:
            temporary = Path(stream.name)
            json.dump(preferences, stream, indent=2, allow_nan=False)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    receipt.update(status="migrated", migrated_fields=sorted(missing))
    return receipt
