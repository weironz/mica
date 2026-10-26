#!/usr/bin/env bash
# Exercise the REAL scripts/ci-container-stack.sh and scripts/ci-web-e2e.sh
# against a Docker/Compose double, twice each, with a developer stack seeded in
# the same daemon.
#
# The two failures this locks down:
#   #126  the job left its stack running (fixed-name container on a fixed port,
#         `docker compose up -d` with no `down`), so the next run on the same
#         self-hosted runner collided on the name/port.
#   review  the first fix ran `down -v --remove-orphans` WITHOUT a project name,
#         which compose resolves from the checkout directory — `mica` — i.e. the
#         exact project a developer's `just dev` stack owns. One CI run would
#         have deleted their containers, network and named volumes.
#
# The double models ports as daemon-wide (a leftover mapping really does fail
# the next `up`) and volumes/orphans as project-scoped, so a `down` that drops
# `-v` or `--remove-orphans` shows up as LEFTOVER FILES, not as a grepped flag.
# The seeded developer stack uses its own ports (5432/8080): with the defaults
# of deploy/docker-compose.single.yml (80/9000/9001) two stacks cannot share a
# daemon at all, which is why the runner runs one job at a time. The property
# under test here is ownership: one project's `down` must not reach another's.
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture=$(mktemp -d "$temp_parent/mica-ci-stacks.XXXXXX")
[[ "$fixture" == "$temp_parent"/mica-ci-stacks.* ]] || exit 2
trap 'rm -rf -- "$fixture"' EXIT

bin="$fixture/bin"
state="$fixture/daemon"
work="$fixture/work"
mkdir -p "$bin" "$state/ports" "$state/containers" "$state/projects" \
  "$work/clients/mica_flutter/build/web" "$work/deploy"
printf 'ok\n' > "$work/clients/mica_flutter/build/web/index.html"
printf 'events {}\n' > "$work/deploy/nginx.dev.conf"
: > "$state/invocations.log"

# ── Docker + Compose double ─────────────────────────────────────────────────
cat > "$bin/docker" <<EOF
#!/bin/sh
state='$state'
cmd=\$1; shift
case "\$cmd" in
  compose) exec '$bin/compose' "\$@" ;;
  run)
    name=""; port=""
    while [ \$# -gt 0 ]; do
      case "\$1" in
        --name) name=\$2; shift 2 ;;
        -p) port=\${2%%:*}; shift 2 ;;
        *) shift ;;
      esac
    done
    if [ -n "\$name" ] && [ -f "\$state/containers/\$name" ]; then
      echo "docker: Error response from daemon: Conflict. The container name \\"/\$name\\" is already in use" >&2
      exit 125
    fi
    if [ -n "\$port" ] && [ -f "\$state/ports/\$port" ]; then
      echo "docker: Error response from daemon: Bind for 0.0.0.0:\$port failed: port is already allocated" >&2
      exit 125
    fi
    [ -n "\$name" ] && printf '%s' "\$port" > "\$state/containers/\$name"
    [ -n "\$port" ] && printf 'container:%s' "\$name" > "\$state/ports/\$port"
    exit 0
    ;;
  rm)
    [ "\$1" = -f ] && shift
    for name in "\$@"; do
      if [ ! -f "\$state/containers/\$name" ]; then
        echo "Error response from daemon: No such container: \$name" >&2
        exit 1
      fi
      port=\$(cat "\$state/containers/\$name")
      [ -n "\$port" ] && rm -f "\$state/ports/\$port"
      rm -f "\$state/containers/\$name"
    done
    exit 0
    ;;
esac
exit 0
EOF

cat > "$bin/compose" <<EOF
#!/bin/sh
state='$state'
project=""; verb=""; volumes=0; orphans=0
while [ \$# -gt 0 ]; do
  case "\$1" in
    --project-name|-p) project=\$2; shift 2 ;;
    up|down|logs) verb=\$1; shift ;;
    -v|--volumes) volumes=1; shift ;;
    --remove-orphans) orphans=1; shift ;;
    *) shift ;;
  esac
done
# No --project-name means compose derives the project from the directory it runs
# in. CI checks the repository out as 'mica', which is also the developer
# stack's project name — that is the whole hazard.
echo "\$verb \${project:-mica}" >> "\$state/invocations.log"
dir="\$state/projects/\${project:-mica}"
case "\$verb" in
  up)
    # The project's own files, and daemon-wide ports. Anything already holding
    # 80/9000/9001 — a crashed run's stack, or another project — is a conflict.
    for port in 80 9000 9001; do
      if [ -f "\$state/ports/\$port" ]; then
        echo "Error response from daemon: Bind for 0.0.0.0:\$port failed: port is already allocated" >&2
        exit 1
      fi
    done
    mkdir -p "\$dir/volumes" "\$dir/containers" "\$dir/orphans" "\$dir/ports"
    : > "\$dir/network"
    for vol in mica-prod-postgres mica-prod-rustfs; do : > "\$dir/volumes/\$vol"; done
    for svc in api web postgres rustfs; do : > "\$dir/containers/\$svc"; done
    # A service the compose files no longer declare: only --remove-orphans
    # clears it.
    : > "\$dir/orphans/stale-sidecar"
    for port in 80 9000 9001; do
      : > "\$dir/ports/\$port"
      printf 'project:%s' "\$project" > "\$state/ports/\$port"
    done
    exit 0
    ;;
  logs) exit 0 ;;
  down)
    [ -d "\$dir" ] || exit 0
    rm -f "\$dir/network" "\$dir/containers/"*
    for f in "\$dir/ports/"*; do
      [ -e "\$f" ] || continue
      rm -f "\$state/ports/\${f##*/}"
    done
    rm -f "\$dir/ports/"*
    # The flags do the remaining work; missing ones simply leave files behind.
    [ "\$volumes" = 1 ] && rm -f "\$dir/volumes/"*
    [ "\$orphans" = 1 ] && rm -f "\$dir/orphans/"*
    exit 0
    ;;
esac
exit 2
EOF

cat > "$bin/sleep" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$bin"/*

run() { # run a script with the double first on PATH, from the fixture worktree
  (cd "$work" && PATH="$bin:$PATH" "$@")
}

# ── the developer's stack, project `mica` ───────────────────────────────────
mkdir -p "$state/projects/mica/volumes" "$state/projects/mica/containers"
: > "$state/projects/mica/network"
: > "$state/projects/mica/volumes/mica-prod-postgres"
printf 'project:mica' > "$state/ports/5432"
: > "$state/projects/mica/containers/api"
printf '8080' > "$state/containers/mica-api-1"
printf 'project:mica' > "$state/ports/8080"
dev_before=$(cd "$state" && find . -type f | sort)

project_dir() { echo "$state/projects/$1"; }
leftovers() { # files still inside a project, one per line
  [ -d "$1" ] && (cd "$1" && find . -mindepth 1 -type f | sort) || true
}

# ── container job: two attempts, same daemon ────────────────────────────────
for attempt in 1 2; do
  project="mica-ci-4242-$attempt"
  run env GITHUB_RUN_ID=4242 GITHUB_RUN_ATTEMPT="$attempt" \
    bash "$source_root/scripts/ci-container-stack.sh" up \
    || { echo "FAIL: container stack up failed on attempt $attempt" >&2; exit 1; }
  run env GITHUB_RUN_ID=4242 GITHUB_RUN_ATTEMPT="$attempt" \
    bash "$source_root/scripts/ci-container-stack.sh" logs >/dev/null \
    || { echo "FAIL: container stack logs failed on attempt $attempt" >&2; exit 1; }
  run env GITHUB_RUN_ID=4242 GITHUB_RUN_ATTEMPT="$attempt" \
    bash "$source_root/scripts/ci-container-stack.sh" down \
    || { echo "FAIL: container stack down failed on attempt $attempt" >&2; exit 1; }
  [ -z "$(leftovers "$(project_dir "$project")")" ] || {
    echo "FAIL: attempt $attempt left its stack behind: $(leftovers "$(project_dir "$project")")" >&2
    exit 1
  }
  for port in 80 9000 9001; do
    [ ! -f "$state/ports/$port" ] || {
      echo "FAIL: attempt $attempt left port $port published — the next run collides" >&2
      exit 1
    }
  done
done

# Every invocation must have been scoped to the per-attempt project. A bare
# `down -v` (no project) is the destructive case this guards.
while read -r verb project; do
  case "$project" in
    mica-ci-4242-1|mica-ci-4242-2) ;;
    *) echo "FAIL: '$verb' ran against project '$project', not the per-run project" >&2; exit 1 ;;
  esac
done < "$state/invocations.log"
[ "$(grep -c '^down ' "$state/invocations.log")" = 2 ] || {
  echo 'FAIL: expected one down per attempt' >&2
  cat "$state/invocations.log" >&2
  exit 1
}

# The project the developer's stack owns may never be named by the script, and
# `down` must refuse it outright rather than trust the caller.
if run env MICA_CI_COMPOSE_PROJECT=mica \
     bash "$source_root/scripts/ci-container-stack.sh" down > "$fixture/refusal.log" 2>&1; then
  echo 'FAIL: down accepted the developer stack project name' >&2
  exit 1
fi
grep -q 'REFUSED' "$fixture/refusal.log" || {
  echo 'FAIL: the refusal does not say why' >&2
  cat "$fixture/refusal.log" >&2
  exit 1
}

# ── web-e2e: fixed-name nginx, removed after each run ──────────────────────
for attempt in 1 2; do
  run bash "$source_root/scripts/ci-web-e2e.sh" serve \
    || { echo "FAIL: web-e2e serve failed on run $attempt" >&2; exit 1; }
  run bash "$source_root/scripts/ci-web-e2e.sh" stop \
    || { echo "FAIL: web-e2e stop failed on run $attempt" >&2; exit 1; }
done
# Stop with nothing running is not an error: the job can fail before `serve`.
run bash "$source_root/scripts/ci-web-e2e.sh" stop \
  || { echo 'FAIL: stop on an already-clean daemon must succeed' >&2; exit 1; }
[ ! -f "$state/containers/mica-web-e2e" ] || {
  echo 'FAIL: the nginx container survived its cleanup' >&2
  exit 1
}
[ ! -f "$state/ports/8090" ] || { echo 'FAIL: port 8090 is still published' >&2; exit 1; }

# ── nothing that belonged to the developer moved ───────────────────────────
for path in $dev_before; do
  [ -f "$state/$path" ] || {
    echo "FAIL: the CI teardown removed the developer stack's $path" >&2
    exit 1
  }
done
[ -f "$state/containers/mica-api-1" ] || { echo "FAIL: a foreign container was removed" >&2; exit 1; }
[ -f "$state/ports/5432" ] || { echo "FAIL: the developer postgres port was released" >&2; exit 1; }

# ── ground truth, not a text assertion: render the real stack ──────────────
# The per-run project name is only worth something if the compose files
# themselves allow it — no fixed `container_name:` (which would make two
# projects collide on the daemon) and exactly the published ports the script
# documents. Run the real `docker compose config` when a daemon is present; on
# CI (self-hosted Linux) it is, and this catches a future `container_name:`
# added to the overlay.
if docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
  rendered=$(docker compose --project-directory "$source_root" --project-name mica-ci-4242-1 \
    --env-file "$source_root/e2e/.env.ci" \
    -f "$source_root/deploy/docker-compose.single.yml" \
    -f "$source_root/e2e/docker-compose.from-source.yml" config) \
    || { echo 'FAIL: docker compose config does not render the CI stack' >&2; exit 1; }
  case "$rendered" in
    *container_name:*) echo 'FAIL: the stack declares a fixed container_name — two runs cannot share a daemon' >&2; exit 1 ;;
  esac
  for pair in 'published: "80"' 'published: "9000"' 'published: "9001"'; do
    printf '%s\n' "$rendered" | grep -qF "$pair" \
      || { echo "FAIL: the rendered stack does not publish $pair as documented" >&2; exit 1; }
  done
  published=$(printf '%s\n' "$rendered" | grep -cF 'published:')
  [ "$published" = 3 ] || { echo "FAIL: expected 3 published ports, the stack renders $published" >&2; exit 1; }
else
  echo 'NOTE: Docker unavailable — skipped the real `docker compose config` ground-truth check'
fi
echo 'PASS: CI stacks are per-run scoped, cleaned up on repeat runs, and never touch the developer stack'