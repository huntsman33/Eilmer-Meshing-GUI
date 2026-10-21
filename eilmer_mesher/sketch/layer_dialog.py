"""Boundary-layer settings, with a y+ estimate for the first cell height."""
from PySide6.QtWidgets import (QCheckBox, QDialog, QDialogButtonBox, QDoubleSpinBox, QFormLayout, QGroupBox,
                               QHBoxLayout, QLabel, QLineEdit, QMessageBox, QPushButton, QSpinBox, QVBoxLayout)

from ..model.layers import first_cell_height, thickness


def _number(edit, name):
    try:
        value = float(edit.text())
    except ValueError:
        raise ValueError(f"{name} must be a number")
    if not value > 0:
        raise ValueError(f"{name} must be positive")
    return value


class LayerDialog(QDialog):
    """
    values: {"first" (metres), "growth", "cells", "transition"}; flow: the y+ inputs
    {"yplus", "velocity", "density", "viscosity", "length" (metres)}.
    """

    def __init__(self, units, values, flow, existing=False, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Boundary layer" if existing else "Boundary layer: settings for new layers")
        self.units = units
        self.removed = False
        u = units.name

        self.first = QLineEdit(f"{units.to_display(values['first']):.6g}")
        self.growth = QDoubleSpinBox()
        self.growth.setDecimals(3)
        self.growth.setRange(1.0, 3.0)
        self.growth.setSingleStep(0.05)
        self.growth.setValue(values["growth"])
        self.cells = QSpinBox()
        self.cells.setRange(1, 500)
        self.cells.setValue(values["cells"])
        self.transition = QCheckBox("Grade the outer blocks from the last layer cell")
        self.transition.setChecked(values["transition"])
        self.transition.setToolTip(
            "Off: the blocks outside the layer keep their original cells.\n"
            "On: they grow from the last layer cell (at the layer's ratio, at least 1.1) up to their\n"
            "original cell size, then stay uniform, adding cells as needed. When the last layer cell is\n"
            "much smaller than the outer cells, this growth fills most of the outer block.")
        self.summary = QLabel()
        for w in (self.first,):
            w.textEdited.connect(self._update)
        self.growth.valueChanged.connect(self._update)
        self.cells.valueChanged.connect(self._update)

        form = QFormLayout()
        form.addRow(f"First cell height ({u})", self.first)
        form.addRow("Growth ratio (1 = uniform)", self.growth)
        form.addRow("Cells across the layer", self.cells)
        form.addRow("", self.transition)
        form.addRow("", self.summary)

        # y+ estimate
        self.yplus = QLineEdit(f"{flow['yplus']:g}")
        self.velocity = QLineEdit(f"{flow['velocity']:g}")
        self.density = QLineEdit(f"{flow['density']:g}")
        self.viscosity = QLineEdit(f"{flow['viscosity']:g}")
        self.length = QLineEdit(f"{units.to_display(flow['length']):g}")
        self.estimate = QLabel()
        self.estimate.setWordWrap(True)
        use = QPushButton("Use this first cell height")
        use.clicked.connect(self._use_estimate)
        for w in (self.yplus, self.velocity, self.density, self.viscosity, self.length):
            w.textEdited.connect(self._update_estimate)
        flow_form = QFormLayout()
        flow_form.addRow("Target y+", self.yplus)
        flow_form.addRow("Free-stream velocity (m/s)", self.velocity)
        flow_form.addRow("Density (kg/m^3)", self.density)
        flow_form.addRow("Dynamic viscosity (Pa s)", self.viscosity)
        flow_form.addRow(f"Distance from the leading edge ({u})", self.length)
        flow_form.addRow(self.estimate)
        flow_form.addRow("", use)
        group = QGroupBox("First cell from y+  (turbulent flat plate, Cf = 0.026 Re_x^-1/7)")
        group.setLayout(flow_form)

        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self._accept)
        buttons.rejected.connect(self.reject)
        row = QHBoxLayout()
        if existing:
            remove = QPushButton("Remove layer")
            remove.clicked.connect(self._remove)
            row.addWidget(remove)
        row.addStretch(1)
        row.addWidget(buttons)
        note = QLabel("The first cell height is the wall-normal size of the cell on the wall; Eilmer's cell "
                      "centre sits at half of it.\nChanging an applied layer cuts its blocks again (their "
                      "counts and clustering are set afresh).")
        note.setWordWrap(True)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(group)
        layout.addWidget(note)
        layout.addLayout(row)
        self.setMinimumWidth(480)
        self._update()
        self._update_estimate()

    # --- live text -------------------------------------------------------------------------
    def _update(self):
        try:
            first = self.units.from_display(_number(self.first, "The first cell height"))
        except ValueError as e:
            self.summary.setText(str(e))
            return
        g, n = self.growth.value(), self.cells.value()
        self.summary.setText(f"Layer thickness {self.units.format(thickness(first, g, n))}, "
                             f"last cell {self.units.format(first * g ** (n - 1))}")

    def _estimate(self):
        return first_cell_height(_number(self.yplus, "y+"), _number(self.velocity, "The velocity"),
                                 _number(self.density, "The density"), _number(self.viscosity, "The viscosity"),
                                 self.units.from_display(_number(self.length, "The distance")))

    def _update_estimate(self):
        try:
            h = self._estimate()
            f = self.flow_values()
            re = f["density"] * f["velocity"] * f["length"] / f["viscosity"]
            self.estimate.setText(f"Re_x = {re:.3g}: first cell height {self.units.format(h)}")
        except ValueError as e:
            self.estimate.setText(str(e))

    def _use_estimate(self):
        try:
            self.first.setText(f"{self.units.to_display(self._estimate()):.4g}")
            self._update()
        except ValueError as e:
            QMessageBox.warning(self, "Boundary layer", str(e))

    # --- results ----------------------------------------------------------------------------
    def _accept(self):
        try:
            self.values()
        except ValueError as e:
            QMessageBox.warning(self, "Boundary layer", str(e))
            return
        self.accept()

    def _remove(self):
        self.removed = True
        self.accept()

    def values(self):
        return {"first": self.units.from_display(_number(self.first, "The first cell height")),
                "growth": self.growth.value(), "cells": self.cells.value(),
                "transition": self.transition.isChecked()}

    def flow_values(self):
        """The y+ inputs as typed (silently skipping any that are not valid numbers)."""
        out = {}
        for name, edit, convert in (("yplus", self.yplus, float), ("velocity", self.velocity, float),
                                    ("density", self.density, float), ("viscosity", self.viscosity, float),
                                    ("length", self.length, lambda v: self.units.from_display(float(v)))):
            try:
                out[name] = convert(edit.text())
            except ValueError:
                pass
        return out
