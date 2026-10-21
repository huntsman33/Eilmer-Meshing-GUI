"""Main window: sketch canvas, tool palette, typed input, menus."""
import os

from PySide6.QtCore import Qt
from PySide6.QtGui import QAction, QActionGroup, QKeySequence
from PySide6.QtWidgets import (QFileDialog, QHBoxLayout, QLabel, QLineEdit, QMainWindow, QMessageBox,
                               QToolButton, QVBoxLayout, QWidget)

from . import settings
from .icons import icon
from .mesh import quality as mesh_quality
from .mesh.live import LiveMesher
from .quality_banner import QualityBanner, ThresholdsDialog
from .model.document import BlockError
from .sketch.bc_dialogs import BCDialog, BCManagerDialog
from .sketch.block_dialogs import EdgeCellsDialog, SplitDialog
from .model.document import Document
from .settings import DEFAULT_HOLD_KEYS, DEFAULT_SHORTCUTS
from .shortcuts_dialog import ShortcutsDialog
from .sketch import snapping
from .sketch.canvas import SketchCanvas
from .sketch.edit_dialogs import edit_entity
from .sketch.import_dialog import ImportPointsDialog
from .sketch.layer_dialog import LayerDialog
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
        self.edit.setEnabled(bool(tool and tool.name not in ("select", "trim", "merge", "mirror", "offset", "block", "layer")))
        if tool is not None and tool.name == "move":
            self.edit.setPlaceholderText(f"type a move in {self.canvas.units.name}:  dx, dy    or  L<deg")
            return
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
        self.mesh_label = QLabel("No blocks")
        self.coord_label = QLabel()
        self.coord_label.setMinimumWidth(340)
        self.coord_label.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        self.statusBar().addWidget(self.hint_label, 1)
        self.statusBar().addPermanentWidget(self.mesh_label)
        self.statusBar().addPermanentWidget(self.coord_label)

        self.mesher = LiveMesher(self.model, self)
        self.mesher.meshChanged.connect(self.canvas.set_mesh)
        self.mesher.status.connect(self.mesh_label.setText)

        self._make_actions()
        self._make_menus_and_toolbars()
        self.apply_shortcuts()

        self.mesh_quality = None
        self._refine_targets, self._ao_targets = [], []
        self.banner = QualityBanner(self.act_refine, self.act_quality)
        self.banner.aoRequested.connect(self.try_aopatch)
        central.layout().insertWidget(0, self.banner)
        self.mesher.meshChanged.connect(self.update_quality)

        self.canvas.cursorMoved.connect(self._show_cursor)
        self.canvas.typedText.connect(self.input_bar.start_typing)
        self.canvas.escapeIdle.connect(lambda: self.set_tool("select"))
        self.canvas.toolStateChanged.connect(self._refresh_tool_text)
        self.canvas.editRequested.connect(self.edit_entity)
        self.canvas.message.connect(lambda text: self.statusBar().showMessage(text, 6000 if len(text) < 90 else 15000))
        self.canvas.edgeRequested.connect(self.edit_edge)
        self.canvas.splitRequested.connect(self.split_region)
        self.canvas.bcRequested.connect(self.new_bc_for)
        self.canvas.layerRequested.connect(self.edit_layer)
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
        self.act_export = A("file.export", "&Export to Eilmer...", self.export_to_eilmer, "export")
        self.act_bcs = A("block.bcs", "Boundary &conditions...", self.manage_bcs)
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
        self.act_toggle_split = A("edit.toggle_split", "Convert to/from block &split line",
                                  lambda: self.model.toggle_split())
        self.act_split_mode = A("sketch.split_mode", "Draw block split (dash-dot) lines", self._set_split_mode,
                                "split", checkable=True)
        self.act_fit = A("view.fit", "Zoom to &fit", self.canvas.zoom_to_fit, "fit")
        self.act_regions = A("view.regions", "Show &regions", self._set_show_regions, "regions", checkable=True)
        self.act_grid = A("view.grid", "Show &grid", self._set_show_grid, "grid", checkable=True)
        self.act_grid.setChecked(settings.show_grid())
        self.canvas.show_grid = self.act_grid.isChecked()
        self.act_shortcuts = A(None, "&Keyboard shortcuts...", self.edit_shortcuts)
        self.act_block_all = A("block.all", "Block &all regions", self.block_all)
        self.act_run_mesh = A("mesh.run", "&Run Eilmer now", lambda: self.mesher.run_now())
        self.act_show_blocks = A("view.blocks", "Show &blocks", self._set_show_blocks, checkable=True)
        self.act_show_blocks.setChecked(True)
        self.act_show_all_blocks = A(None, "Show all &blocks", lambda: self.canvas.set_blocks_hidden(set(self.canvas.hidden_blocks), False))
        self.act_show_mesh = A("view.mesh", "Show &mesh", self._set_show_mesh, checkable=True)
        self.act_show_mesh.setChecked(True)
        self.act_quality = A("view.quality", "Show cell &quality", self._set_show_quality, "quality", checkable=True)
        self.act_refine = A("block.refine", "&Refine flagged blocks", self.refine_mesh, "refine")
        self.act_refine.setToolTip("Laplace-smooth the blocks with poor cells")
        self.act_refine.setEnabled(False)
        self.act_thresholds = A(None, "Quality &thresholds...", self.edit_thresholds)
        self.metric_group = QActionGroup(self)
        self.metric_actions = {}
        for metric, text in (("skew", "Skewness"), ("ar", "Aspect ratio")):
            act = QAction(text, self, checkable=True)
            act.setChecked(metric == "skew")
            act.triggered.connect(lambda checked=False, m=metric: self._set_quality_metric(m))
            self.metric_group.addAction(act)
            self.metric_actions[metric] = act

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
        m.addAction(self.act_export)
        m.addSeparator()
        m.addAction(self.act_quit)

        m = mb.addMenu("&Edit")
        for a in (self.act_undo, self.act_redo):
            m.addAction(a)
        m.addSeparator()
        for a in (self.act_delete, self.act_select_all, self.act_toggle_ref, self.act_toggle_split):
            m.addAction(a)
        m.addSeparator()
        m.addAction(self.act_shortcuts)

        m = mb.addMenu("&Sketch")
        for act in self.tool_actions.values():
            m.addAction(act)
        m.addSeparator()
        m.addAction(self.act_ref_mode)
        m.addAction(self.act_split_mode)
        snap_menu = m.addMenu("Snap to")
        for act in self.snap_actions.values():
            snap_menu.addAction(act)

        m = mb.addMenu("&Block")
        m.addAction(self.tool_actions["block"])
        m.addAction(self.tool_actions["layer"])
        m.addAction(self.act_block_all)
        m.addAction(self.act_bcs)
        m.addAction(self.act_run_mesh)
        m.addSeparator()
        m.addAction(self.act_refine)

        m = mb.addMenu("&View")
        m.addAction(self.act_fit)
        m.addAction(self.act_regions)
        m.addAction(self.act_grid)
        m.addAction(self.act_show_blocks)
        m.addAction(self.act_show_all_blocks)
        m.addAction(self.act_show_mesh)
        m.addAction(self.act_quality)
        metric_menu = m.addMenu("Quality &metric")
        for act in self.metric_actions.values():
            metric_menu.addAction(act)
        m.addAction(self.act_thresholds)
        units_menu = m.addMenu("&Units")
        for act in self.unit_actions.values():
            units_menu.addAction(act)

        tb = self.addToolBar("Sketch")
        tb.setObjectName("sketch-toolbar")
        tb.setMovable(False)
        for name, act in self.tool_actions.items():
            if name == "block":
                tb.addSeparator()
            tb.addAction(act)
        tb.addSeparator()
        tb.addAction(self.act_ref_mode)
        tb.addAction(self.act_split_mode)
        snap_button = QToolButton()
        snap_button.setIcon(icon("snap"))
        snap_button.setToolTip("Snap options")
        snap_button.setPopupMode(QToolButton.InstantPopup)
        snap_button.setMenu(snap_menu)
        tb.addWidget(snap_button)
        tb.addAction(self.act_regions)
        tb.addAction(self.act_grid)
        tb.addAction(self.act_quality)
        tb.addSeparator()
        tb.addAction(self.act_import)
        tb.addAction(self.act_export)
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
        if on and self.act_split_mode.isChecked():          # one drawing mode at a time
            self.act_split_mode.setChecked(False)
            self.model.split_mode = False
        self.canvas.update()

    def _set_split_mode(self, on):
        self.model.split_mode = on
        if on and self.act_ref_mode.isChecked():
            self.act_ref_mode.setChecked(False)
            self.model.reference_mode = False
        self.canvas.update()

    def _set_show_blocks(self, on):
        self.canvas.show_blocks = on
        self.canvas.update()

    def _set_show_mesh(self, on):
        self.canvas.show_mesh = on
        self.mesher.enabled = on
        if on:
            self.mesher.schedule()
        self.canvas.update()

    # --- blocks ------------------------------------------------------------------------------
    def block_all(self):
        made, skipped = self.model.make_all_blocks()
        msg = f"Made {len(made)} block(s)."
        if skipped:
            msg += f" {len(skipped)} region(s) need sub-dividing first (holes or fewer than four corners)."
        self.statusBar().showMessage(msg, 8000)

    def edit_edge(self, bid, face):
        res = self.model.resolution()
        block = res.by_id().get(bid)
        if block is None:
            return
        cells = (block.niv if face in ("south", "north") else block.njv) - 1
        stored = next(b for b in self.model.doc.blocks if b.id == bid)
        dlg = EdgeCellsDialog(cells, stored.cluster.get(face), self, units=self.canvas.units)
        if dlg.exec():
            n, spec, opposite = dlg.values()
            self.model.set_edge_cells(bid, face, n, spec, opposite)
        self.canvas.setFocus()

    def edit_layer(self, bid, face):
        """Settings of the layer on this wall, or (no layer there) the settings for new layers."""
        lid = self.model.layer_at_face(bid, face) if bid else None
        if lid is not None:
            layer = self.model.layer(lid)
            values = {"first": layer.first, "growth": layer.growth, "cells": layer.cells,
                      "transition": layer.transition}
        else:
            values = settings.layer_defaults()
        dlg = LayerDialog(self.canvas.units, values, settings.layer_flow(), existing=lid is not None, parent=self)
        if dlg.exec():
            settings.set_layer_flow(dlg.flow_values())
            try:
                if dlg.removed:
                    self.model.remove_boundary_layer(lid)
                    self.statusBar().showMessage("Boundary layer removed.", 6000)
                elif lid is not None:
                    self.model.edit_boundary_layer(lid, **dlg.values())
                    note = self.model.layer_note(lid)
                    self.statusBar().showMessage("Boundary layer cut again with the new settings. " + note,
                                                 15000 if note else 6000)
                else:
                    settings.set_layer_defaults(dlg.values())
                    self.statusBar().showMessage("Settings saved for the next boundary layer.", 6000)
            except BlockError as e:
                QMessageBox.warning(self, "Boundary layer", f"Cannot change the boundary layer: {e}.")
        self._refresh_tool_text()
        self.canvas.setFocus()

    # --- boundary conditions and export -----------------------------------------------------
    def new_bc_for(self, curve_ids):
        dlg = BCDialog(self.model.doc, parent=self)
        if dlg.exec():
            tag, cls, args = dlg.values()
            try:
                self.model.define_bc(tag, cls, args)
                n = self.model.set_bc(curve_ids, tag)
                self.statusBar().showMessage(f"'{tag}' ({cls}) set on {n} curve(s).", 6000)
            except ValueError as e:
                QMessageBox.warning(self, "Boundary condition", str(e))
        self.canvas.setFocus()

    def manage_bcs(self):
        BCManagerDialog(self.model, self).exec()
        self.canvas.setFocus()

    def export_to_eilmer(self):
        res = self.model.resolution()
        if res.errors:
            text = "\n".join(f"{bid}: {msg}" for bid, msg in res.errors.items())
            QMessageBox.warning(self, "Export", "Some blocks need attention before exporting:\n\n" + text)
            return
        if not res.blocks:
            QMessageBox.information(self, "Export", "There are no blocks to export yet (Block tool, K).")
            return
        start = settings.last_export_dir() or (os.path.dirname(self.path) if self.path else os.getcwd())
        folder = QFileDialog.getExistingDirectory(self, "Export to Eilmer: choose the simulation folder", start,
                                                  FILE_DIALOG_OPTIONS | QFileDialog.ShowDirsOnly)
        if folder:
            self.export_to(folder, res)

    def export_to(self, folder, res=None):
        """Write the Eilmer files into folder, run lmr prep-grids, and report. Returns the LmrResult."""
        import json
        from PySide6.QtGui import QGuiApplication
        from .mesh.runner import export_grids
        res = res or self.model.resolution()
        QGuiApplication.setOverrideCursor(Qt.WaitCursor)
        try:
            result = export_grids(self.model.doc.with_blocks(res.blocks), folder, starter_files=True)
        except (BlockError, FileNotFoundError, OSError) as e:
            QGuiApplication.restoreOverrideCursor()
            QMessageBox.critical(self, "Export", f"Export failed: {e}")
            return None
        QGuiApplication.restoreOverrideCursor()
        settings.set_last_export_dir(folder)
        box = QMessageBox(self)
        box.setWindowTitle("Export to Eilmer")
        box.setDetailedText(result.output)
        if result.ok:
            with open(os.path.join(folder, "lmrsim", "grid", "grid.metadata")) as f:
                n_conn = len(json.load(f)["grid-connections"])
            warn = sum(len(w) for w in res.warnings.values())
            box.setIcon(QMessageBox.Information if not warn else QMessageBox.Warning)
            box.setText(f"Exported {len(res.blocks)} block(s) and {n_conn} connection(s) to\n{folder}\n\n"
                        f"Files: {', '.join(result.written)}"
                        + (f"\n\n{warn} block warning(s): check the blocks marked (!)." if warn else "")
                        + "\n\nNext: edit sim.lua, then\n  lmr prep-gas -i ideal-air.lua -o ideal-air.gas"
                          "\n  lmr prep-sim -j sim.lua\n  lmr run")
        else:
            box.setIcon(QMessageBox.Critical)
            box.setText(f"lmr prep-grids failed:\n{result.error_summary}")
        box.exec()
        return result

    def split_region(self, point):
        dlg = SplitDialog(self)
        if dlg.exec():
            n, m = dlg.values()
            try:
                self.model.subdivide(point, "nxm", n=n, m=m)
            except BlockError as e:
                self.statusBar().showMessage(f"Cannot split: {e}.", 8000)
        self.canvas.setFocus()

    def _set_show_regions(self, on):
        self.canvas.show_regions = on
        self.canvas.update()

    # --- mesh quality and refinement ------------------------------------------------------
    def update_quality(self, mesh):
        """Check the cells of a new mesh; update the heat-map, the banner and Refine."""
        if mesh is None:
            self.mesh_quality = None
            self.canvas.set_quality(None)
            self.banner.show_quality(None, False, [])
            self.act_refine.setEnabled(False)
            return
        res = self.model.resolution()
        q = mesh_quality.assess(mesh, settings.quality_thresholds(), mesh_quality.intentional_aspect(res.blocks))
        self.mesh_quality = q
        self.canvas.set_quality(q)
        self._refine_targets, self._ao_targets, hint = self._refine_plan(q, res)
        self.act_refine.setEnabled(bool(self._refine_targets))
        self.banner.show_quality(q, bool(self._refine_targets), self._ao_targets, hint)

    def _refine_plan(self, q, res):
        """(blocks Refine would smooth, blocks to offer AOPatch, a hint for the banner)."""
        layered = mesh_quality.intentional_aspect(res.blocks)
        stored = {b.id: b for b in self.model.doc.blocks}
        bad = [bid for bid in q.flagged("bad", "error") if bid in stored]
        targets = bad or [bid for bid, b in q.blocks.items() if b.skew_warn and bid in stored]
        refine = [bid for bid in targets
                  if not stored[bid].smoothing and stored[bid].patch == "CoonsPatch" and bid not in layered]
        ao = [bid for bid in bad if stored[bid].smoothing and stored[bid].patch == "CoonsPatch"
              and bid not in layered]
        hints = []
        if ao:
            hints.append(f"Smoothing has not fixed {', '.join(ao)}: try AOPatch, or sub-divide.")
        stuck = [bid for bid in bad if stored[bid].patch == "AOPatch"]
        if stuck:
            hints.append(f"Sub-divide {', '.join(stuck)} or move its corners.")
        if any(bid in layered for bid in targets):
            hints.append("Boundary-layer blocks are not smoothed (it would undo the wall spacing).")
        if q.level() == "warn" and not targets:
            hints.append("Change the cell counts along the long edges to bring the aspect ratio down.")
        return refine, ao, " ".join(hints)

    def refine_mesh(self):
        """Laplace-smooth the blocks the quality check flagged."""
        if not self._refine_targets:
            self.statusBar().showMessage("No blocks need refining.", 5000)
            return
        smoothed, ao, skipped = self.model.refine_blocks(self._refine_targets)
        parts = []
        if smoothed:
            parts.append(f"Laplace smoothing on {', '.join(smoothed)}")
        if ao:
            parts.append(f"AOPatch on {', '.join(ao)} (smoothing would fold cells there)")
        msg = "Refine: " + "; ".join(parts) + "; the mesh is being rebuilt." if parts else ""
        if skipped:
            msg += f" Skipped boundary-layer block(s) {', '.join(skipped)}."
        self.statusBar().showMessage(msg.strip(), 8000)

    def try_aopatch(self):
        if self._ao_targets:
            self.model.set_block_patch(self._ao_targets, "AOPatch", {"nx": 10, "ny": 10}, smoothing=None)
            self.statusBar().showMessage(f"{', '.join(self._ao_targets)} now use AOPatch.", 6000)

    def _set_show_quality(self, on):
        self.canvas.show_quality = on
        self.canvas.update()

    def _set_quality_metric(self, metric):
        self.canvas.quality_metric = metric
        if not self.act_quality.isChecked():
            self.act_quality.setChecked(True)
            self._set_show_quality(True)
        self.canvas.update()

    def edit_thresholds(self):
        dlg = ThresholdsDialog(settings.quality_thresholds(), self)
        if dlg.exec():
            settings.set_quality_thresholds(dlg.values())
            self.update_quality(self.mesher.mesh)
        self.canvas.setFocus()

    def _set_show_grid(self, on):
        self.canvas.show_grid = on
        settings.set_show_grid(on)
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
    def _set_path(self, path):
        self.path = path
        if path:
            base = os.path.basename(path)[:-len(PROJECT_SUFFIX)] if path.endswith(PROJECT_SUFFIX) else \
                os.path.splitext(os.path.basename(path))[0]
            self.mesher.workdir = os.path.join(os.path.dirname(os.path.abspath(path)), ".emesh", base)
        else:
            self.mesher.workdir = None

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
            self._set_path(None)
            self.canvas.hidden_blocks.clear()
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
        self._set_path(path)
        self.canvas.hidden_blocks.clear()
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
        self._set_path(path)
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
