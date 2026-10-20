#!/usr/bin/env bash
# 单文件行数门禁 —— 防巨石文件回潮与膨胀（docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND2.md）。
#
# 三层防线：
#   1) 绝对限：Rust crates/**/src 与 Flutter lib/ 单文件硬上限 8,000 行（失败）、
#      软上限 4,000 行（警告清单）。生成代码在 EXEMPT_RE 显式豁免。
#   2) 棘轮：scripts/file-size-budget.txt 登记的文件只许减不许增，涨一行都失败。
#      存量巨石因此成为只收敛的目标，而不是「每次 PR 涨一点」的温水煮青蛙。
#   3) 增量：--diff <base> 时（CI 传 push 前的 SHA 或 PR merge-base），超过 2,000 行的存量文件
#      一次改动净增 >300 行即失败——在「往大文件里续写」的决策点上拦截，
#      而不是等绝对限触发。新建文件不走增量门禁（拆分产出的新文件由绝对限管理）。
#
# 用法：
#   bash scripts/check-file-size.sh                     # 绝对限 + 棘轮（`just check-structure`）
#   bash scripts/check-file-size.sh --diff <git-ref>    # 追加增量门禁
#
# 豁免是登记制：改 EXEMPT_RE / 预算文件必须留下理由，没有「惯例豁免」。
set -uo pipefail

HARD_LIMIT=8000
SOFT_LIMIT=4000
DELTA_SIZE_FLOOR=2000
DELTA_NET_LIMIT=300
BUDGET_FILE=scripts/file-size-budget.txt

# 显式豁免（生成代码，非人工维护）：
#   - clients/mica_flutter/lib/src/rust/   flutter_rust_bridge 生成的 FFI 绑定
#   - lib/l10n/app_localizations*.dart     gen-l10n 产物（.arb 才是源）
EXEMPT_RE='^(clients/mica_flutter/lib/src/rust/|clients/mica_flutter/lib/l10n/app_localizations)'

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
warned=0

# ── 1) 绝对限 ────────────────────────────────────────────────────────────────
# shellcheck disable=SC2312
while IFS= read -r f; do
  if [[ "$f" =~ $EXEMPT_RE ]]; then
    continue
  fi
  lines=$(wc -l < "$f")
  if (( lines > HARD_LIMIT )); then
    echo "FAIL  $f — $lines 行，超过硬上限 $HARD_LIMIT（拆分它，或带着理由登记豁免）"
    fail=1
  elif (( lines > SOFT_LIMIT )); then
    echo "WARN  $f — $lines 行，超过软上限 $SOFT_LIMIT（排期拆分参考）"
    warned=1
  fi
done < <(
  {
    find crates -path '*/src/*' -name '*.rs' -not -path '*/target/*' 2>/dev/null
    find clients/mica_flutter/lib -name '*.dart' 2>/dev/null
  } | sort
)

# ── 2) 棘轮（预算文件：登记文件只许减不许增）─────────────────────────────────
if [[ -f $BUDGET_FILE ]]; then
  while read -r count path rest; do
    # 空行与注释行（count 字段以 # 开头）
    [[ -z ${count:-} || ${count:-} == \#* ]] && continue
    if ! [[ $count =~ ^[0-9]+$ ]]; then
      echo "FAIL  $BUDGET_FILE — 无法解析的行：'$count $path $rest'（格式：<基线行数> <路径>）"
      fail=1
      continue
    fi
    if [[ ! -f $path ]]; then
      echo "FAIL  预算条目过期：$path 已不存在（改名/删除后请同步更新 $BUDGET_FILE）"
      fail=1
      continue
    fi
    if [[ "$path" =~ $EXEMPT_RE ]]; then
      continue
    fi
    current=$(wc -l < "$path")
    if (( current > count )); then
      echo "FAIL  $path — 棘轮违规：$count → $current（+$((current - count))）。登记文件只许减不许增；新逻辑开新文件"
      fail=1
    elif (( current < count )); then
      echo "INFO  $path — 已从 $count 降到 $current，请收紧 $BUDGET_FILE 里的基线"
      if (( current <= SOFT_LIMIT )); then
        echo "INFO  $path — 已低于软上限 $SOFT_LIMIT，可直接删除该预算条目"
      fi
    fi
  done < "$BUDGET_FILE"
fi

# ── 3) 增量门禁（--diff 模式）────────────────────────────────────────────────
if [[ $mode == diff ]]; then
  if ! git rev-parse --verify --quiet "$base^{commit}" >/dev/null 2>&1; then
    echo "WARN  增量门禁：基线 '$base' 不是一个有效 commit，跳过（绝对限与棘轮仍生效）"
    warned=1
  else
    # shellcheck disable=SC2312
    while read -r added removed path; do
      [[ ${added:-} =~ ^[0-9]+$ ]] || continue          # 二进制/异常行跳过
      # 与绝对限使用同一个源文件范围；文档、测试金样本、配置不属于代码膨胀门禁。
      case "$path" in
        crates/*/src/*.rs|clients/mica_flutter/lib/*.dart) ;;
        *) continue ;;
      esac
      [[ "$path" == */target/* ]] && continue
      [[ -f $path ]] || continue                         # 本次删除的文件
      git cat-file -e "$base:$path" 2>/dev/null || continue  # 新建文件：由绝对限管理，拆分产物不误伤
      [[ "$path" =~ $EXEMPT_RE ]] && continue
      current=$(wc -l < "$path")
      (( current > DELTA_SIZE_FLOOR )) || continue
      net=$(( added - removed ))
      if (( net > DELTA_NET_LIMIT )); then
        echo "FAIL  $path — 增量门禁：现存 $current 行的文件本次净增 +$net（上限 +$DELTA_NET_LIMIT）。新逻辑请开新文件/子模块，不要续写大文件"
        fail=1
      fi
    done < <(git diff --numstat --no-renames "$base" 2>/dev/null)
  fi
fi

if (( fail )); then
  echo "file-size gate: FAILED（见上方 FAIL 行）"
  exit 1
fi
if (( warned )); then
  echo "file-size gate: passed（含上方 WARN/INFO 清单）"
else
  echo "file-size gate: passed"
fi
