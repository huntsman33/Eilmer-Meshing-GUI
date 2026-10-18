"""Dialog for importing a point file (.dat/.txt) as splines."""
from PySide6.QtWidgets import (QCheckBox, QDialog, QDialogButtonBox, QDoubleSpinBox, QFormLayout,
                               QLabel, QVBoxLayout)

from ..model.entities import GEOMETRY, REFERENCE
from .importers import build_curves, leading_edge_index


def _spin(value, lo, hi, decimals=6, step=0.1):
    s = QDoubleSpinBox()
    s.setDecimals(decimals)
    s.setRange(lo, hi)
    s.setSingleStep(step)
    s.setValue(value)
    return s


class ImportPointsDialog(QDialog):
    def __init__(self, point_file, path, parent=None):
        super().__init__(parent)
        self.pf = point_file
        self.setWindowTitle("Import points")
        pts = point_file.points
        xs = [p[0] for p in pts]
        ys = [p[1] for p in pts]
        info = QLabel(f"<b>{point_file.name}</b><br>{path}<br>"
                      f"{len(pts)} points, {point_file.fmt} format<br>"
                      f"x: {min(xs):.6g} to {max(xs):.6g}, y: {min(ys):.6g} to {max(ys):.6g}")
        info.setWordWrap(True)

        self.scale = _spin(1.0, 1e-9, 1e9)
        self.angle = _spin(0.0, -180.0, 180.0, decimals=3, step=1.0)
        self.dx = _spin(0.0, -1e9, 1e9)
        self.dy = _spin(0.0, -1e9, 1e9)
        k_le = leading_edge_index(pts)
        self.split_le = QCheckBox("Split at leading edge (min x) so it can be a block corner")
        self.split_le.setChecked(0 < k_le < len(pts) - 1)
        self.close_te = QCheckBox("Close the trailing edge with a line")
        self.close_te.setChecked(True)
        self.reference = QCheckBox("Import as reference (dashed) curves")

        form = QFormLayout()
        form.addRow("Scale (chord)", self.scale)
        form.addRow("Angle of attack (deg, nose up)", self.angle)
        form.addRow("Translate x", self.dx)
        form.addRow("Translate y", self.dy)

        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)

        layout = QVBoxLayout(self)
        layout.addWidget(info)
        layout.addLayout(form)
        layout.addWidget(self.split_le)
        layout.addWidget(self.close_te)
        layout.addWidget(self.reference)
        layout.addWidget(buttons)

    def curves(self):
        return build_curves(self.pf, scale=self.scale.value(), angle_deg=self.angle.value(),
                            dx=self.dx.value(), dy=self.dy.value(),
                            split_le=self.split_le.isChecked(), close_te=self.close_te.isChecked(),
                            kind=REFERENCE if self.reference.isChecked() else GEOMETRY)
