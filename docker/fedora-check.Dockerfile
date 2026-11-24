# Une Fedora nue : uniquement de quoi afficher et piloter une fenêtre, ni WebKit ni outils de développement.
# Les polices Noto sont celles qu'un bureau Fedora ou Nobara fournit pour les écritures non latines.
FROM fedora:latest
RUN dnf install -y --setopt=install_weak_deps=False \
    xorg-x11-server-Xvfb weston mesa-dri-drivers mesa-libEGL mesa-libGL mesa-libgbm \
    xdotool ImageMagick dbus-daemon dbus-tools procps-ng diffutils findutils \
    google-noto-sans-cjk-vf-fonts google-noto-sans-arabic-vf-fonts google-noto-sans-devanagari-vf-fonts google-noto-sans-bengali-vf-fonts
# GStreamer et ses greffons, comme sur un bureau Fedora ou Nobara : l'audio de WebKit les charge.
RUN dnf install -y --setopt=install_weak_deps=False gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good pulseaudio-utils alsa-utils
