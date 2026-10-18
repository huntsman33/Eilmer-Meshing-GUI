"""Main window: sketch canvas, tool palette, typed input, menus."""
import math
import os

from PySide6.QtCore import Qt
from PySide6.QtGui import QAction, QActionGroup, QKeySequence
from PySide6.QtWidgets import (QFileDialog, QHBoxLayout, QLabel, QLineEdit, QMainWindow, QMenu,
                               QMessageBox, QToolButton, QVBoxLayout, QWidget)

from .icons import icon
from .model.document import Document
from .sketch import snapping
from .sketch.canvas import SketchCanvas
from .sketch.import_dialog import ImportPointsDialog
from .sketch.importers import parse_point_file
from .sketch.model import SketchModel
from .sketch.tools import SKETCH_TOOLS

PROJECT_FILTER = "Eilmer Mesher project (*.emesh.json)"
PROJECT_SUFFIX = ".emesh.json"
# Qt's own file dialog: native/portal dialogs can hang under WSLg (no desktop portal).
FILE_DIALOG_OPTIONS = QFileDialog.DontUseNativeDialog


class InputBar(QWidget):
    """NX-style typed input: x, y | @dx, dy | @L<deg | single value (length, radius)."""

    def __init__(self, canvas, parent=None):
        super().__init__(parent)
        self.canvas = canvas
        self.prompt = QLabel()
        self.prompt.setMinimumWidth(220)
        self.edit = QLineEdit()
        self.edit.setPlaceholderText("type coordinates: x, y   @dx, dy   @L<deg   or a length/radius")
        self.edit.returnPressed.connect(self._submit)
        self.message = QLabel()
        self.message.setStyleSheet("color: #c0392b")
        lay = QHBoxLayout(self)
        lay.setContentsMargins(6, 2, 6, 2)
        lay.addWidget(self.prompt)
        lay.addWidget(self.edit, 1)
        lay.addWidget(self.message)

    def start_typing(self, text):
        self.edit.setFocus()
        self.edit.insert(text)

    def keyPressEvent(self, ev):
        if ev.key() == Qt.Key_Escape:
            self.edit.clear()
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
            self.canvas.setFocus()

    def refresh(self):
        tool = self.canvas.tool
        self.prompt.setText(f"<b>{tool.label}</b>  {tool.prompt()}" if tool else "")
        self.edit.setEnabled(bool(tool and tool.name != "select"))


class MainWindow(QMainWindow):
    def __init__(self, path=None):
        super().__init__()
        self.model = SketchModel(parent=self)
        self.canvas = SketchCanvas(self.model)
        self.tools = {cls.name: cls(self.canvas) for cls, _, _ in SKETCH_TOOLS}
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
        self.coord_label.setMinimumWidth(320)
        self.coord_label.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        self.statusBar().addWidget(self.hint_label, 1)
        self.statusBar().addPermanentWidget(self.coord_label)

        self._make_actions()
        self._make_menus_and_toolbars()

        self.canvas.cursorMoved.connect(self._show_cursor)
        self.canvas.typedText.connect(self.input_bar.start_typing)
        self.canvas.escapeIdle.connect(lambda: self.tool_actions["select"].trigger())
        self.canvas.toolStateChanged.connect(self._refresh_tool_text)
        self.model.undo_stack.cleanChanged.connect(self._update_title)
        self.model.changed.connect(self._update_title)

        self.tool_actions["select"].trigger()
        self.resize(1280, 820)
        if path:
            self.open_path(path)
        self._update_title()

    # --- actions ---------------------------------------------------------------
    def _action(self, text, slot, shortcut=None, icon_name=None, checkable=False):
        act = QAction(text, self)
        if icon_name:
            act.setIcon(icon(icon_name))
        if shortcut:
            act.setShortcut(QKeySequence(shortcut))
        act.setCheckable(checkable)
        if slot is not None:
            act.triggered.connect(slot)
        return act

    def _make_actions(self):
        self.act_new = self._action("&New", self.new_document, QKeySequence.New)
        self.act_open = self._action("&Open...", self.open_dialog, QKeySequence.Open)
        self.act_save = self._action("&Save", self.save, QKeySequence.Save)
        self.act_save_as = self._action("Save &As...", self.save_as, QKeySequence.SaveAs)
        self.act_import = self._action("&Import points (.dat/.txt)...", self.import_points, "Ctrl+I", "import")
        self.act_quit = self._action("&Quit", self.close, "Ctrl+Q")

        self.act_undo = self.model.undo_stack.createUndoAction(self, "&Undo")
        self.act_undo.setShortcut(QKeySequence.Undo)
        self.act_redo = self.model.undo_stack.createRedoAction(self, "&Redo")
        self.act_redo.setShortcut(QKeySequence.Redo)
        self.act_delete = self._action("&Delete", self.model.delete_selection)
        self.act_delete.setShortcuts([QKeySequence.Delete])
        self.act_select_all = self._action("Select &all", lambda: self.model.set_selection(self.model.doc.curves),
                                           QKeySequence.SelectAll)
        self.act_toggle_ref = self._action("Convert to/from &reference", lambda: self.model.toggle_reference(),
                                           "Shift+X")
        self.act_ref_mode = self._action("Draw reference (dashed) lines", self._set_reference_mode, "X",
                                         "reference", checkable=True)
        self.act_ref_mode.setToolTip("Draw reference (dashed) lines: block splitters, not geometry  [X]")
        self.act_fit = self._action("Zoom to &fit", self.canvas.zoom_to_fit, "F", "fit")

        self.tool_group = QActionGroup(self)
        self.tool_actions = {}
        for cls, key, tip in SKETCH_TOOLS:
            act = self._action(cls.label, None, key, cls.name, checkable=True)
            act.setToolTip(f"{tip}  [{key}]")
            act.triggered.connect(lambda checked=False, n=cls.name: self.set_tool(n))
            self.tool_group.addAction(act)
            self.tool_actions[cls.name] = act

        self.snap_actions = {}
        for kind in snapping.ALL_KINDS:
            act = self._action(kind.capitalize(), None, checkable=True)
            act.setChecked(self.canvas.snapper.enabled[kind])
            act.toggled.connect(lambda on, k=kind: self.canvas.snapper.enabled.__setitem__(k, on))
            self.snap_actions[kind] = act
        self.snap_actions[snapping.GRID].setShortcut(QKeySequence("G"))
        self.snap_actions[snapping.GRID].setText("Grid  [G]")

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
        tb.addSeparator()
        tb.addAction(self.act_import)
        tb.addAction(self.act_fit)

    # --- tools -------------------------------------------------------------------
    def set_tool(self, name):
        self.canvas.set_tool(self.tools[name])
        self.tool_actions[name].setChecked(True)
        self.canvas.setFocus()

    def _set_reference_mode(self, on):
        self.model.reference_mode = on
        self.canvas.update()

    def _refresh_tool_text(self):
        self.input_bar.refresh()
        tool = self.canvas.tool
        self.hint_label.setText(tool.hint() if tool else "")

    def _show_cursor(self, snap):
        step = self.canvas.grid_step()
        digits = max(0, -math.floor(math.log10(step)) + 1)
        x, y = snap.point
        kind = f"   [{snap.kind}]" if snap.kind else ""
        self.coord_label.setText(f"x = {x:.{digits}f}   y = {y:.{digits}f}{kind}")
        self.input_bar.refresh()

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
        dlg = ImportPointsDialog(pf, path, self)
        if dlg.exec():
            ids = self.model.add_curves(dlg.curves(), f"Import {pf.name}")
            self.model.set_selection(ids)
            self.canvas.zoom_to_fit()

    def closeEvent(self, ev):
        if self._confirm_discard():
            ev.accept()
        else:
            ev.ignore()
