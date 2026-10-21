"""Offset settings: the distance (display units) and the side, previewed live on the canvas."""
from PySide6.QtCore import Signal
from PySide6.QtWidgets import QCheckBox, QDialog, QDialogButtonBox, QFormLayout, QLabel, QLineEdit, QVBoxLayout


class OffsetDialog(QDialog):
    """
    Emits valuesChanged(distance in metres, reverse, source to reference) whenever the inputs change to a
    usable value (and once on show, via refresh()). The caller reports problems with
    set_error(), which also disables OK.
    """
    valuesChanged = Signal(float, bool, bool)

    def __init__(self, units, distance, reverse=False, summary="", parent=None, source_to_reference=False):
        super().__init__(parent)
        self.setWindowTitle("Offset")
        self.units = units
        self.distance_edit = QLineEdit(f"{units.to_display(distance):.6g}")
        self.reverse_box = QCheckBox("Reverse direction")
        self.reverse_box.setChecked(reverse)
        self.reverse_box.setToolTip("Closed shapes grow by default (reverse shrinks them); open curves "
                                    "go to the left of their direction (the arrow shows the side).")
        self.source_box = QCheckBox("Make the source curves reference lines")
        self.source_box.setChecked(source_to_reference)
        self.source_box.setToolTip("Turns the picked curves dashed (and clears their boundary conditions) "
                                   "when the offset is made, in the same undo step.")
        self.summary = QLabel(summary)
        self.summary.setWordWrap(True)
        self.error = QLabel()
        self.error.setWordWrap(True)
        self.error.setStyleSheet("color: #c0392b")
        self.distance_edit.textEdited.connect(self.refresh)
        self.reverse_box.toggled.connect(self.refresh)
        self.source_box.toggled.connect(self.refresh)

        form = QFormLayout()
        form.addRow(f"Distance ({units.name})", self.distance_edit)
        form.addRow("", self.reverse_box)
        form.addRow("", self.source_box)
        self.buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        self.buttons.accepted.connect(self.accept)
        self.buttons.rejected.connect(self.reject)
        lay = QVBoxLayout(self)
        lay.addWidget(self.summary)
        lay.addLayout(form)
        lay.addWidget(self.error)
        lay.addWidget(self.buttons)

    def distance(self):
        """The distance in metres; ValueError if it is not a positive number."""
        try:
            value = float(self.distance_edit.text())
        except ValueError:
            raise ValueError("the distance must be a number")
        if not value > 0:
            raise ValueError("the distance must be positive")
        return self.units.from_display(value)

    def reverse(self):
        return self.reverse_box.isChecked()

    def source_to_reference(self):
        return self.source_box.isChecked()

    def set_source_to_reference(self, on):
        self.source_box.setChecked(on)

    def set_distance(self, metres):
        self.distance_edit.setText(f"{self.units.to_display(metres):.6g}")
        self.refresh()

    def set_reverse(self, on):
        self.reverse_box.setChecked(on)        # toggled -> refresh

    def set_error(self, text):
        self.error.setText(text)
        self.buttons.button(QDialogButtonBox.Ok).setEnabled(not text)

    def refresh(self, *_):
        try:
            d = self.distance()
        except ValueError as e:
            self.set_error(f"{str(e)[0].upper()}{str(e)[1:]}.")
            return
        self.set_error("")
        self.valuesChanged.emit(d, self.reverse(), self.source_to_reference())

    def accept(self):
        if self.buttons.button(QDialogButtonBox.Ok).isEnabled():
            super().accept()
