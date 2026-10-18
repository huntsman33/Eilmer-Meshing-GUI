"""Main window: sketch canvas, tool palette, typed input, menus."""
import os

from PySide6.QtCore import Qt
from PySide6.QtGui import QAction, QActionGroup, QKeySequence
from PySide6.QtWidgets import (QFileDialog, QHBoxLayout, QLabel, QLineEdit, QMainWindow, QMessageBox,
                               QToolButton, QVBoxLayout, QWidget)

from . import settings
from .icons import icon
from .model.document import Document
from .settings import DEFAULT_HOLD_KEYS, DEFAULT_SHORTCUTS
from .shortcuts_dialog import ShortcutsDialog
from .sketch import snapping
from .sketch.canvas import SketchCanvas
from .sketch.edit_dialogs import edit_entity
from .sketch.import_dialog import ImportPointsDialog
from .sketch.importers import parse_point_file
from .sketch.model import SketchModel
from .sketch.tools import SKETCH_TOOLS
from .units import UNITS, Units

PROJECT_FILTER = "Eilmer Mesher project (*.emesh.json)"
PROJECT_SUFFIX = ".emesh.json"
# Qt's own file dialog: native/portal dialogs can hang under WSLg (no desktop portal).
FILE_DIALOG_OPTIONS = QFileDialog.DontUseNativeDialog


class InputBar(QWidget):
    """
    NX-style typed input, in display units: x, y | @dx, dy | @L<deg | a single
    value (length, radius, angle). A single value locks live while you type;
    click or Enter places it.
    """

    def __init__(self, canvas, parent=None):
        super().__init__(parent)
        self.canvas = canvas
        self.prompt = QLabel()
        self.prompt.setMinimumWidth(220)
        self.edit = QLineEdit()
        self.edit.returnPressed.connect(self._submit)
        self.edit.textEdited.connect(canvas.preview_text)
        canvas.clearTyped.connect(self.clear)
        self.message = QLabel()
        self.message.setStyleSheet("color: #c0392b")
        lay = QHBoxLayout(self)
        lay.setContentsMargins(6, 2, 6, 2)
        lay.addWidget(self.prompt)
        lay.addWidget(self.edit, 1)
        lay.addWidget(self.message)
        self.refresh()

    def start_typing(self, text):
        self.edit.setFocus()
        self.edit.insert(text)
        self.canvas.preview_text(self.edit.text())

    def clear(self):
        self.edit.clear()
        self.message.clear()

    def keyPressEvent(self, ev):
        if ev.key() == Qt.Key_Escape:
            self.clear()
            self.canvas.preview_text("")
            self.canvas.setFocus()
            return
        super().keyPressEvent(ev)

    def _submit(self):
        text = self.edit.text().strip()
        if not text:
            self.canvas.setFocus()
            return
        err = self.canvas.submit_text(text)
        self.message.setText(err or "")
        if not err:
            self.edit.clear()
            self.canvas.preview_text("")
            self.canvas.setFocus()

    def refresh(self):
        tool = self.canvas.tool
        self.prompt.setText(f"<b>{tool.label}</b>  {tool.prompt()}" if tool else "")
        self.edit.setEnabled(bool(tool and tool.name not in ("select", "trim", "merge", "mirror")))
        u = self.canvas.units.name
        self.edit.setPlaceholderText(f"type in {u}:  x, y    @dx, dy    @L<deg    or a length / radius / angle")


class MainWindow(QMainWindow):
    def __init__(self, path=None):
        super().__init__()
        self.model = SketchModel(parent=self)
        self.canvas = SketchCanvas(self.model)
        self.canvas.units = Units(settings.units_name())
        self.tools = {cls.name: cls(self.canvas) for cls, _ in SKETCH_TOOLS}
        self.path = None

        self.input_bar = InputBar(self.canvas)
        central = QWidget()
        lay = QVBoxLayout(central)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(0)
        lay.addWidget(self.canvas, 1)
        lay.addWidget(self.input_bar)
        self.setCentralWidget(central)

        self.hint_label = QLabel()
        self.coord_label = QLabel()
        self.coord_label.setMinimumWidth(340)
        self.coord_label.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        self.statusBar().addWidget(self.hint_label, 1)
        self.statusBar().addPermanentWidget(self.coord_label)

        self._make_actions()
        self._make_menus_and_toolbars()
        self.apply_shortcuts()

        self.canvas.cursorMoved.connect(self._show_cursor)
        self.canvas.typedText.connect(self.input_bar.start_typing)
        self.canvas.escapeIdle.connect(lambda: self.set_tool("select"))
        self.canvas.toolStateChanged.connect(self._refresh_tool_text)
        self.canvas.editRequested.connect(self.edit_entity)
        self.canvas.message.connect(lambda text: self.statusBar().showMessage(text, 6000))
        self.model.undo_stack.cleanChanged.connect(self._update_title)
        self.model.changed.connect(self._update_title)

        self.set_tool("select")
        self.resize(1280, 820)
        if path:
            self.open_path(path)
        self._update_title()

    # --- actions ---------------------------------------------------------------
    def _action(self, sid, text, slot, icon_name=None, checkable=False):
        """An action whose shortcut comes from settings under id sid (None: no shortcut)."""
        act = QAction(text, self)
        if icon_name:
            act.setIcon(icon(icon_name))
        act.setCheckable(checkable)
        if slot is not None:
            act.triggered.connect(slot)
        if sid is not None:
            self.actions_by_id[sid] = act
        return act

    def _make_actions(self):
        self.actions_by_id = {}
        A = self._action
        self.act_new = A("file.new", "&New", self.new_document)
        self.act_open = A("file.open", "&Open...", self.open_dialog)
        self.act_save = A("file.save", "&Save", self.save)
        self.act_save_as = A("file.save_as", "Save &As...", self.save_as)
        self.act_import = A("file.import", "&Import points (.dat/.txt)...", self.import_points, "import")
        self.act_quit = A("file.quit", "&Quit", self.close)

        self.act_undo = self.model.undo_stack.createUndoAction(self, "&Undo")
        self.act_redo = self.model.undo_stack.createRedoAction(self, "&Redo")
        self.actions_by_id["edit.undo"] = self.act_undo
        self.actions_by_id["edit.redo"] = self.act_redo
        self.act_delete = A("edit.delete", "&Delete", self.model.delete_selection)
        self.act_select_all = A("edit.select_all", "Select &all",
                                lambda: self.model.set_selection(self.model.entity_ids()))
        self.act_toggle_ref = A("edit.toggle_reference", "Convert to/from &reference",
                                lambda: self.model.toggle_reference())
        self.act_ref_mode = A("sketch.reference_mode", "Draw reference (dashed) lines", self._set_reference_mode,
                              "reference", checkable=True)
        self.act_fit = A("view.fit", "Zoom to &fit", self.canvas.zoom_to_fit, "fit")
        self.act_regions = A("view.regions", "Show &regions", self._set_show_regions, "regions", checkable=True)
        self.act_shortcuts = A(None, "&Keyboard shortcuts...", self.edit_shortcuts)

        self.tool_group = QActionGroup(self)
        self.tool_actions = {}
        for cls, tip in SKETCH_TOOLS:
            act = A(f"tool.{cls.name}", cls.label, None, cls.name, checkable=True)
            act.setData(tip)
            act.triggered.connect(lambda checked=False, n=cls.name: self.set_tool(n))
            self.tool_group.addAction(act)
            self.tool_actions[cls.name] = act

        self.snap_actions = {}
        for kind in snapping.ALL_KINDS:
            act = A("snap.grid" if kind == snapping.GRID else None, kind.capitalize(), None, checkable=True)
            act.setChecked(self.canvas.snapper.enabled[kind])
            act.toggled.connect(lambda on, k=kind: self.canvas.snapper.enabled.__setitem__(k, on))
            self.snap_actions[kind] = act

        self.units_group = QActionGroup(self)
        self.unit_actions = {}
        for name in UNITS:
            act = QAction(name, self, checkable=True)
            act.setChecked(name == self.canvas.units.name)
            act.triggered.connect(lambda checked=False, n=name: self.set_units(n))
            self.units_group.addAction(act)
            self.unit_actions[name] = act

    def _make_menus_and_toolbars(self):
        mb = self.menuBar()
        m = mb.addMenu("&File")
        for a in (self.act_new, self.act_open, self.act_save, self.act_save_as):
            m.addAction(a)
        m.addSeparator()
        m.addAction(self.act_import)
        m.addSeparator()
        m.addAction(self.act_quit)

        m = mb.addMenu("&Edit")
        for a in (self.act_undo, self.act_redo):
            m.addAction(a)
        m.addSeparator()
        for a in (self.act_delete, self.act_select_all, self.act_toggle_ref):
            m.addAction(a)
        m.addSeparator()
        m.addAction(self.act_shortcuts)

        m = mb.addMenu("&Sketch")
        for act in self.tool_actions.values():
            m.addAction(act)
        m.addSeparator()
        m.addAction(self.act_ref_mode)
        snap_menu = m.addMenu("Snap to")
        for act in self.snap_actions.values():
            snap_menu.addAction(act)

        m = mb.addMenu("&View")
        m.addAction(self.act_fit)
        m.addAction(self.act_regions)
        units_menu = m.addMenu("&Units")
        for act in self.unit_actions.values():
            units_menu.addAction(act)

        tb = self.addToolBar("Sketch")
        tb.setObjectName("sketch-toolbar")
        tb.setMovable(False)
        for act in self.tool_actions.values():
            tb.addAction(act)
        tb.addSeparator()
        tb.addAction(self.act_ref_mode)
        snap_button = QToolButton()
        snap_button.setIcon(icon("snap"))
        snap_button.setToolTip("Snap options")
        snap_button.setPopupMode(QToolButton.InstantPopup)
        snap_button.setMenu(snap_menu)
        tb.addWidget(snap_button)
        tb.addAction(self.act_regions)
        tb.addSeparator()
        tb.addAction(self.act_import)
        tb.addAction(self.act_fit)

    # --- shortcuts ---------------------------------------------------------------
    def current_shortcuts(self):
        keys = {**DEFAULT_SHORTCUTS, **DEFAULT_HOLD_KEYS}
        return {k: settings.shortcut(k) for k in keys}

    def apply_shortcuts(self):
        current = self.current_shortcuts()
        for sid, act in self.actions_by_id.items():
            seq = QKeySequence(current.get(sid, ""))
            act.setShortcut(seq)
            text = seq.toString(QKeySequence.NativeText)
            tip = act.data() or act.text().replace("&", "")
            act.setToolTip(f"{tip}  [{text}]" if text else tip)
        for which in ("horizontal", "vertical"):
            seq = QKeySequence(current[f"hold.{which}"])
            if not seq.isEmpty():
                self.canvas.hold_keys[which] = seq[0].key()
        self.canvas.held.clear()

    def edit_shortcuts(self):
        dlg = ShortcutsDialog(self.current_shortcuts(), self)
        if dlg.exec():
            settings.set_shortcuts(dlg.values())
            self.apply_shortcuts()

    # --- tools, units, view -----------------------------------------------------------
    def set_tool(self, name):
        self.canvas.set_tool(self.tools[name])
        self.tool_actions[name].setChecked(True)
        self.input_bar.clear()
        self.canvas.setFocus()

    def set_units(self, name):
        self.canvas.units = Units(name)
        self.unit_actions[name].setChecked(True)
        settings.set_units_name(name)
        self.input_bar.refresh()
        if self.canvas.last_snap is not None:
            self._show_cursor(self.canvas.last_snap)
        self.canvas.update()

    def _set_reference_mode(self, on):
        self.model.reference_mode = on
        self.canvas.update()

    def _set_show_regions(self, on):
        self.canvas.show_regions = on
        self.canvas.update()

    def _refresh_tool_text(self):
        self.input_bar.refresh()
        tool = self.canvas.tool
        self.hint_label.setText(tool.hint() if tool else "")

    def _show_cursor(self, snap):
        units = self.canvas.units
        digits = units.digits(self.canvas.grid_step())
        x, y = units.point_to_display(snap.point)
        kind = f"   [{snap.kind}]" if snap.kind else ""
        self.coord_label.setText(f"x = {x:.{digits}f}   y = {y:.{digits}f}  {units.name}{kind}")
        self.input_bar.refresh()

    def edit_entity(self, eid):
        edit_entity(self, self.model, self.canvas.units, eid)
        self.canvas.setFocus()

    # --- files -------------------------------------------------------------------
    def _update_title(self):
        name = os.path.basename(self.path) if self.path else "untitled"
        self.setWindowTitle(f"{name}[*] - Eilmer Mesher")
        self.setWindowModified(not self.model.undo_stack.isClean())

    def _confirm_discard(self):
        if self.model.undo_stack.isClean():
            return True
        r = QMessageBox.question(self, "Unsaved changes", "Save changes to the current project?",
                                 QMessageBox.Save | QMessageBox.Discard | QMessageBox.Cancel)
        if r == QMessageBox.Save:
            return self.save()
        return r == QMessageBox.Discard

    def new_document(self):
        if self._confirm_discard():
            self.path = None
            self.model.reset(Document())
            self.canvas.zoom_to_fit()
            self._update_title()

    def open_dialog(self):
        if not self._confirm_discard():
            return
        path, _ = QFileDialog.getOpenFileName(self, "Open project", "", PROJECT_FILTER + ";;All files (*)",
                                              options=FILE_DIALOG_OPTIONS)
        if path:
            self.open_path(path)

    def open_path(self, path):
        try:
            doc = Document.load(path)
        except Exception as e:
            QMessageBox.critical(self, "Open failed", f"Could not open {path}:\n{e}")
            return
        self.path = path
        self.model.reset(doc)
        self.model.undo_stack.setClean()
        self.canvas.zoom_to_fit()
        self._update_title()

    def save(self):
        if self.path is None:
            return self.save_as()
        self.model.doc.save(self.path)
        self.model.undo_stack.setClean()
        self._update_title()
        return True

    def save_as(self):
        path, _ = QFileDialog.getSaveFileName(self, "Save project", self.path or "project" + PROJECT_SUFFIX,
                                              PROJECT_FILTER, options=FILE_DIALOG_OPTIONS)
        if not path:
            return False
        if not path.endswith(PROJECT_SUFFIX):
            path += PROJECT_SUFFIX
        self.path = path
        return self.save()

    def import_points(self):
        path, _ = QFileDialog.getOpenFileName(self, "Import point file", "",
                                              "Point files (*.dat *.txt *.csv *.xy);;All files (*)",
                                              options=FILE_DIALOG_OPTIONS)
        if not path:
            return
        try:
            with open(path) as f:
                pf = parse_point_file(f.read(), os.path.splitext(os.path.basename(path))[0])
        except (OSError, ValueError) as e:
            QMessageBox.critical(self, "Import failed", f"Could not read {path}:\n{e}")
            return
        dlg = ImportPointsDialog(pf, path, self, self.canvas.units)
        if dlg.exec():
            ids = self.model.add_curves(dlg.curves(), f"Import {pf.name}")
            self.model.set_selection(ids)
            self.canvas.zoom_to_fit()

    def closeEvent(self, ev):
        if self._confirm_discard():
            ev.accept()
        else:
            ev.ignore()
