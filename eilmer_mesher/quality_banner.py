"""The mesh-quality warning banner above the canvas, and the thresholds dialog."""
from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (QDialog, QDialogButtonBox, QDoubleSpinBox, QFormLayout, QFrame, QHBoxLayout,
                               QLabel, QPushButton, QToolButton, QVBoxLayout)

STYLES = {   # level -> (background, border)
    "warn": ("#fff6cc", "#c9a227"),
    "bad": ("#fdebd3", "#e67e22"),
    "error": ("#fde2e1", "#d43f3a"),
}
TITLES = {"warn": "Mesh quality", "bad": "Mesh needs refinement", "error": "Mesh has inverted cells"}


class QualityBanner(QFrame):
    """
    Shown when the mesh has poor cells: what is wrong, and [Refine] (Laplace smoothing of
    the flagged blocks), [Try AOPatch] (for blocks smoothing has not fixed), [Show cells]
    (the heat-map). Dismissed, it stays hidden until the problems change.
    """
    aoRequested = Signal()

    def __init__(self, refine_action, heatmap_action, parent=None):
        super().__init__(parent)
        self.setObjectName("qualityBanner")
        self.text = QLabel()
        self.text.setWordWrap(True)
        self.refine = QToolButton()
        self.refine.setDefaultAction(refine_action)
        self.ao = QPushButton("Try AOPatch")
        self.ao.setToolTip("Switch the blocks that smoothing has not fixed to Eilmer's AOPatch")
        self.ao.clicked.connect(self.aoRequested)
        self.cells = QToolButton()
        self.cells.setDefaultAction(heatmap_action)
        for b in (self.refine, self.cells):
            b.setToolButtonStyle(Qt.ToolButtonTextBesideIcon)
        close = QToolButton()
        close.setText("x")
        close.setToolTip("Hide until the mesh quality changes")
        close.setAutoRaise(True)
        close.clicked.connect(self.dismiss)
        lay = QHBoxLayout(self)
        lay.setContentsMargins(10, 4, 6, 4)
        lay.addWidget(self.text, 1)
        for w in (self.refine, self.ao, self.cells, close):
            lay.addWidget(w)
        self._shown = None
        self._dismissed = None
        self.hide()

    def show_quality(self, quality, can_refine, ao_blocks, hint=""):
        """Update from a MeshQuality; hidden when every cell is fine."""
        level = quality.level() if quality is not None else "ok"
        if level == "ok":
            self._shown = None
            self.hide()
            return
        message = f"<b>{TITLES[level]}:</b> {quality.summary()}." + (f" {hint}" if hint else "")
        self._shown = message
        bg, border = STYLES[level]
        self.setStyleSheet(f"#qualityBanner {{ background: {bg}; border-bottom: 2px solid {border}; }}")
        self.text.setText(message)
        self.refine.setVisible(can_refine)
        self.ao.setVisible(bool(ao_blocks))
        self.setVisible(message != self._dismissed)

    def dismiss(self):
        self._dismissed = self._shown
        self.hide()


class ThresholdsDialog(QDialog):
    """Limits for the quality warnings."""

    def __init__(self, values, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Mesh quality thresholds")
        self.skew_warn = self._spin(values["skew_warn"], 0.05, 0.99, 0.05)
        self.skew_bad = self._spin(values["skew_bad"], 0.05, 1.0, 0.05)
        self.ar_warn = self._spin(values["ar_warn"], 1.5, 1e6, 5.0)
        form = QFormLayout()
        form.addRow("Warn above skewness", self.skew_warn)
        form.addRow("Needs refinement above skewness", self.skew_bad)
        form.addRow("Warn above aspect ratio", self.ar_warn)
        note = QLabel("Equiangle skewness: 0 for rectangular cells, 1 for degenerate ones. Blocks with "
                      "boundary-layer spacing\nare long and thin by design, so their aspect ratio is not checked.")
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        lay = QVBoxLayout(self)
        lay.addLayout(form)
        lay.addWidget(note)
        lay.addWidget(buttons)

    @staticmethod
    def _spin(value, lo, hi, step):
        s = QDoubleSpinBox()
        s.setDecimals(2)
        s.setRange(lo, hi)
        s.setSingleStep(step)
        s.setValue(value)
        return s

    def values(self):
        warn, bad = self.skew_warn.value(), self.skew_bad.value()
        return {"skew_warn": min(warn, bad), "skew_bad": max(warn, bad), "ar_warn": self.ar_warn.value()}
