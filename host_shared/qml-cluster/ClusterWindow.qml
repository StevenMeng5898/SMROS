import QtQuick 2.15
import QtQuick.Window 2.15

Window {
    id: window
    width: 1280
    height: 720
    visible: true
    color: "#071013"
    title: "SMROS Vehicle Cluster"

    InstrumentCluster {
        id: cluster
        anchors.fill: parent
        focus: true
    }
}
