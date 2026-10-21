"""Double-click dimension dialogs for points, lines and arcs (values in display units)."""
import math

from PySide6.QtWidgets import (QDialog, QDialogButtonBox, QDoubleSpinBox, QFormLayout, QLabel,
                               QMessageBox, QVBoxLayout)

from ..model import geom


class DimensionDialog(QDialog):
    def __init__(self, title, fields, note="", parent=None):
        """fields: [(label, value, suffix, minimum, maximum)]"""
        super().__init__(parent)
        self.setWindowTitle(title)
        form = QFormLayout()
        self.spins = []
        for label, value, suffix, lo, hi in fields:
            spin = QDoubleSpinBox()
            spin.setDecimals(9)
            spin.setRange(lo, hi)
            spin.setValue(value)
            spin.setSuffix(f" {suffix}")
            spin.setMinimumWidth(180)
            form.addRow(label, spin)
            self.spins.append(spin)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        if note:
            layout.addWidget(QLabel(note))
        layout.addWidget(buttons)
        if self.spins:
            self.spins[0].selectAll()

    def values(self):
        return [s.value() for s in self.spins]


BIG = 1.0e12


def dialog_for(model, units, eid, parent=None):
    """(dialog, apply(values)) for an entity, or None if it has no dimensions to edit."""
    doc = model.doc
    u = units.name
    if eid in doc.points:
        x, y = units.point_to_display(doc.points[eid])
        dlg = DimensionDialog("Point", [("X", x, u, -BIG, BIG), ("Y", y, u, -BIG, BIG)], parent=parent)
        return dlg, lambda v: model.set_point(eid, units.point_from_display(v))
    if eid in doc.dimensions:
        dim = doc.dimensions[eid]
        value = model.dimension_value(eid)
        if dim.kind == "distance":
            dlg = DimensionDialog("Distance dimension", [("Distance", units.to_display(value), u, 1e-12, BIG)],
                                  "The second line moves parallel to itself; the first stays.", parent)
            return dlg, lambda v: model.set_dimension_value(eid, units.from_display(v[0]))
        if dim.curve2 is not None:
            dlg = DimensionDialog("Angle dimension", [("Angle", value, "deg", 1e-9, 179.999999)],
                                  "The second line turns about the corner; the first stays.", parent)
            return dlg, lambda v: model.set_dimension_value(eid, v[0])
        if dim.kind == "length":
            dlg = DimensionDialog("Length dimension", [("Length", units.to_display(value), u, 1e-12, BIG)],
                                  "The line's start stays; its end moves.", parent)
            return dlg, lambda v: model.set_dimension_value(eid, units.from_display(v[0]))
        dlg = DimensionDialog("Angle dimension", [("Sweep angle", value, "deg", 1e-9, 359.999999)],
                              "The arc's centre and start stay; its end moves.", parent)
        return dlg, lambda v: model.set_dimension_value(eid, v[0])
    c = doc.curves.get(eid)
    if c is None:
        return None
    if c.type == "line":
        length = math.dist(c.p0, c.p1)
        angle = math.degrees(math.atan2(c.p1[1] - c.p0[1], c.p1[0] - c.p0[0]))
        dlg = DimensionDialog("Line", [("Length", units.to_display(length), u, 1e-12, BIG),
                                       ("Angle", angle, "deg", -360.0, 360.0)],
                              "The start point stays; the end moves.", parent)
        return dlg, lambda v: model.set_line(eid, units.from_display(v[0]), v[1])
    if c.type == "arc":
        _, r, _, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
        dlg = DimensionDialog("Arc", [("Radius", units.to_display(r), u, 1e-12, BIG),
                                      ("Sweep angle", math.degrees(abs(sweep)), "deg", 1e-9, 359.999999)],
                              f"Arc length {units.format(abs(r * sweep))}.\n"
                              "The centre and start direction stay; the end moves.", parent)
        return dlg, lambda v: model.set_arc(eid, units.from_display(v[0]), v[1])
    return None


def edit_entity(parent, model, units, eid):
    found = dialog_for(model, units, eid, parent)
    if found is None:
        return False
    dlg, apply = found
    if not dlg.exec():
        return False
    try:
        apply(dlg.values())
    except ValueError as e:
        QMessageBox.warning(parent, "Edit", str(e))
        return False
    return True
