#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  ubuntu)
    sudo apt-get update
    sudo apt-get install -y \
      build-essential \
      curl \
      git \
      pkg-config \
      clang \
      libclang-dev \
      libglib2.0-dev \
      libpipewire-0.3-dev \
      libdrm-dev \
      libinput-dev \
      libudev-dev \
      libegl1-mesa-dev \
      libwayland-dev \
      libxkbcommon-dev \
      libgstreamer1.0-dev \
      libgstreamer-plugins-base1.0-dev
    ;;
  arch)
    pacman -Syu --noconfirm
    pacman -S --noconfirm \
      clang \
      glib2 \
      gstreamer \
      gst-plugins-base \
      libdrm \
      libinput \
      libxkbcommon \
      pipewire \
      mesa \
      pkgconf \
      wayland \
      systemd-libs
    ;;
  *)
    echo "usage: $0 {ubuntu|arch}" >&2
    exit 1
    ;;
esac