#!/bin/bash

# ============================================================
#  ESP-32 Rust Embedded — Prerequisite Setup Script
# ============================================================
set -euo pipefail

BOLD='\033[1m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Colour

info()    { echo -e "${GREEN}[INFO]${NC}  $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }
section() { echo -e "\n${BOLD}==> $*${NC}"; }

# ------------------------------------------------------------
# 1. System dependencies
# ------------------------------------------------------------
section "Installing system dependencies"
sudo apt-get update -qq
sudo apt-get install -y \
    gcc \
    build-essential \
    curl \
    pkg-config \
    git
info "System dependencies installed."

# ------------------------------------------------------------
# 2. Ensure Rust / cargo is available
# ------------------------------------------------------------
section "Checking for Rust toolchain"
if ! command -v cargo &>/dev/null; then
    warn "cargo not found — installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    # Source cargo env for the rest of this script
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
    info "Rust installed: $(rustc --version)"
else
    info "Rust already installed: $(rustc --version)"
fi

# Make sure cargo is on PATH for subsequent steps
export PATH="$HOME/.cargo/bin:$PATH"

# ------------------------------------------------------------
# 3. Install / verify espup
# ------------------------------------------------------------
section "Checking for espup"
if command -v espup &>/dev/null; then
    info "espup is already installed: $(espup --version 2>/dev/null || true)"
else
    warn "espup not found — building from source..."

    TMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_DIR"' EXIT

    git clone --depth 1 https://github.com/esp-rs/espup "$TMP_DIR/espup"
    cargo install --path "$TMP_DIR/espup" --locked

    info "espup installed: $(espup --version)"
fi

# ------------------------------------------------------------
# 4. Run espup to install the ESP-IDF / Xtensa toolchain
# ------------------------------------------------------------
section "Running espup install"
if [[ ! -f "$HOME/export-esp.sh" ]]; then
    info "Running 'espup install' — this may take several minutes..."
    espup install
    info "espup install complete. export-esp.sh generated."
else
    info "export-esp.sh already exists — skipping espup install."
fi

# ------------------------------------------------------------
# 5. Install cargo tools: esp-generate and espflash
# ------------------------------------------------------------
section "Installing cargo tools"

install_cargo_tool() {
    local crate="$1"
    local bin="${2:-$1}"   # binary name defaults to crate name
    if command -v "$bin" &>/dev/null; then
        info "$crate already installed: $(\"$bin\" --version 2>/dev/null | head -1 || true)"
    else
        info "Installing $crate..."
        cargo install "$crate" --locked
        info "$crate installed."
    fi
}

install_cargo_tool "esp-generate"
install_cargo_tool "espflash"        # provides the `espflash` binary

# ------------------------------------------------------------
# 6. Source the ESP environment
# ------------------------------------------------------------
section "Sourcing ESP environment"
if [[ -f "$HOME/export-esp.sh" ]]; then
    # shellcheck source=/dev/null
    . "$HOME/export-esp.sh"
    info "ESP environment sourced successfully."
else
    error "$HOME/export-esp.sh not found. Did 'espup install' complete successfully?"
fi


# ------------------------------------------------------------
# 7. Detect host IP and write to .env
# ------------------------------------------------------------
section "Detecting host IP address"
 
# Grab the first non-loopback, non-docker inet address from ifconfig.
# Matches lines like:  "        inet 192.168.x.x"
HOST_IP=$(ifconfig 2>/dev/null \
    | awk '/inet / && !/127\.0\.0\.1/ && !/172\.[0-9]+\./ { print $2; exit }')
 
if [[ -z "$HOST_IP" ]]; then
    warn "Could not detect a LAN IP address — HOST_IP will not be written to .env"
else
    info "Detected host IP: $HOST_IP"
 
    ENV_FILE=".env"
 
    # Create .env if it doesn't exist yet
    touch "$ENV_FILE"
 
    if grep -q "^HOST_IP=" "$ENV_FILE" 2>/dev/null; then
        # Overwrite the existing entry in-place
        sed -i "s|^HOST_IP=.*|HOST_IP=$HOST_IP|" "$ENV_FILE"
        info "Updated HOST_IP in $ENV_FILE"
    else
        # Append a new entry
        echo "HOST_IP=$HOST_IP" >> "$ENV_FILE"
        info "Appended HOST_IP to $ENV_FILE"
    fi
fi


# ------------------------------------------------------------
# Done
# ------------------------------------------------------------
echo ""
echo -e "${GREEN}${BOLD}✔  ESP-32 Rust environment is ready!${NC}"
echo ""
echo "  To activate the ESP environment in future shells, add this to your ~/.bashrc or ~/.zshrc:"
echo "    . \$HOME/export-esp.sh"
echo ""
echo "  Quick-start a new project:"
echo "    esp-generate --chip esp32 my_project"
echo "    cd my_project"
echo "    cargo build"
echo ""
