# Une Ubuntu nue, comme pour fedora-check : de quoi afficher et piloter une fenêtre, GStreamer et les polices.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends     xvfb weston xdotool imagemagick dbus dbus-x11 procps diffutils findutils     libgl1-mesa-dri libegl1 libgbm1     gstreamer1.0-plugins-base gstreamer1.0-plugins-good pulseaudio-utils alsa-utils     fonts-noto-cjk fonts-noto-core     && rm -rf /var/lib/apt/lists/*
