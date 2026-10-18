"""python -m eilmer_mesher [project.emesh.json]"""
import os
import signal
import sys

from PySide6.QtWidgets import QApplication

from .app import MainWindow


def _running_on_wsl():
    try:
        with open("/proc/version") as f:
            return "microsoft" in f.read().lower()
    except OSError:
        return False


def _choose_platform():
    """
    Prefer X11 (xcb) under WSLg. Qt's Wayland backend there mishandles popups:
    menus drop out of place and modal dialogs can fail to appear, which makes
    the app look frozen (e.g. the save prompt on Ctrl+Q).
    QT_QPA_PLATFORM set by the user always wins.
    """
    if "QT_QPA_PLATFORM" not in os.environ and os.environ.get("DISPLAY") and _running_on_wsl():
        os.environ["QT_QPA_PLATFORM"] = "xcb"


def main():
    _choose_platform()
    # Ctrl+C in the terminal ends the program instead of raising KeyboardInterrupt
    # inside whichever Qt event handler happens to be running.
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    app = QApplication(sys.argv)
    app.setApplicationName("Eilmer Mesher")
    win = MainWindow(sys.argv[1] if len(sys.argv) > 1 else None)
    win.show()
    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
