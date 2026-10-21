"""Boundary-condition dialogs and the right-click submenu."""
from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QIcon, QPixmap
from PySide6.QtWidgets import (QComboBox, QDialog, QDialogButtonBox, QFormLayout, QHBoxLayout, QHeaderView,
                               QLabel, QLineEdit, QMessageBox, QPushButton, QTableWidget, QTableWidgetItem,
                               QVBoxLayout)

from ..model.bcs import BC_BY_CLASS, BC_CLASSES, check_tag, tag_color


def swatch(color):
    pix = QPixmap(12, 12)
    pix.fill(QColor(color))
    return QIcon(pix)


class BCDialog(QDialog):
    """Define a new tag, or edit one (class, arguments, name)."""

    def __init__(self, doc, tag=None, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Boundary condition")
        self.doc = doc
        spec = doc.bcs.get(tag, {})
        self.cls = QComboBox()
        for cls, group, _, _, desc in BC_CLASSES:
            self.cls.addItem(f"{group}: {cls}", cls)
            self.cls.setItemData(self.cls.count() - 1, desc, Qt.ToolTipRole)
        self.tag = QLineEdit(tag or "")
        self.args = QLineEdit(spec.get("args", ""))
        self.desc = QLabel()
        self.desc.setWordWrap(True)
        self._args_edited = tag is not None
        self.args.textEdited.connect(lambda _: setattr(self, "_args_edited", True))
        self.cls.currentIndexChanged.connect(self._class_changed)
        if spec:
            self.cls.setCurrentIndex(self.cls.findData(spec["class"]))
        else:
            self.cls.setCurrentIndex(self.cls.findData("InFlowBC_Supersonic"))
            self._class_changed()
        self._update_desc()

        form = QFormLayout()
        form.addRow("Eilmer class", self.cls)
        form.addRow("Tag", self.tag)
        form.addRow("Arguments", self.args)
        note = QLabel("Arguments go inside Class:new{...} in bcs.lua, e.g. flowState=inflow. Names such as\n"
                      "'inflow' are flow states; the starter sim.lua written on export defines them.")
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self._accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(self.desc)
        layout.addWidget(note)
        layout.addWidget(buttons)
        self.setMinimumWidth(460)

    def _class_changed(self):
        cls = self.cls.currentData()
        _, _, args, default_tag, _ = BC_BY_CLASS[cls]
        if not self._args_edited:
            self.args.setText(args)
        if not self.tag.isModified() and (not self.tag.text() or self.tag.text() in
                                          {c[3] for c in BC_CLASSES}):
            self.tag.setText(self._free_tag(default_tag))
        self._update_desc()

    def _free_tag(self, base):
        tag, k = base, 2
        while tag in self.doc.bcs:
            tag, k = f"{base}{k}", k + 1
        return tag

    def _update_desc(self):
        self.desc.setText(BC_BY_CLASS[self.cls.currentData()][4])

    def _accept(self):
        try:
            check_tag(self.tag.text().strip())
        except ValueError as e:
            QMessageBox.warning(self, "Boundary condition", str(e))
            return
        self.accept()

    def values(self):
        return self.tag.text().strip(), self.cls.currentData(), self.args.text().strip()


class BCManagerDialog(QDialog):
    """All boundary-condition tags: edit, add, delete."""

    def __init__(self, model, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Boundary conditions")
        self.model = model
        self.table = QTableWidget(0, 4)
        self.table.setHorizontalHeaderLabels(["Tag", "Eilmer class", "Arguments", "Curves"])
        self.table.verticalHeader().setVisible(False)
        self.table.setSelectionBehavior(QTableWidget.SelectRows)
        self.table.setEditTriggers(QTableWidget.NoEditTriggers)
        self.table.horizontalHeader().setSectionResizeMode(2, QHeaderView.Stretch)
        self.table.doubleClicked.connect(lambda _: self.edit())
        new = QPushButton("New...")
        new.clicked.connect(self.new)
        edit = QPushButton("Edit...")
        edit.clicked.connect(self.edit)
        delete = QPushButton("Delete")
        delete.clicked.connect(self.delete)
        close = QDialogButtonBox(QDialogButtonBox.Close)
        close.rejected.connect(self.reject)
        row = QHBoxLayout()
        for b in (new, edit, delete):
            row.addWidget(b)
        row.addStretch(1)
        layout = QVBoxLayout(self)
        layout.addWidget(QLabel("Untagged solid curves are inviscid slip walls (Eilmer's default)."))
        layout.addWidget(self.table)
        layout.addLayout(row)
        layout.addWidget(close)
        self.resize(640, 320)
        self.refresh()

    def refresh(self):
        usage = self.model.bc_usage()
        tags = sorted(self.model.doc.bcs)
        self.table.setRowCount(len(tags))
        for r, tag in enumerate(tags):
            spec = self.model.doc.bcs[tag]
            item = QTableWidgetItem(swatch(tag_color(self.model.doc, tag)), tag)
            self.table.setItem(r, 0, item)
            self.table.setItem(r, 1, QTableWidgetItem(spec["class"]))
            self.table.setItem(r, 2, QTableWidgetItem(spec.get("args", "")))
            self.table.setItem(r, 3, QTableWidgetItem(str(usage.get(tag, 0))))

    def current(self):
        row = self.table.currentRow()
        return self.table.item(row, 0).text() if row >= 0 else None

    def new(self):
        dlg = BCDialog(self.model.doc, parent=self)
        if dlg.exec():
            try:
                self.model.define_bc(*dlg.values())
            except ValueError as e:
                QMessageBox.warning(self, "Boundary condition", str(e))
        self.refresh()

    def edit(self):
        tag = self.current()
        if tag is None:
            return
        dlg = BCDialog(self.model.doc, tag, self)
        if dlg.exec():
            new_tag, cls, args = dlg.values()
            try:
                self.model.define_bc(new_tag, cls, args, old_tag=tag)
            except ValueError as e:
                QMessageBox.warning(self, "Boundary condition", str(e))
        self.refresh()

    def delete(self):
        tag = self.current()
        if tag is not None:
            self.model.delete_bc(tag)
            self.refresh()


def add_bc_menu(menu, canvas, curve_ids):
    """'Boundary condition' submenu applying to the solid curves among curve_ids."""
    model = canvas.model
    solid = [i for i in curve_ids if i in model.doc.curves and model.doc.curves[i].kind == "geometry"]
    sub = menu.addMenu("Boundary condition")
    if not solid:
        sub.setEnabled(False)
        return sub
    current = {model.doc.curves[i].bc_tag for i in solid}
    act = sub.addAction("Slip wall (default, untagged)", lambda: model.set_bc(solid, ""))
    act.setCheckable(True)
    act.setChecked(current == {""})
    if model.doc.bcs:
        sub.addSeparator()
    for tag in sorted(model.doc.bcs):
        act = sub.addAction(swatch(tag_color(model.doc, tag)), f"{tag}  ({model.doc.bcs[tag]['class']})",
                            lambda t=tag: model.set_bc(solid, t))
        act.setCheckable(True)
        act.setChecked(current == {tag})
    sub.addSeparator()
    sub.addAction("New boundary condition...", lambda: canvas.bcRequested.emit(solid))
    return sub
