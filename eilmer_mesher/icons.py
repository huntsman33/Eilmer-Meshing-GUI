"""Toolbar icons drawn with QPainter, so the tool ships without image files."""
from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPainterPath, QPen, QPixmap, QPolygonF

INK = "#1d2733"
ACCENT = "#1a9fd9"
REF = "#9c6b30"


def _pen(color=INK, width=2.0, dashed=False):
    pen = QPen(QColor(color), width)
    pen.setCapStyle(Qt.RoundCap)
    pen.setJoinStyle(Qt.RoundJoin)
    if dashed:
        pen.setStyle(Qt.CustomDashLine)
        pen.setDashPattern([2.0, 1.6])
    return pen


def _dots(p, pts):
    p.setPen(Qt.NoPen)
    p.setBrush(QColor(ACCENT))
    for x, y in pts:
        p.drawEllipse(QPointF(x, y), 2.4, 2.4)
    p.setBrush(Qt.NoBrush)


def _draw(name, p):
    p.setPen(_pen())
    if name == "select":
        p.setBrush(QColor(INK))
        p.drawPolygon(QPolygonF([QPointF(9, 5), QPointF(9, 25), QPointF(14, 20),
                                 QPointF(18, 28), QPointF(21, 26), QPointF(17, 19), QPointF(24, 19)]))
    elif name == "line":
        p.drawLine(QPointF(6, 26), QPointF(26, 6))
        _dots(p, [(6, 26), (26, 6)])
    elif name == "profile":
        path = QPainterPath(QPointF(5, 26))
        path.lineTo(12, 10)
        path.lineTo(19, 20)
        path.quadTo(QPointF(24, 6), QPointF(28, 8))
        p.drawPath(path)
        _dots(p, [(5, 26), (12, 10), (19, 20)])
    elif name == "arc":
        p.drawArc(QRectF(6, 8, 22, 22), 0 * 16, 180 * 16)
        _dots(p, [(6, 19), (28, 19), (17, 8)])
    elif name == "circle":
        p.drawEllipse(QRectF(6, 6, 20, 20))
        _dots(p, [(16, 16)])
    elif name == "rectangle":
        p.drawRect(QRectF(5, 9, 22, 14))
    elif name == "triangle":
        p.drawPolygon(QPolygonF([QPointF(16, 5), QPointF(28, 26), QPointF(4, 26)]))
    elif name == "bezier":
        p.setPen(_pen(ACCENT, 1.0, dashed=True))
        p.drawPolyline(QPolygonF([QPointF(5, 26), QPointF(10, 5), QPointF(27, 8)]))
        p.setPen(_pen())
        path = QPainterPath(QPointF(5, 26))
        path.quadTo(QPointF(10, 5), QPointF(27, 8))
        p.drawPath(path)
        _dots(p, [(5, 26), (10, 5), (27, 8)])
    elif name == "spline":
        path = QPainterPath(QPointF(4, 22))
        path.cubicTo(QPointF(9, 6), QPointF(14, 6), QPointF(17, 16))
        path.cubicTo(QPointF(20, 26), QPointF(25, 26), QPointF(28, 10))
        p.drawPath(path)
        _dots(p, [(4, 22), (17, 16), (28, 10)])
    elif name == "point":
        p.setPen(_pen(INK, 2.4))
        p.drawLine(QPointF(16, 6), QPointF(16, 26))
        p.drawLine(QPointF(6, 16), QPointF(26, 16))
    elif name == "trim":
        p.drawLine(QPointF(4, 22), QPointF(28, 22))
        p.drawLine(QPointF(10, 6), QPointF(10, 28))
        p.drawLine(QPointF(22, 6), QPointF(22, 28))
        p.setPen(_pen("#d43f3a", 3.0))
        p.drawLine(QPointF(11.5, 22), QPointF(20.5, 22))
    elif name == "dimension":
        p.drawLine(QPointF(5, 8), QPointF(5, 24))
        p.drawLine(QPointF(27, 8), QPointF(27, 24))
        p.setPen(_pen(ACCENT, 1.6))
        p.drawLine(QPointF(6, 16), QPointF(26, 16))
        p.setBrush(QColor(ACCENT))
        p.drawPolygon(QPolygonF([QPointF(6, 16), QPointF(11, 13), QPointF(11, 19)]))
        p.drawPolygon(QPolygonF([QPointF(26, 16), QPointF(21, 13), QPointF(21, 19)]))
        p.setBrush(Qt.NoBrush)
    elif name == "move":
        p.drawLine(QPointF(16, 4), QPointF(16, 28))
        p.drawLine(QPointF(4, 16), QPointF(28, 16))
        for tip, a, b in (((16, 4), (12, 9), (20, 9)), ((16, 28), (12, 23), (20, 23)),
                          ((4, 16), (9, 12), (9, 20)), ((28, 16), (23, 12), (23, 20))):
            p.drawPolyline(QPolygonF([QPointF(*a), QPointF(*tip), QPointF(*b)]))
    elif name == "merge":
        p.drawLine(QPointF(4, 16), QPointF(13, 16))
        p.drawLine(QPointF(19, 16), QPointF(28, 16))
        p.setPen(_pen(ACCENT, 2.0))
        p.drawPolyline(QPolygonF([QPointF(11, 10), QPointF(16, 16), QPointF(11, 22)]))
        p.drawPolyline(QPolygonF([QPointF(21, 10), QPointF(16, 16), QPointF(21, 22)]))
    elif name == "mirror":
        p.setPen(_pen(REF, 1.6, dashed=True))
        p.drawLine(QPointF(16, 3), QPointF(16, 29))
        p.setPen(_pen())
        p.drawPolyline(QPolygonF([QPointF(12, 8), QPointF(5, 24), QPointF(12, 24)]))
        p.setPen(_pen(ACCENT, 2.0))
        p.drawPolyline(QPolygonF([QPointF(20, 8), QPointF(27, 24), QPointF(20, 24)]))
    elif name == "regions":
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(ACCENT).lighter(150))
        p.drawRect(QRectF(5, 7, 11, 18))
        p.setBrush(QColor("#2e9d57").lighter(150))
        p.drawRect(QRectF(16, 7, 11, 18))
        p.setBrush(Qt.NoBrush)
        p.setPen(_pen())
        p.drawRect(QRectF(5, 7, 22, 18))
    elif name == "reference":
        p.setPen(_pen(REF, 2.0, dashed=True))
        p.drawLine(QPointF(5, 27), QPointF(27, 5))
    elif name == "import":
        p.drawPolyline(QPolygonF([QPointF(6, 18), QPointF(6, 27), QPointF(26, 27), QPointF(26, 18)]))
        p.drawLine(QPointF(16, 4), QPointF(16, 20))
        p.drawPolyline(QPolygonF([QPointF(10, 14), QPointF(16, 20), QPointF(22, 14)]))
    elif name == "fit":
        for (x, y, dx, dy) in ((5, 5, 1, 1), (27, 5, -1, 1), (5, 27, 1, -1), (27, 27, -1, -1)):
            p.drawPolyline(QPolygonF([QPointF(x + 7*dx, y), QPointF(x, y), QPointF(x, y + 7*dy)]))
    elif name == "snap":
        p.drawRect(QRectF(10, 10, 12, 12))
        p.setPen(_pen(ACCENT, 1.5))
        p.drawLine(QPointF(4, 16), QPointF(28, 16))


def icon(name):
    pix = QPixmap(32, 32)
    pix.fill(Qt.transparent)
    p = QPainter(pix)
    p.setRenderHint(QPainter.Antialiasing)
    _draw(name, p)
    p.end()
    return QIcon(pix)
