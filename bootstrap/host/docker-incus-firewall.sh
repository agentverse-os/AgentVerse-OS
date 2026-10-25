#!/usr/bin/env bash
# Pass the traffic of the Incus bridges (net-*) through the DOCKER-USER chain, which Docker leaves to the
# user. Without this Docker's FORWARD DROP breaks internet access from Incus instances. Idempotent: the
# rules are added only if not present yet.
set -euo pipefail

add_rule() { # add_rule <args...>
  iptables -C DOCKER-USER "$@" 2>/dev/null || iptables -I DOCKER-USER 1 "$@"
}

# wait until Docker creates the chain
for _ in $(seq 1 30); do iptables -L DOCKER-USER -n >/dev/null 2>&1 && break; sleep 1; done

add_rule -i 'net-+' -j ACCEPT
add_rule -o 'net-+' -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT

# Isolation of projects from each other: traffic from one net-* into another net-* is
# forbidden. The rule must stand ABOVE the accept by -i net-+, so it is inserted last (at
# position 1).
iptables -C DOCKER-USER -i 'net-+' -o 'net-+' -j DROP 2>/dev/null || iptables -I DOCKER-USER 1 -i 'net-+' -o 'net-+' -j DROP

# Docker enables br_netfilter, so traffic between two ports of ONE bridge (workspace ↔ gate inside net-alpha) also goes
# through FORWARD with -i net-alpha -o net-alpha and would fall under the DROP above. Pass L2-bridged traffic first.
# (found on the stand: without this rule the Incus gate is unreachable from a workspace of the same network)
iptables -C DOCKER-USER -m physdev --physdev-is-bridged -j ACCEPT 2>/dev/null || iptables -I DOCKER-USER 1 -m physdev --physdev-is-bridged -j ACCEPT

echo "DOCKER-USER:"; iptables -S DOCKER-USER
