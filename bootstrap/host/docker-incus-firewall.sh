#!/usr/bin/env bash
# Пропустить трафик мостов Incus (net-*) через цепочку DOCKER-USER, которую Docker оставляет пользователю.
# Без этого Docker'овский FORWARD DROP ломает выход в интернет из Incus-инстансов.
# Идемпотентно: правила добавляются только если их ещё нет.
set -euo pipefail

add_rule() { # add_rule <args...>
  iptables -C DOCKER-USER "$@" 2>/dev/null || iptables -I DOCKER-USER 1 "$@"
}

# ждём, пока Docker создаст цепочку
for _ in $(seq 1 30); do iptables -L DOCKER-USER -n >/dev/null 2>&1 && break; sleep 1; done

add_rule -i 'net-+' -j ACCEPT
add_rule -o 'net-+' -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT

# Изоляция проектов между собой: трафик из одного net-* в другой net-* — запрещён.
# Правило должно стоять ВЫШЕ accept'а по -i net-+, поэтому вставляем последним (в позицию 1).
iptables -C DOCKER-USER -i 'net-+' -o 'net-+' -j DROP 2>/dev/null || iptables -I DOCKER-USER 1 -i 'net-+' -o 'net-+' -j DROP

# Docker включает br_netfilter, поэтому трафик между двумя портами ОДНОГО моста (workspace ↔ gate внутри net-alpha)
# тоже проходит через FORWARD с -i net-alpha -o net-alpha и попадал бы под DROP выше. Пропускаем L2-bridged трафик первым.
# (найдено на стенде: без этого правила Incus-gate недостижим из workspace той же сети)
iptables -C DOCKER-USER -m physdev --physdev-is-bridged -j ACCEPT 2>/dev/null || iptables -I DOCKER-USER 1 -m physdev --physdev-is-bridged -j ACCEPT

echo "DOCKER-USER:"; iptables -S DOCKER-USER
