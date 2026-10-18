"""Dialogs for blocking: cells along an edge, and N x M splits."""
from PySide6.QtWidgets import (QCheckBox, QComboBox, QDialog, QDialogButtonBox, QDoubleSpinBox,
                               QFormLayout, QLabel, QSpinBox, QVBoxLayout)

SPACING = [("Uniform", False, False),
           ("Finer toward the start (o)", True, False),
           ("Finer toward the end (>)", False, True),
           ("Finer toward both ends", True, True)]


class EdgeCellsDialog(QDialog):
    def __init__(self, cells, spec=None, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Edge cells")
        self.cells = QSpinBox()
        self.cells.setRange(1, 100000)
        self.cells.setValue(cells)
        self.spacing = QComboBox()
        for label, _, _ in SPACING:
            self.spacing.addItem(label)
        self.beta = QDoubleSpinBox()
        self.beta.setDecimals(5)
        self.beta.setRange(1.00001, 10.0)
        self.beta.setSingleStep(0.01)
        self.beta.setValue(1.05)
        if spec and spec.get("type") == "RobertsFunction":
            ends = (spec.get("end0", False), spec.get("end1", False))
            for k, (_, e0, e1) in enumerate(SPACING):
                if (e0, e1) == ends:
                    self.spacing.setCurrentIndex(k)
            self.beta.setValue(spec.get("beta", 1.05))
        self.opposite = QCheckBox("Also the opposite edge of this block")
        self.opposite.setChecked(True)
        self.spacing.currentIndexChanged.connect(self._enable)
        self._enable()

        form = QFormLayout()
        form.addRow("Cells", self.cells)
        form.addRow("Spacing", self.spacing)
        form.addRow("Clustering strength (beta)", self.beta)
        note = QLabel("The edge's start (o) and end (>) are marked on the canvas while you hover it.\n"
                      "Beta closer to 1 clusters more strongly (Roberts stretching).\n"
                      "The cell count also applies to every edge linked to this one, so shared faces match.")
        note.setWordWrap(True)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(self.opposite)
        layout.addWidget(note)
        layout.addWidget(buttons)

    def _enable(self):
        self.beta.setEnabled(self.spacing.currentIndex() != 0)

    def values(self):
        """(cells, cluster spec or None, apply to opposite edge)."""
        _, end0, end1 = SPACING[self.spacing.currentIndex()]
        spec = None
        if end0 or end1:
            spec = {"type": "RobertsFunction", "end0": end0, "end1": end1, "beta": self.beta.value()}
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
