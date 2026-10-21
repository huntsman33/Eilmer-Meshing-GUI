"""Dialogs for blocking: cells along an edge, and N x M splits."""
from PySide6.QtWidgets import (QCheckBox, QComboBox, QDialog, QDialogButtonBox, QDoubleSpinBox,
                               QFormLayout, QLabel, QLineEdit, QMessageBox, QSpinBox, QVBoxLayout)

from ..units import Units

# (label, kind, end0, end1): Roberts stretching toward the marked ends, or a fixed first cell
SPACING = [("Uniform", "uniform", False, False),
           ("Finer toward the start (o)", "roberts", True, False),
           ("Finer toward the end (>)", "roberts", False, True),
           ("Finer toward both ends", "roberts", True, True),
           ("First cell height at the start (o)", "growth", True, False),
           ("First cell height at the end (>)", "growth", False, True)]


class EdgeCellsDialog(QDialog):
    def __init__(self, cells, spec=None, parent=None, units=None):
        super().__init__(parent)
        self.setWindowTitle("Edge cells")
        self.units = units or Units()
        self.cells = QSpinBox()
        self.cells.setRange(1, 100000)
        self.cells.setValue(cells)
        self.spacing = QComboBox()
        for label, *_ in SPACING:
            self.spacing.addItem(label)
        self.beta = QDoubleSpinBox()
        self.beta.setDecimals(5)
        self.beta.setRange(1.00001, 10.0)
        self.beta.setSingleStep(0.01)
        self.beta.setValue(1.05)
        self.first = QLineEdit("")
        if spec and spec.get("type") == "RobertsFunction":
            ends = (spec.get("end0", False), spec.get("end1", False))
            for k, (_, kind, e0, e1) in enumerate(SPACING):
                if kind == "roberts" and (e0, e1) == ends:
                    self.spacing.setCurrentIndex(k)
            self.beta.setValue(spec.get("beta", 1.05))
        elif spec and spec.get("type") in ("Growth", "Grade"):
            self.spacing.setCurrentIndex(5 if spec.get("reverse") else 4)
            self.first.setText(f"{self.units.to_display(spec['first']):.6g}")
        # A boundary layer's outer edge grows only up to a size, then stays uniform: keep that.
        self._grade = spec.get("growth") if spec and spec.get("type") == "Grade" else None
        self.opposite = QCheckBox("Also the opposite edge of this block")
        self.opposite.setChecked(True)
        self.spacing.currentIndexChanged.connect(self._enable)
        self._enable()

        form = QFormLayout()
        form.addRow("Cells", self.cells)
        form.addRow("Spacing", self.spacing)
        form.addRow("Clustering strength (beta)", self.beta)
        form.addRow(f"First cell height ({self.units.name})", self.first)
        note = QLabel("The edge's start (o) and end (>) are marked on the canvas while you hover it.\n"
                      "Beta closer to 1 clusters more strongly (Roberts stretching). A first cell height\n"
                      "grows the cells geometrically from that end (as in a boundary layer).\n"
                      "The cell count also applies to every edge linked to this one, so shared faces match.")
        note.setWordWrap(True)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self._accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(self.opposite)
        layout.addWidget(note)
        layout.addWidget(buttons)

    def _enable(self):
        kind = SPACING[self.spacing.currentIndex()][1]
        self.beta.setEnabled(kind == "roberts")
        self.first.setEnabled(kind == "growth")

    def _accept(self):
        try:
            self.values()
        except ValueError as e:
            QMessageBox.warning(self, "Edge cells", str(e))
            return
        self.accept()

    def values(self):
        """(cells, cluster spec or None, apply to opposite edge)."""
        _, kind, end0, end1 = SPACING[self.spacing.currentIndex()]
        spec = None
        if kind == "roberts":
            spec = {"type": "RobertsFunction", "end0": end0, "end1": end1, "beta": self.beta.value()}
        elif kind == "growth":
            try:
                first = self.units.from_display(float(self.first.text()))
            except ValueError:
                raise ValueError("the first cell height must be a number")
            if not first > 0:
                raise ValueError("the first cell height must be positive")
            spec = {"type": "Growth", "first": first, "reverse": end1}
            if self._grade:
                spec = {"type": "Grade", "first": first, "growth": self._grade, "reverse": end1}
        return self.cells.value(), spec, self.opposite.isChecked()


class SplitDialog(QDialog):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Split into N x M blocks")
        self.n = QSpinBox()
        self.n.setRange(1, 50)
        self.n.setValue(2)
        self.m = QSpinBox()
        self.m.setRange(1, 50)
        self.m.setValue(2)
        form = QFormLayout()
        form.addRow("Blocks along i (south edge)", self.n)
        form.addRow("Blocks along j (west edge)", self.m)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(buttons)

    def values(self):
        return self.n.value(), self.m.value()


class AOPatchDialog(QDialog):
    """Eilmer AOPatch: an area-orthogonality patch built on an nx x ny background mesh."""

    def __init__(self, nx=10, ny=10, parent=None):
        super().__init__(parent)
        self.setWindowTitle("AOPatch")
        self.nx = QSpinBox()
        self.nx.setRange(2, 200)
        self.nx.setValue(nx)
        self.ny = QSpinBox()
        self.ny.setRange(2, 200)
        self.ny.setValue(ny)
        form = QFormLayout()
        form.addRow("Background mesh nx", self.nx)
        form.addRow("Background mesh ny", self.ny)
        note = QLabel("AOPatch smooths the interior towards area-orthogonal cells; it can untangle blocks\n"
                      "with strongly curved sides. The preview shows a CoonsPatch until Eilmer's mesh arrives.")
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(note)
        layout.addWidget(buttons)

    def values(self):
        return {"nx": self.nx.value(), "ny": self.ny.value()}
