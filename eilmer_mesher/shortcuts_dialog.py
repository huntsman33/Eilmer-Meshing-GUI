"""Keyboard shortcut settings dialog."""
from PySide6.QtCore import Qt
from PySide6.QtGui import QKeySequence
from PySide6.QtWidgets import (QDialog, QDialogButtonBox, QHeaderView, QKeySequenceEdit, QMessageBox,
                               QPushButton, QTableWidget, QTableWidgetItem, QVBoxLayout)

from .settings import DEFAULT_HOLD_KEYS, DEFAULT_SHORTCUTS


class ShortcutsDialog(QDialog):
    def __init__(self, current, parent=None):
        """current: id -> shortcut text (portable format)."""
        super().__init__(parent)
        self.setWindowTitle("Keyboard shortcuts")
        self.resize(460, 640)
        self.rows = {**DEFAULT_SHORTCUTS, **DEFAULT_HOLD_KEYS}
        self.table = QTableWidget(len(self.rows), 2)
        self.table.setHorizontalHeaderLabels(["Action", "Shortcut"])
        self.table.verticalHeader().setVisible(False)
        self.table.horizontalHeader().setSectionResizeMode(0, QHeaderView.Stretch)
        self.table.horizontalHeader().setSectionResizeMode(1, QHeaderView.Stretch)
        self.editors = {}
        for row, (key, (label, _)) in enumerate(self.rows.items()):
            item = QTableWidgetItem(label)
            item.setFlags(Qt.ItemIsEnabled)
            self.table.setItem(row, 0, item)
            edit = QKeySequenceEdit(QKeySequence(current.get(key, "")))
            if key in DEFAULT_HOLD_KEYS:
                edit.setMaximumSequenceLength(1)
            self.table.setCellWidget(row, 1, edit)
            self.editors[key] = edit

        reset = QPushButton("Restore defaults")
        reset.clicked.connect(self.restore_defaults)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self._accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addWidget(self.table)
        layout.addWidget(reset)
        layout.addWidget(buttons)

    def restore_defaults(self):
        for key, (_, default) in self.rows.items():
            self.editors[key].setKeySequence(QKeySequence(default))

    def values(self):
        return {key: edit.keySequence().toString(QKeySequence.PortableText)
                for key, edit in self.editors.items()}

    def _accept(self):
        seen = {}
        for key, seq in self.values().items():
            if not seq:
                continue
            if seq in seen:
                QMessageBox.warning(self, "Keyboard shortcuts",
                                    f"'{seq}' is used by both '{self.rows[seen[seq]][0]}' "
                                    f"and '{self.rows[key][0]}'.")
                return
            seen[seq] = key
        self.accept()
