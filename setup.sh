#!/bin/bash
# setup.sh - Install system dependencies for ytm-cli

echo "Checking system dependencies for ytm-cli..."

# Check OS and install ALSA development headers (needed for rodio/cpal)
if [ -f /etc/debian_version ]; then
    echo "Detected Debian/Ubuntu-based system."
    if ! dpkg -s libasound2-dev >/dev/null 2>&1; then
        echo "Installing libasound2-dev..."
        sudo apt-get update && sudo apt-get install -y libasound2-dev
    else
        echo "libasound2-dev is already installed."
    fi
elif [ -f /etc/redhat-release ]; then
    echo "Detected RedHat/Fedora-based system."
    if ! rpm -q alsa-lib-devel >/dev/null 2>&1; then
        echo "Installing alsa-lib-devel..."
        sudo dnf install -y alsa-lib-devel
    else
        echo "alsa-lib-devel is already installed."
    fi
elif [ -f /etc/arch-release ]; then
    echo "Detected Arch Linux."
    if ! pacman -Qs alsa-lib >/dev/null 2>&1; then
        echo "Installing alsa-lib..."
        sudo pacman -S --noconfirm alsa-lib
    else
        echo "alsa-lib is already installed."
    fi
else
    echo "Warning: Unsupported OS. Please ensure ALSA development headers (libasound2-dev or similar) are installed manually."
fi

# Check for Rust/Cargo
if ! command -v cargo &> /dev/null; then
    echo "Cargo/Rust is not installed. Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
else
    echo "Rust/Cargo is already installed: $(cargo --version)"
fi

echo "Dependencies setup completed!"
