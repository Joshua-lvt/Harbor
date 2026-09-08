import QtQuick
import QtQuick.Controls

Item {
    id: root

    property string name: ""
    property color color: Theme.iconPrimary
    property real strokeWidth: 1.8
    property bool decorative: true
    property string accessibleName: ""
    readonly property bool validName: ["activity", "app", "attach", "chat", "check", "check-circle", "chevron-down", "chevron-left", "chevron-right", "chevron-up", "clock", "close", "error", "file", "game", "info", "laptop", "lock", "maximize", "menu", "mic", "mic-off", "minus", "monitor", "network", "offline", "online", "phone", "plus", "refresh", "restore", "search", "send", "settings", "tablet", "user", "volume"].indexOf(name) >= 0

    implicitWidth: 24
    implicitHeight: 24
    Accessible.role: Accessible.Graphic
    Accessible.name: accessibleName
    Accessible.ignored: decorative
    onNameChanged: canvas.requestPaint()
    onColorChanged: canvas.requestPaint()
    onStrokeWidthChanged: canvas.requestPaint()
    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()

    Canvas {
        id: canvas

        function line(ctx, x1, y1, x2, y2) {
            ctx.beginPath();
            ctx.moveTo(x1, y1);
            ctx.lineTo(x2, y2);
            ctx.stroke();
        }

        function circle(ctx, x, y, radius) {
            ctx.beginPath();
            ctx.arc(x, y, radius, 0, Math.PI * 2);
            ctx.stroke();
        }

        anchors.fill: parent
        antialiasing: true
        onPaint: {
            const ctx = getContext("2d");
            ctx.reset();
            ctx.clearRect(0, 0, width, height);
            if (!root.validName || width <= 0 || height <= 0)
                return ;

            ctx.save();
            ctx.scale(width / 24, height / 24);
            ctx.strokeStyle = root.color;
            ctx.fillStyle = root.color;
            ctx.lineWidth = root.strokeWidth;
            ctx.lineCap = "round";
            ctx.lineJoin = "round";
            switch (root.name) {
            case "check":
                ctx.beginPath();
                ctx.moveTo(5, 12.5);
                ctx.lineTo(10, 17);
                ctx.lineTo(19, 7);
                ctx.stroke();
                break;
            case "check-circle":
                circle(ctx, 12, 12, 9);
                ctx.beginPath();
                ctx.moveTo(7.5, 12);
                ctx.lineTo(10.5, 15);
                ctx.lineTo(16.8, 8.5);
                ctx.stroke();
                break;
            case "chevron-down":
                ctx.beginPath();
                ctx.moveTo(5, 9);
                ctx.lineTo(12, 16);
                ctx.lineTo(19, 9);
                ctx.stroke();
                break;
            case "chevron-up":
                ctx.beginPath();
                ctx.moveTo(5, 15);
                ctx.lineTo(12, 8);
                ctx.lineTo(19, 15);
                ctx.stroke();
                break;
            case "chevron-left":
                ctx.beginPath();
                ctx.moveTo(15, 5);
                ctx.lineTo(8, 12);
                ctx.lineTo(15, 19);
                ctx.stroke();
                break;
            case "chevron-right":
                ctx.beginPath();
                ctx.moveTo(9, 5);
                ctx.lineTo(16, 12);
                ctx.lineTo(9, 19);
                ctx.stroke();
                break;
            case "close":
                line(ctx, 6, 6, 18, 18);
                line(ctx, 18, 6, 6, 18);
                break;
            case "plus":
                line(ctx, 12, 5, 12, 19);
                line(ctx, 5, 12, 19, 12);
                break;
            case "minus":
                line(ctx, 5, 12, 19, 12);
                break;
            case "clock":
                circle(ctx, 12, 12, 9);
                line(ctx, 12, 7, 12, 12);
                line(ctx, 12, 12, 16, 14);
                break;
            case "info":
                circle(ctx, 12, 12, 9);
                line(ctx, 12, 11, 12, 17);
                circle(ctx, 12, 7.5, 0.5);
                break;
            case "error":
                ctx.beginPath();
                ctx.moveTo(12, 3);
                ctx.lineTo(21, 20);
                ctx.lineTo(3, 20);
                ctx.closePath();
                ctx.stroke();
                line(ctx, 12, 9, 12, 14);
                circle(ctx, 12, 17, 0.45);
                break;
            case "refresh":
                ctx.beginPath();
                ctx.arc(12, 12, 8, -0.9, 3.9);
                ctx.stroke();
                ctx.beginPath();
                ctx.moveTo(4, 7);
                ctx.lineTo(4.2, 13);
                ctx.lineTo(9.5, 10);
                ctx.stroke();
                break;
            case "search":
                circle(ctx, 10.5, 10.5, 6.5);
                line(ctx, 15.5, 15.5, 20, 20);
                break;
            case "online":
                circle(ctx, 12, 12, 8);
                ctx.beginPath();
                ctx.arc(12, 12, 3, 0, Math.PI * 2);
                ctx.fill();
                break;
            case "offline":
                circle(ctx, 12, 12, 8);
                line(ctx, 5, 5, 19, 19);
                break;
            case "mic":
            case "mic-off":
                ctx.beginPath();
                ctx.roundedRect(9, 3, 6, 11, 3, 3);
                ctx.stroke();
                ctx.beginPath();
                ctx.arc(12, 11, 8, 0.2, Math.PI - 0.2);
                ctx.stroke();
                line(ctx, 12, 19, 12, 22);
                line(ctx, 8, 22, 16, 22);
                if (root.name === "mic-off")
                    line(ctx, 4, 4, 20, 20);

                break;
            case "phone":
                ctx.beginPath();
                ctx.moveTo(7, 4);
                ctx.lineTo(10, 7);
                ctx.lineTo(8.5, 9.5);
                ctx.bezierCurveTo(10.3, 13.2, 12.8, 15.7, 16.5, 17.5);
                ctx.lineTo(19, 16);
                ctx.lineTo(22, 19);
                ctx.bezierCurveTo(18.2, 23, 11, 19.5, 7.3, 15.7);
                ctx.bezierCurveTo(3.5, 12, 0, 5.8, 4, 2);
                ctx.closePath();
                ctx.stroke();
                break;
            case "lock":
                ctx.strokeRect(5, 10, 14, 11);
                ctx.beginPath();
                ctx.arc(12, 10, 5, Math.PI, 0);
                ctx.stroke();
                line(ctx, 12, 14, 12, 17);
                break;
            case "app":
                ctx.strokeRect(4, 4, 6, 6);
                ctx.strokeRect(14, 4, 6, 6);
                ctx.strokeRect(4, 14, 6, 6);
                ctx.strokeRect(14, 14, 6, 6);
                break;
            case "chat":
                ctx.beginPath();
                ctx.roundedRect(3, 4, 18, 13, 4, 4);
                ctx.stroke();
                ctx.beginPath();
                ctx.moveTo(7, 17);
                ctx.lineTo(7, 21);
                ctx.lineTo(12, 17);
                ctx.closePath();
                ctx.fill();
                break;
            case "send":
                ctx.beginPath();
                ctx.moveTo(3, 11.5);
                ctx.lineTo(21, 3.5);
                ctx.lineTo(14, 21);
                ctx.lineTo(11, 13.5);
                ctx.closePath();
                ctx.stroke();
                line(ctx, 11, 13.5, 21, 3.5);
                break;
            case "attach":
                ctx.beginPath();
                ctx.moveTo(16.5, 6.5);
                ctx.lineTo(8.5, 14.5);
                ctx.arcTo(6, 17, 8.5, 19.5, 2.1);
                ctx.arcTo(11, 17, 8.5, 14.5, 2.1);
                ctx.lineTo(17.5, 8);
                ctx.arcTo(20.5, 5, 17.5, 2, 2.1);
                ctx.arcTo(14.5, 5, 16.5, 6.5, 2.1);
                ctx.lineTo(7, 16);
                ctx.stroke();
                break;
            case "file":
                ctx.beginPath();
                ctx.moveTo(6, 3);
                ctx.lineTo(14, 3);
                ctx.lineTo(18, 7);
                ctx.lineTo(18, 21);
                ctx.lineTo(6, 21);
                ctx.closePath();
                ctx.stroke();
                ctx.beginPath();
                ctx.moveTo(14, 3);
                ctx.lineTo(14, 7);
                ctx.lineTo(18, 7);
                ctx.stroke();
                break;
            case "game":
                ctx.beginPath();
                ctx.moveTo(8, 8);
                ctx.bezierCurveTo(5, 8, 3, 11, 3, 16);
                ctx.bezierCurveTo(3, 20, 6, 21, 8, 18);
                ctx.lineTo(10, 16);
                ctx.lineTo(14, 16);
                ctx.lineTo(16, 18);
                ctx.bezierCurveTo(18, 21, 21, 20, 21, 16);
                ctx.bezierCurveTo(21, 11, 19, 8, 16, 8);
                ctx.closePath();
                ctx.stroke();
                line(ctx, 7, 11, 7, 15);
                line(ctx, 5, 13, 9, 13);
                circle(ctx, 16, 12, 0.7);
                circle(ctx, 18, 14, 0.7);
                break;
            case "network":
                circle(ctx, 5, 12, 2.5);
                circle(ctx, 18, 6, 2.5);
                circle(ctx, 18, 18, 2.5);
                line(ctx, 7.3, 10.9, 15.7, 7.1);
                line(ctx, 7.3, 13.1, 15.7, 16.9);
                break;
            case "activity":
                ctx.beginPath();
                ctx.moveTo(3, 13);
                ctx.lineTo(7, 13);
                ctx.lineTo(10, 6);
                ctx.lineTo(14, 18);
                ctx.lineTo(17, 11);
                ctx.lineTo(21, 11);
                ctx.stroke();
                break;
            case "settings":
                circle(ctx, 12, 12, 4);
                for (let index = 0; index < 8; ++index) {
                    const angle = index * Math.PI / 4;
                    line(ctx, 12 + Math.cos(angle) * 6, 12 + Math.sin(angle) * 6, 12 + Math.cos(angle) * 9, 12 + Math.sin(angle) * 9);
                }
                break;
            case "user":
                circle(ctx, 12, 8, 4);
                ctx.beginPath();
                ctx.arc(12, 21, 8, Math.PI, 0);
                ctx.stroke();
                break;
            case "menu":
                line(ctx, 4, 7, 20, 7);
                line(ctx, 4, 12, 20, 12);
                line(ctx, 4, 17, 20, 17);
                break;
            case "monitor":
                ctx.strokeRect(3, 4.5, 18, 12);
                line(ctx, 12, 16.5, 12, 19);
                line(ctx, 8, 19.5, 16, 19.5);
                break;
            case "laptop":
                ctx.strokeRect(5, 5, 14, 9);
                ctx.beginPath();
                ctx.moveTo(3, 17.5);
                ctx.lineTo(21, 17.5);
                ctx.stroke();
                break;
            case "tablet":
                ctx.strokeRect(5.5, 3.5, 13, 17);
                line(ctx, 10.5, 17.5, 13.5, 17.5);
                break;
            case "volume":
                ctx.beginPath();
                ctx.moveTo(4, 9.5);
                ctx.lineTo(7.5, 9.5);
                ctx.lineTo(12, 5.5);
                ctx.lineTo(12, 18.5);
                ctx.lineTo(7.5, 14.5);
                ctx.lineTo(4, 14.5);
                ctx.closePath();
                ctx.stroke();
                ctx.beginPath();
                ctx.arc(12, 12, 8, -0.7, 0.7);
                ctx.stroke();
                ctx.beginPath();
                ctx.arc(12, 12, 8, Math.PI - 0.7, Math.PI + 0.7);
                ctx.stroke();
                break;
            case "maximize":
                ctx.strokeRect(5.5, 5.5, 13, 13);
                break;
            case "restore":
                ctx.strokeRect(4.5, 8.5, 9, 9);
                ctx.beginPath();
                ctx.moveTo(8.5, 8.5);
                ctx.lineTo(8.5, 4.5);
                ctx.lineTo(19.5, 4.5);
                ctx.lineTo(19.5, 15.5);
                ctx.lineTo(15.5, 15.5);
                ctx.stroke();
                break;
            }
            ctx.restore();
        }
    }

}
