#!/usr/bin/env bash
# 纯 Rust 单元行数门禁 —— 防巨石回潮与膨胀。
#
# 计量单位：Rust 单文件（crates/*/src 与 clients/mica_rust/src）。
# 三层防线：
#   1) 绝对限：硬上限 8,000 行（失败）、软上限 4,000 行（警告清单）。
#   2) 棘轮：scripts/file-size-budget.txt 登记的存量单元只许减不许增。
#   3) 增量：--diff <base> 时，超过 2,000 行的存量文件净增 >300 行即失败。
#
# 用法：
#   bash scripts/check-file-size.sh
#   bash scripts/check-file-size.sh --diff <git-ref>
set -uo pipefail

HARD_LIMIT=8000
SOFT_LIMIT=4000
DELTA_SIZE_FLOOR=2000
DELTA_NET_LIMIT=300
BUDGET_FILE=scripts/file-size-budget.txt

cd "$(dirname "$0")/.."

mode=default
base=
if [[ "${1:-}" == "--diff" ]]; then
  if [[ -z "${2:-}" ]]; then
    echo "usage: $0 [--diff <git-ref>]" >&2
    exit 2
  fi
  mode=diff
  base=$2
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--diff <git-ref>]" >&2
  exit 2
fi

fail=0
declare -A unit_total=()

while IFS= read -r f; do
  lines=$(wc -l < "$f")
  unit_total[$f]=$lines
done < <(
  {
    find crates -path '*/src/*' -name '*.rs' -not -path '*/target/*' 2>/dev/null
    find clients/mica_rust/src -name '*.rs' -not -path '*/target/*' 2>/dev/null
  } | sort
)

declare -A budget_count=()
if [[ -f $BUDGET_FILE ]]; then
  while read -r count path rest; do
    [[ -z ${count:-} || ${count:-} == \#* ]] && continue
    if ! [[ $count =~ ^[0-9]+$ ]]; then
      echo "FAIL  $BUDGET_FILE — 无法解析的行：'$count $path $rest'"
      fail=1
      continue
    fi
    budget_count[$path]=$count
  done < "$BUDGET_FILE"
fi

for u in $(printf '%s\n' "${!unit_total[@]}" | sort); do
  lines=${unit_total[$u]}
  if [[ -n ${budget_count[$u]:-} ]]; then
    limit=${budget_count[$u]}
    if (( lines > limit )); then
      echo "FAIL  $u — $lines 行，超过棘轮基线 $limit 行（只许减不许增）"
      fail=1
    elif (( lines > SOFT_LIMIT )); then
      echo "WARN  $u — $lines 行（棘轮基线 $limit）"
    fi
  else
    if (( lines > HARD_LIMIT )); then
      echo "FAIL  $u — $lines 行，超过硬上限 $HARD_LIMIT 行"
      fail=1
    elif (( lines > SOFT_LIMIT )); then
      echo "WARN  $u — $lines 行，超过软上限 $SOFT_LIMIT（排期拆分参考）"
    fi
  fi
done

for p in "${!budget_count[@]}"; do
  if [[ -z ${unit_total[$p]:-} ]]; then
    echo "FAIL  $BUDGET_FILE 登记的 $p 已不存在，请从预算表移除"
    fail=1
  fi
done

if [[ $mode == diff ]]; then
  while IFS=$'\t' read -r add del path; do
    [[ $add == '-' || $del == '-' ]] && continue
    [[ -n ${unit_total[$path]:-} ]] || continue
    base_lines=$(( $( (git show "$base:$path" 2>/dev/null || true) | wc -l ) ))
    net=$(( add - del ))
    if (( base_lines > DELTA_SIZE_FLOOR && net > DELTA_NET_LIMIT )); then
      echo "FAIL  $path — 基线 $base_lines 行（>${DELTA_SIZE_FLOOR}），本次净增 +$net 行（上限 +${DELTA_NET_LIMIT}）"
      fail=1
    fi
  done < <(git diff --numstat "$base"...HEAD -- crates clients/mica_rust/src)
fi

if (( fail != 0 )); then
  echo "file-size gate: FAILED"
  exit 1
fi
echo "file-size gate: passed"
