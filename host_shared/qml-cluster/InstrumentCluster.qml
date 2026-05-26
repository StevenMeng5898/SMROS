import QtQuick 2.15

Item {
    id: cluster
    objectName: "smrosVehicleCluster"
    width: 1280
    height: 720
    clip: true

    property string title: "SMROS Vehicle Cluster"
    property int speedKph: 88
    property int rpm: 3200
    property int batteryPercent: 78
    property int rangeKm: 326
    property string gear: "D"
    property string driveMode: "Comfort"
    property bool leftTurn: false
    property bool rightTurn: true
    property string warning: "ADAS ready"

    property color backgroundColor: "#071013"
    property color panelColor: "#121d20"
    property color lineColor: "#364d52"
    property color textColor: "#f4fbf8"
    property color mutedColor: "#899ea3"
    property color tealColor: "#00a6a6"
    property color amberColor: "#ffb000"
    property color greenColor: "#48be7b"
    property color blueColor: "#5fa6e6"

    Rectangle {
        anchors.fill: parent
        color: cluster.backgroundColor
    }

    Canvas {
        id: clusterCanvas
        anchors.fill: parent
        antialiasing: true

        onPaint: {
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            drawGrid(ctx)
            drawPanel(ctx, 38, 26, width - 76, 92, 14)
            drawGauge(ctx, 330, 410, 188, cluster.speedKph, 240, cluster.tealColor)
            drawGauge(ctx, 950, 410, 188, cluster.rpm, 8000, cluster.amberColor)
            drawLane(ctx)
            drawBars(ctx)
        }

        function drawGrid(ctx) {
            ctx.save()
            ctx.strokeStyle = "#0a191d"
            ctx.lineWidth = 1
            for (var x = 0; x < width; x += 80) {
                ctx.beginPath()
                ctx.moveTo(x, 0)
                ctx.lineTo(x, height)
                ctx.stroke()
            }
            for (var y = 0; y < height; y += 60) {
                ctx.beginPath()
                ctx.moveTo(0, y)
                ctx.lineTo(width, y)
                ctx.stroke()
            }
            ctx.restore()
        }

        function drawPanel(ctx, x, y, w, h, radius) {
            ctx.save()
            ctx.fillStyle = cluster.panelColor
            ctx.strokeStyle = cluster.lineColor
            ctx.lineWidth = 1
            roundedRect(ctx, x, y, w, h, radius)
            ctx.fill()
            ctx.stroke()
            ctx.restore()
        }

        function roundedRect(ctx, x, y, w, h, radius) {
            ctx.beginPath()
            ctx.moveTo(x + radius, y)
            ctx.lineTo(x + w - radius, y)
            ctx.quadraticCurveTo(x + w, y, x + w, y + radius)
            ctx.lineTo(x + w, y + h - radius)
            ctx.quadraticCurveTo(x + w, y + h, x + w - radius, y + h)
            ctx.lineTo(x + radius, y + h)
            ctx.quadraticCurveTo(x, y + h, x, y + h - radius)
            ctx.lineTo(x, y + radius)
            ctx.quadraticCurveTo(x, y, x + radius, y)
            ctx.closePath()
        }

        function drawGauge(ctx, cx, cy, radius, value, maxValue, accent) {
            drawPanel(ctx, cx - 238, cy - 238, 476, 428, 18)

            var start = Math.PI * 0.78
            var sweep = Math.PI * 1.44
            var ratio = Math.max(0, Math.min(1, value / maxValue))

            ctx.save()
            ctx.lineCap = "round"
            ctx.lineWidth = 20
            ctx.strokeStyle = "#2d4045"
            ctx.beginPath()
            ctx.arc(cx, cy, radius, start, start + sweep, false)
            ctx.stroke()

            ctx.strokeStyle = accent
            ctx.beginPath()
            ctx.arc(cx, cy, radius, start, start + sweep * ratio, false)
            ctx.stroke()

            ctx.lineWidth = 2
            ctx.strokeStyle = cluster.lineColor
            for (var i = 0; i <= 8; i += 1) {
                var a = start + sweep * (i / 8)
                var x1 = cx + Math.cos(a) * (radius - 8)
                var y1 = cy + Math.sin(a) * (radius - 8)
                var x2 = cx + Math.cos(a) * (radius + 20)
                var y2 = cy + Math.sin(a) * (radius + 20)
                ctx.beginPath()
                ctx.moveTo(x1, y1)
                ctx.lineTo(x2, y2)
                ctx.stroke()
            }

            var angle = start + sweep * ratio
            ctx.lineWidth = 5
            ctx.strokeStyle = cluster.textColor
            ctx.beginPath()
            ctx.moveTo(cx, cy)
            ctx.lineTo(cx + Math.cos(angle) * (radius - 54), cy + Math.sin(angle) * (radius - 54))
            ctx.stroke()
            ctx.fillStyle = accent
            ctx.beginPath()
            ctx.arc(cx, cy, 12, 0, Math.PI * 2, false)
            ctx.fill()
            ctx.fillStyle = "#03080a"
            ctx.beginPath()
            ctx.arc(cx, cy, 5, 0, Math.PI * 2, false)
            ctx.fill()
            ctx.restore()
        }

        function drawLane(ctx) {
            drawPanel(ctx, 520, 194, 240, 336, 18)
            ctx.save()
            ctx.strokeStyle = cluster.blueColor
            ctx.lineWidth = 4
            ctx.beginPath()
            ctx.moveTo(576, 282)
            ctx.lineTo(520, 510)
            ctx.stroke()
            ctx.beginPath()
            ctx.moveTo(704, 282)
            ctx.lineTo(760, 510)
            ctx.stroke()

            ctx.fillStyle = cluster.tealColor
            roundedRect(ctx, 588, 310, 104, 162, 24)
            ctx.fill()
            ctx.fillStyle = "#03080a"
            roundedRect(ctx, 606, 330, 68, 52, 10)
            ctx.fill()
            ctx.fillStyle = "#0d2d30"
            roundedRect(ctx, 606, 404, 68, 46, 10)
            ctx.fill()
            ctx.restore()
        }

        function drawBars(ctx) {
            drawPanel(ctx, 66, 604, 504, 72, 12)
            drawPanel(ctx, 710, 604, 504, 72, 12)
            drawBar(ctx, 314, 630, 190, 20, Math.min(cluster.rangeKm, 420) / 420)
            drawBar(ctx, 1012, 630, 156, 20, cluster.batteryPercent / 100)
        }

        function drawBar(ctx, x, y, w, h, ratio) {
            ctx.save()
            ctx.fillStyle = "#03080a"
            roundedRect(ctx, x, y, w, h, 6)
            ctx.fill()
            ctx.fillStyle = cluster.greenColor
            roundedRect(ctx, x + 3, y + 3, Math.max(0, Math.min(1, ratio)) * (w - 6), h - 6, 4)
            ctx.fill()
            ctx.restore()
        }
    }

    Text {
        text: cluster.title
        x: 70
        y: 52
        color: cluster.textColor
        font.pixelSize: 34
        font.bold: true
    }

    Text {
        text: "QML component / native SMROS mirror"
        x: 70
        y: 88
        color: cluster.mutedColor
        font.pixelSize: 16
        font.letterSpacing: 0
    }

    Text {
        text: cluster.leftTurn ? "<<" : "<"
        x: 532
        y: 54
        color: cluster.leftTurn ? cluster.greenColor : cluster.lineColor
        font.pixelSize: 32
        font.bold: true
    }

    Text {
        text: cluster.gear
        anchors.horizontalCenter: parent.horizontalCenter
        y: 38
        color: cluster.amberColor
        font.pixelSize: 76
        font.bold: true
    }

    Text {
        text: cluster.rightTurn ? ">>" : ">"
        x: 708
        y: 54
        color: cluster.rightTurn ? cluster.greenColor : cluster.lineColor
        font.pixelSize: 32
        font.bold: true
    }

    Text {
        text: cluster.driveMode
        anchors.horizontalCenter: parent.horizontalCenter
        y: 102
        color: cluster.blueColor
        font.pixelSize: 24
    }

    Text {
        text: cluster.warning
        x: 960
        y: 66
        width: 230
        color: cluster.textColor
        horizontalAlignment: Text.AlignRight
        font.pixelSize: 24
        elide: Text.ElideRight
    }

    Text {
        text: cluster.speedKph
        x: 212
        y: 342
        width: 236
        color: cluster.textColor
        horizontalAlignment: Text.AlignHCenter
        font.pixelSize: 86
        font.bold: true
    }

    Text {
        text: "KM/H"
        x: 212
        y: 436
        width: 236
        color: cluster.mutedColor
        horizontalAlignment: Text.AlignHCenter
        font.pixelSize: 24
    }

    Text {
        text: cluster.rpm
        x: 832
        y: 342
        width: 236
        color: cluster.textColor
        horizontalAlignment: Text.AlignHCenter
        font.pixelSize: 86
        font.bold: true
    }

    Text {
        text: "RPM"
        x: 832
        y: 436
        width: 236
        color: cluster.mutedColor
        horizontalAlignment: Text.AlignHCenter
        font.pixelSize: 24
    }

    Text {
        text: "LANE"
        x: 520
        y: 226
        width: 240
        color: cluster.mutedColor
        horizontalAlignment: Text.AlignHCenter
        font.pixelSize: 22
    }

    Text {
        text: cluster.rangeKm + " km"
        x: 94
        y: 624
        color: cluster.greenColor
        font.pixelSize: 32
        font.bold: true
    }

    Text {
        text: cluster.batteryPercent + "%"
        x: 738
        y: 624
        color: cluster.greenColor
        font.pixelSize: 32
        font.bold: true
    }

    onSpeedKphChanged: clusterCanvas.requestPaint()
    onRpmChanged: clusterCanvas.requestPaint()
    onBatteryPercentChanged: clusterCanvas.requestPaint()
    onRangeKmChanged: clusterCanvas.requestPaint()
    Component.onCompleted: clusterCanvas.requestPaint()
}
