#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Mika OS entrypoint — boots OpenRC and supervises services.
#
# Lifecycle contract (F2):
#   1. Boot OpenRC via `rc default` (dependency graph: net -> mika-server -> mika-gateway)
#   2. SIGTERM trap: stop services in reverse dependency order, then exit
#   3. Child-exit: supervise-daemon handles respawn (respawn_delay=5, respawn_max=10, respawn_period=60)
#   4. Container stays alive via `tail -F` on log files (backgrounded, trap remains active)
#
# DO NOT use `exec tail -F` — exec replaces the shell and discards the trap,
# breaking SIGTERM propagation to OpenRC services.

set -e

# ── SIGTERM handler: graceful shutdown in reverse dependency order ──
shutdown() {
    echo "[mika-os-init] SIGTERM received, stopping services..."
    rc-service mika-gateway stop 2>/dev/null || true
    rc-service mika-server stop 2>/dev/null || true
    echo "[mika-os-init] Services stopped, exiting."
    exit 0
}

trap shutdown TERM INT

# ── Seed default configs from /etc/mika/ if not already present (F4) ──
# Copy-if-not-exists guard: preserves user-mounted configs on volume restart
# and never overwrites operator customizations.
if [ -n "$MIKA_HOME" ]; then
    mkdir -p "$MIKA_HOME"
    [ -f "$MIKA_HOME/config.toml" ] || cp /etc/mika/config.toml "$MIKA_HOME/config.toml" 2>/dev/null || true
    [ -f "$MIKA_HOME/.env.template" ] || cp /etc/mika/mika.env.template "$MIKA_HOME/.env.template" 2>/dev/null || true
    chown -R mika:mika "$MIKA_HOME" 2>/dev/null || true
fi

# ── Ensure OpenRC directories exist ──
# Some container runtimes don't mount tmpfs at /run
mkdir -p /run/openrc
touch /run/openrc/softlevel

# ── Boot OpenRC default runlevel ──
echo "[mika-os-init] Booting OpenRC default runlevel..."
rc default || {
    echo "[mika-os-init] WARNING: rc default exited with $?, some services may not have started"
}

echo "[mika-os-init] OpenRC boot complete. Services running."

# ── Stay alive: tail log files (backgrounded so trap remains active) ──
# Create log files if they don't exist yet (first boot)
LOG_DIR="${MIKA_HOME:-/home/mika/.mika}/logs"
mkdir -p "$LOG_DIR"
touch "$LOG_DIR/mika-server.log" "$LOG_DIR/mika-gateway.log"

tail -F "$LOG_DIR/mika-server.log" "$LOG_DIR/mika-gateway.log" &
TAIL_PID=$!

# Wait for the tail process — trap will interrupt this on SIGTERM
wait $TAIL_PID
