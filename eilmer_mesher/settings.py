"""
Persistent user settings (display units, keyboard shortcuts) via QSettings.

Stored in the platform's usual place (~/.config/eilmer-mesher/eilmer-mesher.conf
on Linux). Set EMESH_SETTINGS to an .ini path to use a different file (the tests do).
"""
import os

from PySide6.QtCore import QSettings

from .units import DEFAULT_UNITS

# id -> (menu text, default shortcut)
DEFAULT_SHORTCUTS = {
    "tool.select": ("Select", "S"),
    "tool.line": ("Line", "L"),
    "tool.profile": ("Profile", "P"),
    "tool.arc": ("Arc", "A"),
    "tool.circle": ("Circle", "C"),
    "tool.rectangle": ("Rectangle", "R"),
    "tool.triangle": ("Triangle", "T"),
    "tool.bezier": ("Bezier", "B"),
    "tool.spline": ("Spline", "N"),
    "tool.point": ("Point", "O"),
    "tool.trim": ("Quick trim", "Q"),
    "tool.merge": ("Merge", "J"),
    "tool.mirror": ("Mirror", "Shift+M"),
    "file.new": ("New", "Ctrl+N"),
    "file.open": ("Open", "Ctrl+O"),
    "file.save": ("Save", "Ctrl+S"),
    "file.save_as": ("Save as", "Ctrl+Shift+S"),
    "file.import": ("Import points", "Ctrl+I"),
    "file.quit": ("Quit", "Ctrl+Q"),
    "edit.undo": ("Undo", "Ctrl+Z"),
    "edit.redo": ("Redo", "Ctrl+Shift+Z"),
    "edit.delete": ("Delete", "Del"),
    "edit.select_all": ("Select all", "Ctrl+A"),
    "edit.toggle_reference": ("Convert to/from reference", "Shift+X"),
    "sketch.reference_mode": ("Draw reference lines", "X"),
    "snap.grid": ("Grid snap", "G"),
    "view.fit": ("Zoom to fit", "Space"),
    "view.regions": ("Show regions", "Shift+R"),
}

# Keys held down while drawing (single keys, not menu shortcuts).
DEFAULT_HOLD_KEYS = {
    "hold.horizontal": ("Hold for horizontal", "H"),
    "hold.vertical": ("Hold for vertical", "V"),
}


def app_settings():
    path = os.environ.get("EMESH_SETTINGS")
    if path:
        return QSettings(path, QSettings.IniFormat)
    return QSettings("eilmer-mesher", "eilmer-mesher")


def shortcut(key):
    defaults = {**DEFAULT_SHORTCUTS, **DEFAULT_HOLD_KEYS}
    return str(app_settings().value(f"shortcuts/{key}", defaults[key][1]))


def set_shortcuts(values):
    s = app_settings()
    defaults = {**DEFAULT_SHORTCUTS, **DEFAULT_HOLD_KEYS}
    for key, seq in values.items():
        if seq == defaults[key][1]:
            s.remove(f"shortcuts/{key}")
        else:
            s.setValue(f"shortcuts/{key}", seq)
    s.sync()


def units_name():
    return str(app_settings().value("view/units", DEFAULT_UNITS))


def set_units_name(name):
    s = app_settings()
    s.setValue("view/units", name)
    s.sync()
