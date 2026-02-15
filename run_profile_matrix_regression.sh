#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
OUT_DIR="scenario_results/profile_matrix_${TIMESTAMP}"
mkdir -p "$OUT_DIR"
OUT_DIR_ABS="$(cd "$OUT_DIR" && pwd)"

API_PORT="${STEER_PROFILE_MATRIX_PORT:-5682}"
API_BASE="${STEER_API_BASE:-http://127.0.0.1:${API_PORT}/api}"
HEALTH_URL="${STEER_API_HEALTH_URL:-${API_BASE}/system/health}"
PROMPT="${STEER_PROFILE_MATRIX_PROMPT:-서울에서 도쿄 2026-03-10 항공권을 찾아줘}"
PROFILES_RAW="${STEER_PROFILE_MATRIX_PROFILES:-fast}"
EXEC_TIMEOUT_SEC="${STEER_PROFILE_MATRIX_EXEC_TIMEOUT_SEC:-120}"
MIN_SCORE="${STEER_PROFILE_MATRIX_MIN_SCORE:-35}"
SOFT_FAIL="${STEER_PROFILE_MATRIX_SOFT_FAIL:-0}"
ASSUME_APPROVED="${STEER_PROFILE_MATRIX_ASSUME_APPROVED:-1}"
APPROVAL_DECISION="${STEER_PROFILE_MATRIX_APPROVAL_DECISION:-allow_once}"
MAX_APPROVAL_LOOPS="${STEER_PROFILE_MATRIX_MAX_APPROVAL_LOOPS:-8}"
REQUIRE_BUSINESS="${STEER_PROFILE_MATRIX_REQUIRE_BUSINESS:-1}"

CSV_FILE="${OUT_DIR_ABS}/matrix.csv"
MD_FILE="${OUT_DIR_ABS}/matrix.md"
SUMMARY_FILE="${OUT_DIR_ABS}/summary.txt"
CORE_LOG="${OUT_DIR_ABS}/core_server.log"

CORE_STARTED=0
CORE_PID=""

cleanup() {
    if [ "$CORE_STARTED" -eq 1 ] && [ -n "$CORE_PID" ]; then
        kill "$CORE_PID" 2>/dev/null || true
        wait "$CORE_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

wait_for_api() {
    local max_wait="${1:-45}"
    local waited=0
    while [ "$waited" -lt "$max_wait" ]; do
        if curl -fsS "${HEALTH_URL}" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    return 1
}

if ! curl -fsS "${HEALTH_URL}" >/dev/null 2>&1; then
    echo "ℹ️ API not running. Starting core server..."
    (
        cd core
        nohup env \
            STEER_API_PORT="${API_PORT}" \
            STEER_API_ALLOW_NO_KEY=1 \
            STEER_DEV_LOCAL_MODE=1 \
            STEER_API_KEY= \
            cargo run --bin local_os_agent >"${CORE_LOG}" 2>&1 &
        echo $! > "${OUT_DIR_ABS}/core.pid"
    )
    CORE_PID="$(cat "${OUT_DIR_ABS}/core.pid")"
    CORE_STARTED=1
    if ! wait_for_api 60; then
        echo "❌ failed to start API server. see ${CORE_LOG}"
        exit 1
    fi
fi

echo "profile,status,run_id,planner_complete,execution_complete,business_complete,business_gate_pass,failed_assertions,approval_loops,score" >"${CSV_FILE}"
{
    echo "# Profile Matrix Regression"
    echo ""
    echo "- timestamp: ${TIMESTAMP}"
    echo "- api: ${API_BASE}"
    echo "- prompt: ${PROMPT}"
    echo "- assume_approved: ${ASSUME_APPROVED}"
    echo "- approval_decision: ${APPROVAL_DECISION}"
    echo "- max_approval_loops: ${MAX_APPROVAL_LOOPS}"
    echo "- require_business: ${REQUIRE_BUSINESS}"
    echo ""
} > "${MD_FILE}"

score_for() {
    local status="$1"
    local planner_complete="$2"
    local execution_complete="$3"
    local business_complete="$4"
    local failed_assertions="$5"
    python3 - "$status" "$planner_complete" "$execution_complete" "$business_complete" "$failed_assertions" <<'PY'
import sys
status, planner, execution, business, failed = sys.argv[1:6]
score = 0
if planner == "true":
    score += 20
if execution == "true":
    score += 25
if business == "true":
    score += 35
if status in ("completed", "success"):
    score += 10
if failed == "0":
    score += 10
else:
    score -= min(10, int(failed) * 2)
score = max(0, min(100, score))
print(score)
PY
}

to_json() {
    local py="$1"
    shift
    python3 - "$py" "$@" <<'PY'
import json, sys
code = sys.argv[1]
args = sys.argv[2:]
if code == "intent":
    print(json.dumps({"text": args[0]}, ensure_ascii=False))
elif code == "plan":
    print(json.dumps({"session_id": args[0]}, ensure_ascii=False))
elif code == "execute":
    print(json.dumps({"plan_id": args[0], "profile": args[1]}, ensure_ascii=False))
elif code == "approve":
    print(json.dumps({"plan_id": args[0], "action": args[1], "decision": args[2]}, ensure_ascii=False))
else:
    raise SystemExit(f"unknown payload type: {code}")
PY
}

profiles="$(printf '%s' "$PROFILES_RAW" | tr ',' ' ')"
total=0
failed=0

for profile in $profiles; do
    total=$((total + 1))
    profile="$(echo "$profile" | tr -d '[:space:]' | tr '[:upper:]' '[:lower:]')"
    case "$profile" in
        strict|test|fast) ;;
        *) echo "⚠️ skip unknown profile: $profile"; continue ;;
    esac

    intent_payload="$(to_json intent "$PROMPT")"
    intent_file="${OUT_DIR_ABS}/intent_${profile}.json"
    intent_http="$(curl -sS -o "${intent_file}" -w "%{http_code}" -X POST "${API_BASE}/agent/intent" -H 'Content-Type: application/json' -d "${intent_payload}" || true)"
    if [ "${intent_http}" -lt 200 ] || [ "${intent_http}" -ge 300 ]; then
        echo "❌ ${profile}: intent failed (http=${intent_http})"
        failed=$((failed + 1))
        echo "${profile},error,,false,false,false,0,0,0,0" >> "${CSV_FILE}"
        continue
    fi
    intent_resp="$(cat "${intent_file}")"
    session_id="$(printf '%s' "$intent_resp" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("session_id",""))')"
    if [ -z "$session_id" ]; then
        echo "❌ ${profile}: missing session_id"
        failed=$((failed + 1))
        echo "${profile},error,,false,false,false,0,0,0,0" >> "${CSV_FILE}"
        continue
    fi

    plan_payload="$(to_json plan "$session_id")"
    plan_file="${OUT_DIR_ABS}/plan_${profile}.json"
    plan_http="$(curl -sS -o "${plan_file}" -w "%{http_code}" -X POST "${API_BASE}/agent/plan" -H 'Content-Type: application/json' -d "${plan_payload}" || true)"
    if [ "${plan_http}" -lt 200 ] || [ "${plan_http}" -ge 300 ]; then
        echo "❌ ${profile}: plan failed (http=${plan_http})"
        failed=$((failed + 1))
        echo "${profile},error,,false,false,false,0,0,0,0" >> "${CSV_FILE}"
        continue
    fi
    plan_resp="$(cat "${plan_file}")"
    plan_id="$(printf '%s' "$plan_resp" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("plan_id",""))')"
    if [ -z "$plan_id" ]; then
        echo "❌ ${profile}: missing plan_id"
        failed=$((failed + 1))
        echo "${profile},error,,false,false,false,0,0" >> "${CSV_FILE}"
        continue
    fi

    exec_payload="$(to_json execute "$plan_id" "$profile")"
    exec_file="${OUT_DIR_ABS}/execute_${profile}.json"
    if ! curl -fsS --max-time "${EXEC_TIMEOUT_SEC}" -X POST "${API_BASE}/agent/execute" \
        -H 'Content-Type: application/json' \
        -d "${exec_payload}" > "${exec_file}"; then
        echo "❌ ${profile}: execute timeout/error"
        failed=$((failed + 1))
        echo "${profile},error,,false,false,false,0,0,0,0" >> "${CSV_FILE}"
        continue
    fi

    status="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("status",""))' "${exec_file}")"
    approval_loops=0
    while [ "${ASSUME_APPROVED}" = "1" ] && [ "${status}" = "approval_required" ] && [ "${approval_loops}" -lt "${MAX_APPROVAL_LOOPS}" ]; do
        approval_action="$(python3 -c 'import json,sys; print(((json.load(open(sys.argv[1])).get("approval") or {}).get("action") or ""))' "${exec_file}")"
        if [ -z "${approval_action}" ]; then
            break
        fi
        approval_payload="$(to_json approve "$plan_id" "$approval_action" "$APPROVAL_DECISION")"
        approval_file="${OUT_DIR_ABS}/approve_${profile}_${approval_loops}.json"
        approval_http="$(curl -sS -o "${approval_file}" -w "%{http_code}" -X POST "${API_BASE}/agent/approve" -H 'Content-Type: application/json' -d "${approval_payload}" || true)"
        if [ "${approval_http}" -lt 200 ] || [ "${approval_http}" -ge 300 ]; then
            status="error"
            break
        fi
        approval_status="$(python3 -c 'import json,sys; print((json.load(open(sys.argv[1])).get("status") or ""))' "${approval_file}")"
        if [ "${approval_status}" = "denied" ]; then
            status="denied"
            break
        fi
        approval_loops=$((approval_loops + 1))
        exec_file="${OUT_DIR_ABS}/execute_${profile}_resume_${approval_loops}.json"
        if ! curl -fsS --max-time "${EXEC_TIMEOUT_SEC}" -X POST "${API_BASE}/agent/execute" \
            -H 'Content-Type: application/json' \
            -d "${exec_payload}" > "${exec_file}"; then
            status="error"
            break
        fi
        status="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("status",""))' "${exec_file}")"
    done

    run_id="$(python3 -c 'import json,sys; print((json.load(open(sys.argv[1])).get("run_id") or ""))' "${exec_file}")"
    planner_complete="$(python3 -c 'import json,sys; print(str(bool(json.load(open(sys.argv[1])).get("planner_complete", False))).lower())' "${exec_file}")"
    execution_complete="$(python3 -c 'import json,sys; print(str(bool(json.load(open(sys.argv[1])).get("execution_complete", False))).lower())' "${exec_file}")"
    business_complete="$(python3 -c 'import json,sys; print(str(bool(json.load(open(sys.argv[1])).get("business_complete", False))).lower())' "${exec_file}")"

    failed_assertions=0
    if [ -n "$run_id" ]; then
        assertions_file="${OUT_DIR_ABS}/assertions_${profile}.json"
        if curl -fsS "${API_BASE}/agent/task-runs/${run_id}/assertions" > "${assertions_file}"; then
            failed_assertions="$(python3 -c 'import json,sys; arr=json.load(open(sys.argv[1])); print(sum(1 for x in arr if not x.get("passed", False)))' "${assertions_file}")"
        fi
    fi

    score="$(score_for "$status" "$planner_complete" "$execution_complete" "$business_complete" "$failed_assertions")"
    business_gate_pass=1
    if [ "${REQUIRE_BUSINESS}" = "1" ] && [ "${business_complete}" != "true" ]; then
        business_gate_pass=0
    fi
    echo "${profile},${status},${run_id},${planner_complete},${execution_complete},${business_complete},${business_gate_pass},${failed_assertions},${approval_loops},${score}" >> "${CSV_FILE}"

    {
        echo "## ${profile}"
        echo "- status: ${status}"
        echo "- run_id: ${run_id:-n/a}"
        echo "- planner_complete: ${planner_complete}"
        echo "- execution_complete: ${execution_complete}"
        echo "- business_complete: ${business_complete}"
        echo "- business_gate_pass: ${business_gate_pass}"
        echo "- failed_assertions: ${failed_assertions}"
        echo "- approval_loops: ${approval_loops}"
        echo "- score: ${score}"
        echo ""
    } >> "${MD_FILE}"

    if [ "$score" -lt "$MIN_SCORE" ] || [ "${business_gate_pass}" -eq 0 ]; then
        failed=$((failed + 1))
    fi
done

{
    echo "timestamp=${TIMESTAMP}"
    echo "profiles=${PROFILES_RAW}"
    echo "total=${total}"
    echo "fail=${failed}"
    echo "min_score=${MIN_SCORE}"
    echo "require_business=${REQUIRE_BUSINESS}"
    if [ "$failed" -eq 0 ]; then
        echo "status=success"
    else
        echo "status=failed"
    fi
} > "${SUMMARY_FILE}"

echo "📊 profile matrix summary"
cat "${SUMMARY_FILE}"
echo "📁 output: ${OUT_DIR_ABS}"

if [ "$failed" -ne 0 ]; then
    if [ "$SOFT_FAIL" = "1" ]; then
        echo "⚠️ profile matrix completed with failures (soft-fail enabled)"
        exit 0
    fi
    exit 1
fi
