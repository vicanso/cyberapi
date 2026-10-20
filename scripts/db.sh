#!/usr/bin/env bash
# The databases the SQL store's tests run against, in Apple's `container`.
#
#   scripts/db.sh up     start them, creating them the first time
#   scripts/db.sh down   stop them (their data stays until they are removed)
#
# PostgreSQL listens on 127.0.0.1:55432 and MySQL on 127.0.0.1:53306, both
# with user, password and database `cyberapi` — the URLs `make test-sql`
# passes to the tests.
set -euo pipefail

state() {
  container inspect "$1" 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d[0]["status"]["state"] if d else "")' 2>/dev/null \
    || true
}

up() {
  local name=$1 image=$2 publish=$3
  shift 3
  case "$(state "$name")" in
    running) echo "$name: running" ;;
    "")
      container run -d --name "$name" -p "$publish" "$@" "$image" >/dev/null
      echo "$name: created"
      ;;
    *)
      container start "$name" >/dev/null
      echo "$name: started"
      ;;
  esac
}

case "${1:-}" in
  up)
    up cyberapi-pg postgres:18-trixie 127.0.0.1:55432:5432 \
      -e POSTGRES_USER=cyberapi -e POSTGRES_PASSWORD=cyberapi -e POSTGRES_DB=cyberapi
    up cyberapi-mysql mysql:8.4 127.0.0.1:53306:3306 \
      -e MYSQL_ROOT_PASSWORD=cyberapi -e MYSQL_DATABASE=cyberapi \
      -e MYSQL_USER=cyberapi -e MYSQL_PASSWORD=cyberapi
    ;;
  down)
    for name in cyberapi-pg cyberapi-mysql; do
      if [ "$(state "$name")" = running ]; then
        container stop "$name" >/dev/null
        echo "$name: stopped"
      fi
    done
    ;;
  *)
    echo "usage: $0 up|down" >&2
    exit 2
    ;;
esac
