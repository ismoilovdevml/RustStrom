#!/usr/bin/env bash
# Kernel tuning for the duration of a benchmark run, with save and restore.
#
#   sysctl-tune.sh apply     save current values to $STATE_DIR/sysctl.saved, then tune
#   sysctl-tune.sh restore   write the saved values back and delete the save file
#   sysctl-tune.sh show      print current values
#
# The host is shared, so tuning never lowers a limit: each key is only changed when the
# current value is below the target. `apply` refuses to run while an older save file exists
# (a crashed run); run `restore` first so the true original values are never lost.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

SAVE_FILE="$STATE_DIR/sysctl.saved"
KEYS=(
    net.core.somaxconn
    net.ipv4.tcp_max_syn_backlog
    net.ipv4.ip_local_port_range
    net.ipv4.ip_local_reserved_ports
    net.ipv4.tcp_tw_reuse
    fs.file-max
)
TARGET_SOMAXCONN=65535
TARGET_SYN_BACKLOG=65535
TARGET_PORT_LO=16384
TARGET_PORT_HI=65535
# Our listening ports must never be handed out as ephemeral ports once the range is widened.
RESERVED_PORTS="22000-22999"
TARGET_FILE_MAX=2097152

get() { sysctl -n "$1" | tr '\t' ' ' | tr -s ' '; }

raise() { # key target
    local cur
    cur=$(get "$1")
    if ((cur < $2)); then
        sysctl -qw "$1=$2"
        log "sysctl $1: $cur -> $2"
    fi
}

apply() {
    [[ -e "$SAVE_FILE" ]] && die "$SAVE_FILE exists (unfinished run?). Run '$0 restore' first."
    mkdir -p "$STATE_DIR"
    local k
    for k in "${KEYS[@]}"; do printf '%s\t%s\n' "$k" "$(get "$k")"; done > "$SAVE_FILE.tmp"
    mv "$SAVE_FILE.tmp" "$SAVE_FILE"

    raise net.core.somaxconn "$TARGET_SOMAXCONN"
    raise net.ipv4.tcp_max_syn_backlog "$TARGET_SYN_BACKLOG"
    raise fs.file-max "$TARGET_FILE_MAX"

    # 1 = reuse TIME_WAIT for all outgoing connections; 2 (RHEL default) = loopback only,
    # which already covers this single-host benchmark. Only change 0.
    if [[ $(get net.ipv4.tcp_tw_reuse) == 0 ]]; then
        sysctl -qw net.ipv4.tcp_tw_reuse=2
        log "sysctl net.ipv4.tcp_tw_reuse: 0 -> 2"
    fi

    local reserved
    reserved=$(get net.ipv4.ip_local_reserved_ports)
    if [[ ",$reserved," != *",$RESERVED_PORTS,"* ]]; then
        sysctl -qw net.ipv4.ip_local_reserved_ports="${reserved:+$reserved,}$RESERVED_PORTS"
        log "sysctl net.ipv4.ip_local_reserved_ports: '$reserved' -> '$(get net.ipv4.ip_local_reserved_ports)'"
    fi

    local lo hi
    read -r lo hi <<<"$(get net.ipv4.ip_local_port_range)"
    if ((lo > TARGET_PORT_LO || hi < TARGET_PORT_HI)); then
        local nlo=$((lo < TARGET_PORT_LO ? lo : TARGET_PORT_LO)) nhi=$((hi > TARGET_PORT_HI ? hi : TARGET_PORT_HI))
        sysctl -qw net.ipv4.ip_local_port_range="$nlo $nhi"
        log "sysctl net.ipv4.ip_local_port_range: $lo $hi -> $nlo $nhi"
    fi
}

restore() {
    if [[ ! -e "$SAVE_FILE" ]]; then
        log "nothing to restore ($SAVE_FILE missing)"
        return 0
    fi
    local k v cur
    while IFS=$'\t' read -r k v; do
        cur=$(get "$k")
        if [[ "$cur" != "$v" ]]; then
            sysctl -qw "$k=$v"
            log "sysctl $k restored: '$cur' -> '$(get "$k")'"
        fi
    done < "$SAVE_FILE"
    # Verify before dropping the save file.
    while IFS=$'\t' read -r k v; do
        [[ "$(get "$k")" == "$v" ]] || die "restore of $k failed: now '$(get "$k")', saved '$v' (kept $SAVE_FILE)"
    done < "$SAVE_FILE"
    rm -f "$SAVE_FILE"
}

show() { local k; for k in "${KEYS[@]}"; do printf '%s = %s\n' "$k" "$(get "$k")"; done; }

case "${1:-}" in
    apply) apply ;;
    restore) restore ;;
    show) show ;;
    *) echo "usage: $0 apply|restore|show" >&2; exit 2 ;;
esac
