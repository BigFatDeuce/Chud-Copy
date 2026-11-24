#!/bin/sh
# Lance l'AppImage, le paquet RPM ou le paquet DEB sur une distribution nue, sous X11 puis sous Wayland.
# Usage : fedora-check.sh <appimage|rpm|deb> <dossier-de-sortie>
set -u

APP="$1"
OUT="$2"
mkdir -p "$OUT"
# Un paquet RPM s'installe avec dnf et utilise le WebKitGTK de Fedora ; une AppImage embarque le sien.
case "$APP" in
  *.rpm)
    dnf install -y -q "$APP" > "$OUT/dnf.log" 2>&1 || { tail -20 "$OUT/dnf.log"; exit 1; }
    PACKAGE=$(rpm -qp --queryformat '%{NAME}' "$APP")
    BIN=$(rpm -ql "$PACKAGE" | grep '^/usr/bin/' | grep -v rclone | head -1)
    echo "RPM installe : $(rpm -q "$PACKAGE") -> $BIN"
    printf '#!/bin/sh
exec "%s" "$@"
' "$BIN" > /tmp/chud.AppImage
    ;;
  *.deb)
    cp "$APP" /tmp/chud.deb
    (apt-get update -q && apt-get install -y -q /tmp/chud.deb) > "$OUT/apt.log" 2>&1 || { tail -20 "$OUT/apt.log"; exit 1; }
    PACKAGE=$(dpkg-deb -f /tmp/chud.deb Package)
    BIN=$(dpkg -L "$PACKAGE" | grep '^/usr/bin/' | grep -v rclone | head -1)
    echo "DEB installe : $(dpkg-query -W -f='${Package} ${Version}' "$PACKAGE") -> $BIN"
    printf '#!/bin/sh
exec "%s" "$@"
' "$BIN" > /tmp/chud.AppImage
    ;;
  *)
    cp "$APP" /tmp/chud.AppImage
    ;;
esac
chmod +x /tmp/chud.AppImage
export APPIMAGE_EXTRACT_AND_RUN=1 XDG_RUNTIME_DIR=/tmp/xdg CHUD_COPY_DATA_DIR=/tmp/data
mkdir -p "$XDG_RUNTIME_DIR" /tmp/data/tasks /tmp/lab/src/sous /tmp/lab/dst
chmod 700 "$XDG_RUNTIME_DIR"

# Une tâche réelle dans un dossier temporaire.
echo "un" > /tmp/lab/src/un.txt
echo "deux" > "/tmp/lab/src/sous/été 日本語.txt"
cat > /tmp/data/tasks/essai.json <<'JSON'
{"schema":1,"id":"essai","name":"Essai Fedora","sources":[{"id":"s","path":"/tmp/lab/src"}],
 "destinations":[{"id":"d","path":"/tmp/lab/dst"}],
 "links":[{"id":"l","sourceId":"s","destinationId":"d","subfolder":"","enabled":true}],"mode":"update","createdAt":1,"updatedAt":1}
JSON

cat /etc/fedora-release 2>/dev/null || grep PRETTY_NAME /etc/os-release
for lib in libwebkit2gtk-4.1.so.0 libgtk-3.so.0; do
  ldconfig -p | grep -q "$lib" && echo "PRESENT sur le systeme : $lib" || echo "absent du systeme : $lib"
done

alive() { kill -0 "$1" 2>/dev/null; }

echo "=== X11 ==="
Xvfb :9 -screen 0 1400x900x24 >/dev/null 2>&1 &
sleep 1
DISPLAY=:9 GDK_BACKEND=x11 dbus-run-session -- /tmp/chud.AppImage > "$OUT/x11.log" 2>&1 &
PID=$!
sleep 14
if alive "$PID"; then echo "X11 : application vivante"; else echo "X11 : ARRETEE"; fi
DISPLAY=:9 import -window root "$OUT/x11-accueil.png"
DISPLAY=:9 xdotool search --name "Chud Copy" getwindowgeometry 2>/dev/null | tail -2

# Clique sur Exécuter : la première carte se trouve à une position fixe dans une fenêtre centrée.
if [ -n "${CLICK:-}" ]; then
  DISPLAY=:9 xdotool mousemove $CLICK click 1
  # Assez longtemps pour que la copie se termine et que le son de fin soit joué.
  sleep 12
  if grep -qE "autoaudiosink|invalid \(NULL\) pointer" "$OUT/x11.log"; then echo "ERREUR AUDIO WEBKIT : interface figee"; else echo "Aucune erreur audio de WebKit"; fi
  DISPLAY=:9 import -window root "$OUT/x11-apres-clic.png"
  echo "--- destination :"
  find /tmp/lab/dst -type f | sort
  diff -r /tmp/lab/src /tmp/lab/dst && echo "COPIE IDENTIQUE"
fi
pkill -f chud 2>/dev/null
sleep 1

echo "=== Wayland ==="
weston --backend=headless --socket=wl-chud --width=1400 --height=900 --renderer=gl > "$OUT/weston.log" 2>&1 &
sleep 2
if [ ! -S "$XDG_RUNTIME_DIR/wl-chud" ]; then
  weston --backend=headless --socket=wl-chud --width=1400 --height=900 > "$OUT/weston.log" 2>&1 &
  sleep 2
fi
WAYLAND_DISPLAY=wl-chud GDK_BACKEND=wayland dbus-run-session -- /tmp/chud.AppImage > "$OUT/wayland.log" 2>&1 &
PID=$!
sleep 14
if alive "$PID"; then echo "Wayland : application vivante"; else echo "Wayland : ARRETEE"; fi
pkill -f chud 2>/dev/null
sleep 1

echo "=== Ecritures non latines ==="
for language in zh ar hi; do
  echo "{\"language\":\"$language\",\"quality\":\"off\"}" > /tmp/data/settings.json
  DISPLAY=:9 GDK_BACKEND=x11 dbus-run-session -- /tmp/chud.AppImage > /dev/null 2>&1 &
  sleep 11
  DISPLAY=:9 import -window root "$OUT/x11-$language.png"
  pkill -f chud 2>/dev/null
  sleep 1
  echo "capture $language"
done

echo "--- messages notables :"
grep -hiE 'egl|fatal|abort|cannot|could not|error' "$OUT/x11.log" "$OUT/wayland.log" | sort | uniq -c | head -20
