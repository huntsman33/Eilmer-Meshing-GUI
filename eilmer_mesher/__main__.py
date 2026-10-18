"""python -m eilmer_mesher [project.emesh.json]"""
import sys

from PySide6.QtWidgets import QApplication

from .app import MainWindow


def main():
    app = QApplication(sys.argv)
    app.setApplicationName("Eilmer Mesher")
    win = MainWindow(sys.argv[1] if len(sys.argv) > 1 else None)
    win.show()
    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
