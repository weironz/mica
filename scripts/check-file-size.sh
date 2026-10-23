#!/usr/bin/env bash
# 单元行数门禁 —— 防巨石回潮与膨胀（docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND2.md）。
#
# 计量单位（2026-10-23，#87）：
#   Rust  单文件 —— mod 是独立作用域，文件即边界；FFI crate
#         （clients/mica_flutter/rust/src）同在扫描范围内。
#   Dart  library —— root + 其全部 `part` 文件合计。part 与 root 共享同一作用域/
#         命名空间，把代码挪进 part 只是换了个位置（耦合没降，还能绕过按文件计的
#         门禁——483ab092 就是这么过闸的），所以按库合计。
#
# 三层防线：
#   1) 绝对限：单元硬上限 8,000 行（失败）、软上限 4,000 行（警告清单）。生成代码在
#      EXEMPT_RE 显式豁免；登记进预算文件（第 2 层）的存量巨石跳过硬限——棘轮比
#      硬限更紧（涨一行即失败），存量走「只许减」过渡而不是当场炸。
#   2) 棘轮：scripts/file-size-budget.txt 登记的单元只许减不许增，涨一行都失败。
#      存量巨石因此成为只收敛的目标，而不是「每次 PR 涨一点」的温水煮青蛙。
#   3) 增量：--diff <base> 时（CI 传 PR merge-base），超过 2,000 行的存量单元一次
#      改动净增 >300 行即失败——按单元聚合：同一库的 root 与 part 各 +200、合计
#      +400 同样被拦。新建单元不走增量门禁（拆分产出的新文件由绝对限管理）。
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
#   - clients/mica_flutter/lib/src/rust/               flutter_rust_bridge 生成的 FFI 绑定（Dart 侧）
#   - clients/mica_flutter/lib/l10n/app_localizations  gen-l10n 产物（.arb 才是源）
#   - clients/mica_flutter/rust/src/frb_generated.rs   flutter_rust_bridge 生成的 FFI 绑定（Rust 侧）
EXEMPT_RE='^(clients/mica_flutter/lib/src/rust/|clients/mica_flutter/lib/l10n/app_localizations|clients/mica_flutter/rust/src/frb_generated\.rs)'

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

# ── 单元表：Dart part 归并到 library root，其余文件自成单元 ─────────────────────
declare -A unit_of=()      # 文件 → 所属单元（library root 或自身）
declare -A unit_total=()   # 单元 → 行数合计
declare -A unit_files=()   # 单元 → 文件数（报错时说明合计口径）

# root 的 `part 'x.dart';` 声明（URI 相对该文件目录）→ 仓库相对路径。
resolve_part_uri() {
  local p
  p="$(dirname "$1")/$2"
  if command -v realpath >/dev/null 2>&1; then
    realpath -m --relative-to="$PWD" "$p"
  else
    printf '%s\n' "$p"
  fi
}

while IFS= read -r f; do
  [[ "$f" =~ $EXEMPT_RE ]] && continue
  while IFS= read -r p; do
    part_file=$(resolve_part_uri "$f" "$p")
    if [[ ! -f $part_file ]]; then
      echo "WARN  $f 声明了 part '$p'，但 '$part_file' 不存在——该声明按未解析计" >&2
      warned=1
      continue
    fi
    if [[ -n ${unit_of[$part_file]:-} && ${unit_of[$part_file]} != "$f" ]]; then
      echo "WARN  $part_file 被多个 library 声明（${unit_of[$part_file]} 与 $f），以先声明者为准" >&2
      warned=1
      continue
    fi
    unit_of[$part_file]=$f
  done < <(sed -n -E "s/^part ['\"]([^'\"]+)['\"].*/\1/p" "$f")
done < <(find clients/mica_flutter/lib -name '*.dart' 2>/dev/null | sort)

while IFS= read -r f; do
  [[ "$f" =~ $EXEMPT_RE ]] && continue
  lines=$(wc -l < "$f")
  u=${unit_of[$f]:-$f}
  unit_total[$u]=$(( ${unit_total[$u]:-0} + lines ))
  unit_files[$u]=$(( ${unit_files[$u]:-0} + 1 ))
done < <(
  {
    find crates -path '*/src/*' -name '*.rs' -not -path '*/target/*' 2>/dev/null
    find clients/mica_flutter/rust/src -name '*.rs' -not -path '*/target/*' 2>/dev/null
    find clients/mica_flutter/lib -name '*.dart' 2>/dev/null
  } | sort
)

# 预算（棘轮登记表）：路径 → 基线行数。
declare -A budget_count=()
if [[ -f $BUDGET_FILE ]]; then
  while read -r count path rest; do
    # 空行与注释行（count 字段以 # 开头）
    [[ -z ${count:-} || ${count:-} == \#* ]] && continue
    if ! [[ $count =~ ^[0-9]+$ ]]; then
      echo "FAIL  $BUDGET_FILE — 无法解析的行：'$count $path $rest'（格式：<基线行数> <路径>）"
      fail=1
      continue
    fi
    budget_count[$path]=$count
  done < "$BUDGET_FILE"
fi

# ── 1) 绝对限（按单元）────────────────────────────────────────────────────────
while IFS= read -r u; do
  [[ -z $u ]] && continue
  [[ "$u" =~ $EXEMPT_RE ]] && continue
  lines=${unit_total[$u]}
  if (( ${unit_files[$u]} > 1 )); then
    desc="（library，${unit_files[$u]} 个文件合计）"
  else
    desc=""
  fi
  if (( lines > HARD_LIMIT )); then
    if [[ -n ${budget_count[$u]:-} ]]; then
      echo "WARN  $u$desc — $lines 行，超硬上限 $HARD_LIMIT（已在预算登记，由棘轮「只许减」约束——涨一行即失败）"
      warned=1
    else
      echo "FAIL  $u$desc — $lines 行，超过硬上限 $HARD_LIMIT（拆分它，或带着理由登记进 $BUDGET_FILE）"
      fail=1
    fi
  elif (( lines > SOFT_LIMIT )); then
    echo "WARN  $u$desc — $lines 行，超过软上限 $SOFT_LIMIT（排期拆分参考）"
    warned=1
  fi
done < <(printf '%s\n' "${!unit_total[@]}" | sort)

# ── 2) 棘轮（预算文件：登记单元只许减不许增）─────────────────────────────────
while IFS= read -r path; do
  [[ -z $path ]] && continue
  if [[ ! -f $path ]]; then
    echo "FAIL  预算条目过期：$path 已不存在（改名/删除后请同步更新 $BUDGET_FILE）"
    fail=1
    continue
  fi
  [[ "$path" =~ $EXEMPT_RE ]] && continue
  count=${budget_count[$path]}
  # Dart 条目按其所属库计：登记一个 part，核对的也是整个 library 的合计。
  unit=${unit_of[$path]:-$path}
  current=${unit_total[$unit]:-}
  [[ -z $current ]] && current=$(wc -l < "$path")
  if (( current > count )); then
    echo "FAIL  $path — 棘轮违规：$count → $current（+$((current - count))）。登记单元只许减不许增；新逻辑开新文件"
    fail=1
  elif (( current < count )); then
    echo "INFO  $path — 已从 $count 降到 $current，请收紧 $BUDGET_FILE 里的基线"
    if (( current <= SOFT_LIMIT )); then
      echo "INFO  $path — 已低于软上限 $SOFT_LIMIT，可直接删除该预算条目"
    fi
  fi
done < <(printf '%s\n' "${!budget_count[@]}" | sort)

# ── 3) 增量门禁（--diff 模式，按单元聚合）────────────────────────────────────
if [[ $mode == diff ]]; then
  if ! git rev-parse --verify --quiet "$base^{commit}" >/dev/null 2>&1; then
    echo "WARN  增量门禁：基线 '$base' 不是一个有效 commit，跳过（绝对限与棘轮仍生效）"
    warned=1
  else
    declare -A unit_added=() unit_removed=()
    while read -r added removed path; do
      [[ ${added:-} =~ ^[0-9]+$ ]] || continue          # 二进制/异常行跳过
      # 与绝对限使用同一个源文件范围；文档、测试金样本、配置不属于代码膨胀门禁。
      case "$path" in
        crates/*/src/*.rs|clients/mica_flutter/lib/*.dart|clients/mica_flutter/rust/src/*.rs) ;;
        *) continue ;;
      esac
      [[ "$path" == */target/* ]] && continue
      [[ -f $path ]] || continue                       # 本次删除的文件
      [[ "$path" =~ $EXEMPT_RE ]] && continue
      u=${unit_of[$path]:-$path}
      unit_added[$u]=$(( ${unit_added[$u]:-0} + added ))
      unit_removed[$u]=$(( ${unit_removed[$u]:-0} + removed ))
    done < <(git diff --numstat --no-renames "$base" 2>/dev/null)

    while IFS= read -r u; do
      [[ -z $u ]] && continue
      net=$(( ${unit_added[$u]} - ${unit_removed[$u]:-0} ))
      (( net > DELTA_NET_LIMIT )) || continue
      # 新建单元（root 在 base 中不存在）由绝对限管理，拆分产物不误伤。
      git cat-file -e "$base:$u" 2>/dev/null || continue
      current=${unit_total[$u]:-0}
      (( current > DELTA_SIZE_FLOOR )) || continue
      echo "FAIL  $u — 增量门禁：现存 $current 行的单元本次净增 +$net（上限 +$DELTA_NET_LIMIT）。新逻辑请开新文件/子模块，不要续写大文件"
      fail=1
    done < <(printf '%s\n' "${!unit_added[@]}" | sort)
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
