// mango-sddm — the login screen for the mango desktop.
//
// Colours come from Colors.qml, which matugen regenerates on every wallpaper
// switch; this file only ever refers to Material 3 role names, never to a
// literal colour. Everything is in this one file on purpose: a flat theme
// directory needs no qmldir and no QML2_IMPORT_PATH, which is one whole class
// of greeter startup failure that cannot happen.
//
// Test without installing:  sddm-greeter-qt6 --test-mode --theme <this dir>

import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts

Item {
	id: root

	readonly property var geometry: screenModel.geometry(screenModel.primary)
	readonly property string fontFamily: config.Font || "JetBrainsMono Nerd Font"
	readonly property real dim: parseFloat(config.Dim) || 0

	x: geometry.x
	y: geometry.y
	width: geometry.width
	height: geometry.height

	Colors {
		id: colors
	}

	// ---------------------------------------------------------------- shared

	component ThemedCombo: ComboBox {
		id: combo

		implicitHeight: 40
		font.family: root.fontFamily
		font.pixelSize: 14
		textRole: "name"

		contentItem: Text {
			leftPadding: 12
			rightPadding: 30
			text: combo.displayText
			color: colors.on_surface
			font: combo.font
			verticalAlignment: Text.AlignVCenter
			elide: Text.ElideRight
		}

		indicator: Text {
			x: combo.width - width - 12
			y: (combo.height - height) / 2
			text: "" // nf-fa-angle_down
			font.family: root.fontFamily
			font.pixelSize: 14
			color: colors.on_surface_variant
		}

		background: Rectangle {
			radius: 10
			color: combo.hovered ? colors.surface_container_highest : colors.surface_container_high
			border.width: 1
			border.color: combo.activeFocus ? colors.primary : colors.outline_variant

			Behavior on color {
				ColorAnimation { duration: 150 }
			}
			Behavior on border.color {
				ColorAnimation { duration: 150 }
			}
		}

		delegate: ItemDelegate {
			id: entry

			required property int index
			required property var model

			width: combo.width - 8
			height: 34
			highlighted: combo.highlightedIndex === entry.index

			contentItem: Text {
				leftPadding: 8
				text: entry.model.name
				color: entry.highlighted ? colors.on_primary : colors.on_surface
				font.family: root.fontFamily
				font.pixelSize: 14
				verticalAlignment: Text.AlignVCenter
				elide: Text.ElideRight
			}

			background: Rectangle {
				radius: 8
				color: entry.highlighted ? colors.primary : "transparent"
			}
		}

		popup: Popup {
			y: combo.height + 4
			width: combo.width
			implicitHeight: Math.min(listView.contentHeight + 8, 260)
			padding: 4

			background: Rectangle {
				radius: 10
				color: colors.surface_container_high
				border.width: 1
				border.color: colors.outline_variant
			}

			contentItem: ListView {
				id: listView
				clip: true
				model: combo.popup.visible ? combo.delegateModel : null
				currentIndex: combo.highlightedIndex
				ScrollIndicator.vertical: ScrollIndicator {}
			}
		}
	}

	component IconButton: Button {
		id: button

		implicitWidth: 40
		implicitHeight: 40
		flat: true
		font.family: root.fontFamily

		contentItem: Text {
			text: button.text
			color: button.enabled ? colors.on_surface_variant : colors.outline
			font.family: root.fontFamily
			font.pixelSize: 16
			horizontalAlignment: Text.AlignHCenter
			verticalAlignment: Text.AlignVCenter
		}

		background: Rectangle {
			radius: 20
			color: button.hovered ? colors.surface_container_highest : "transparent"

			Behavior on color {
				ColorAnimation { duration: 150 }
			}
		}
	}

	// ------------------------------------------------------------ background

	Rectangle {
		anchors.fill: parent
		color: colors.background
	}

	Image {
		id: wallpaper

		anchors.fill: parent
		source: config.Background ? Qt.resolvedUrl(config.Background) : ""
		fillMode: Image.PreserveAspectCrop
		asynchronous: true
		cache: true
		visible: false // drawn through the blur below
	}

	// MultiEffect is a single GPU pass. Qt5Compat's GaussianBlur would need
	// radius*2+1 samples per pixel for the same look, which is what makes other
	// themes slow to appear.
	MultiEffect {
		anchors.fill: wallpaper
		source: wallpaper
		blurEnabled: true
		blur: 1.0
		blurMax: 48
		visible: wallpaper.status === Image.Ready
	}

	Rectangle {
		anchors.fill: parent
		color: colors.scrim
		opacity: root.dim
	}

	MouseArea {
		anchors.fill: parent
		onClicked: password.forceActiveFocus()
	}

	// ----------------------------------------------------------------- clock

	QtObject {
		id: clock
		property date now: new Date()
	}

	Timer {
		interval: 1000
		running: true
		repeat: true
		onTriggered: clock.now = new Date()
	}

	// ----------------------------------------------------------------- panel

	ColumnLayout {
		id: panel

		anchors.centerIn: parent
		spacing: 28
		opacity: 0
		Component.onCompleted: opacity = 1

		Behavior on opacity {
			NumberAnimation { duration: 250; easing.type: Easing.OutCubic }
		}

		ColumnLayout {
			Layout.alignment: Qt.AlignHCenter
			spacing: 2

			Text {
				Layout.alignment: Qt.AlignHCenter
				text: Qt.formatDateTime(clock.now, config.ClockFormat || "HH:mm")
				color: colors.on_surface
				font.family: root.fontFamily
				font.pixelSize: 76
				font.weight: Font.Light
			}

			Text {
				Layout.alignment: Qt.AlignHCenter
				text: Qt.formatDateTime(clock.now, config.DateFormat || "dddd, d MMMM")
				color: colors.on_surface_variant
				font.family: root.fontFamily
				font.pixelSize: 15
			}
		}

		Rectangle {
			id: card

			Layout.alignment: Qt.AlignHCenter
			implicitWidth: 380
			implicitHeight: cardColumn.implicitHeight + 40
			radius: 20
			color: colors.surface_container
			border.width: 1
			border.color: colors.outline_variant
			transform: Translate { id: cardShake }

			ColumnLayout {
				id: cardColumn

				anchors.centerIn: parent
				width: parent.width - 40
				spacing: 14

				ThemedCombo {
					id: user

					Layout.fillWidth: true
					model: userModel
					currentIndex: userModel.lastIndex
					// One user account means nothing to choose between.
					visible: userModel.count > 1
				}

				Text {
					Layout.alignment: Qt.AlignHCenter
					text: user.displayText
					color: colors.on_surface
					font.family: root.fontFamily
					font.pixelSize: 16
					visible: userModel.count <= 1
				}

				RowLayout {
					Layout.fillWidth: true
					spacing: 8

					TextField {
						id: password

						Layout.fillWidth: true
						implicitHeight: 40
						echoMode: TextInput.Password
						passwordCharacter: "●"
						placeholderText: qsTr("Password")
						placeholderTextColor: colors.on_surface_variant
						color: colors.on_surface
						font.family: root.fontFamily
						font.pixelSize: 14
						leftPadding: 12
						rightPadding: 12
						focus: true
						onAccepted: root.login()
						onTextChanged: message.text = ""

						background: Rectangle {
							radius: 10
							color: colors.surface_container_high
							border.width: 1
							border.color: password.activeFocus ? colors.primary : colors.outline_variant

							Behavior on border.color {
								ColorAnimation { duration: 150 }
							}
						}
					}

					IconButton {
						text: "" // nf-fa-arrow_right
						enabled: password.text.length > 0
						onClicked: root.login()
					}
				}

				Text {
					Layout.alignment: Qt.AlignHCenter
					Layout.fillWidth: true
					horizontalAlignment: Text.AlignHCenter
					wrapMode: Text.Wrap
					text: message.text || (keyboard.capsLock ? qsTr("Caps Lock is on") : "")
					color: message.text ? colors.error : colors.on_surface_variant
					font.family: root.fontFamily
					font.pixelSize: 13
					visible: text.length > 0
				}
			}
		}
	}

	QtObject {
		id: message
		property string text: ""
	}

	SequentialAnimation {
		id: shake
		loops: 2

		NumberAnimation { target: cardShake; property: "x"; to: -8; duration: 50 }
		NumberAnimation { target: cardShake; property: "x"; to: 8; duration: 50 }
		NumberAnimation { target: cardShake; property: "x"; to: 0; duration: 50 }
	}

	// ------------------------------------------------------------ bottom bar

	RowLayout {
		anchors.left: parent.left
		anchors.right: parent.right
		anchors.bottom: parent.bottom
		anchors.margins: 24
		spacing: 8

		ThemedCombo {
			Layout.preferredWidth: 220
			id: session
			model: sessionModel
			currentIndex: sessionModel.lastIndex
		}

		Item {
			Layout.fillWidth: true
		}

		IconButton {
			text: "" // nf-fa-moon_o
			enabled: sddm.canSuspend
			onClicked: sddm.suspend()
		}

		IconButton {
			text: "" // nf-fa-refresh
			enabled: sddm.canReboot
			onClicked: sddm.reboot()
		}

		IconButton {
			text: "" // nf-fa-power_off
			enabled: sddm.canPowerOff
			onClicked: sddm.powerOff()
		}
	}

	// ----------------------------------------------------------------- login

	function login() {
		if (password.text.length === 0)
			return;
		message.text = "";
		sddm.login(user.currentText, password.text, session.currentIndex);
	}

	Connections {
		target: sddm

		function onLoginFailed() {
			message.text = qsTr("Login failed");
			password.text = "";
			password.forceActiveFocus();
			shake.start();
		}

		function onLoginSucceeded() {
			panel.opacity = 0;
		}

		function onInformationMessage(info) {
			message.text = info;
		}
	}

	Component.onCompleted: password.forceActiveFocus()
}
