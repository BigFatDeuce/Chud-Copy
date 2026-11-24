#!/bin/sh
# Capture la fenêtre de l'application sous Wayland, pour vérifier que sa barre de titre porte bien
# les boutons de fenêtre. Weston ne dessine aucune décoration lui-même, comme GNOME : ce que la
# capture montre est ce que l'application obtient de GTK.
# Usage : wayland-shot.sh <appimage|rpm> <dossier-de-sortie> [nom-de-la-capture]
set -u

APP="$1"
OUT="$2"
NAME="${3:-wayland}"
mkdir -p "$OUT"
command -v weston-screenshooter >/dev/null 2>&1 || dnf install -y -q weston-demo >/dev/null 2>&1 || true

case "$APP" in
  *.rpm)
    dnf install -y -q "$APP" > "$OUT/dnf.log" 2>&1 || { tail -20 "$OUT/dnf.log"; exit 1; }
    PACKAGE=$(rpm -qp --queryformat '%{NAME}' "$APP")
    BIN=$(rpm -ql "$PACKAGE" | grep '^/usr/bin/' | grep -v rclone | head -1)
    ;;
  *)
    cp "$APP" /tmp/chud.AppImage && chmod +x /tmp/chud.AppImage
    BIN=/tmp/chud.AppImage
    ;;
esac

export APPIMAGE_EXTRACT_AND_RUN=1 XDG_RUNTIME_DIR=/tmp/xdg CHUD_COPY_DATA_DIR=/tmp/data
mkdir -p "$XDG_RUNTIME_DIR" /tmp/data
chmod 700 "$XDG_RUNTIME_DIR"

# --debug autorise la capture d'écran par un client ; sans moteur de rendu, il n'y aurait rien à capturer.
weston --backend=headless --renderer=pixman --socket=wl-chud --width=1400 --height=900 --debug > "$OUT/weston.log" 2>&1 &
sleep 3
WAYLAND_DISPLAY=wl-chud GDK_BACKEND=wayland dbus-run-session -- "$BIN" > "$OUT/$NAME.log" 2>&1 &
PID=$!
sleep 16
if kill -0 "$PID" 2>/dev/null; then echo "Wayland : application vivante"; else echo "Wayland : ARRETEE"; tail -20 "$OUT/$NAME.log"; fi
cd "$OUT" && WAYLAND_DISPLAY=wl-chud timeout 30 weston-screenshooter 2>&1 | tail -2
SHOT=$(ls -t "$OUT"/wayland-screenshot-*.png 2>/dev/null | head -1)
if [ -n "$SHOT" ]; then mv "$SHOT" "$OUT/$NAME.png"; echo "capture : $OUT/$NAME.png"; else echo "AUCUNE CAPTURE"; fi
pkill -f -i chud 2>/dev/null
exit 0
