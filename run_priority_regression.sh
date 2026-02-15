#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
OUT_DIR="scenario_results/priority_regression_${TIMESTAMP}"
mkdir -p "$OUT_DIR"

CORE_LOG="${OUT_DIR}/core_tests.log"
WEB_LOG="${OUT_DIR}/web_build.log"
CONTRACT_LOG="${OUT_DIR}/contract_checks.log"
PROFILE_MATRIX_LOG="${OUT_DIR}/profile_matrix.log"
SUMMARY_FILE="${OUT_DIR}/summary.txt"

PASS_COUNT=0
FAIL_COUNT=0

run_step() {
    local name="$1"
    local logfile="$2"
    shift 2
    echo "== ${name} =="
    if "$@" >"${logfile}" 2>&1; then
        echo "✅ ${name}"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        echo "❌ ${name} (see ${logfile})"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

run_step "core-tests" "${CORE_LOG}" bash -lc "cd core && cargo test -q"
run_step "web-build" "${WEB_LOG}" bash -lc "cd web && npm run -s build"
run_step "contract-checks" "${CONTRACT_LOG}" bash -lc "
    rg -n 'enum AgentExecutionProfile|collision_policy|execution_options\\(' core/src/api_server.rs &&
    rg -n 'agentExecute\\(planId, executionProfile\\)|agentExecute\\(planRes.plan_id, executionProfile\\)|ExecutionProfile' web/src/features/dashboard/Dashboard.tsx &&
    rg -n 'telegram_transport::send_message_chunked' core/src/telegram.rs core/src/integrations/telegram.rs
"

if [ "${STEER_RUN_PROFILE_MATRIX:-1}" = "1" ]; then
    run_step "profile-matrix" "${PROFILE_MATRIX_LOG}" bash -lc "STEER_PROFILE_MATRIX_MIN_SCORE=\${STEER_PROFILE_MATRIX_MIN_SCORE:-35} STEER_PROFILE_MATRIX_SOFT_FAIL=\${STEER_PROFILE_MATRIX_SOFT_FAIL:-0} STEER_PROFILE_MATRIX_REQUIRE_BUSINESS=\${STEER_PROFILE_MATRIX_REQUIRE_BUSINESS:-1} STEER_PROFILE_MATRIX_PROFILES=\${STEER_PROFILE_MATRIX_PROFILES:-fast} ./run_profile_matrix_regression.sh"
fi

if [ "${STEER_RUN_GUI_REGRESSION:-0}" = "1" ]; then
    GUI_LOG="${OUT_DIR}/gui_regression.log"
    run_step "gui-regression-pack" "${GUI_LOG}" bash -lc "./run_gui_regression_pack.sh"
fi

{
    echo "timestamp=${TIMESTAMP}"
    echo "pass=${PASS_COUNT}"
    echo "fail=${FAIL_COUNT}"
    if [ "$FAIL_COUNT" -eq 0 ]; then
        echo "status=success"
    else
        echo "status=failed"
    fi
} > "${SUMMARY_FILE}"

echo ""
echo "📊 priority regression summary"
echo " - pass: ${PASS_COUNT}"
echo " - fail: ${FAIL_COUNT}"
echo " - out: ${OUT_DIR}"

if [ "$FAIL_COUNT" -ne 0 ]; then
    exit 1
fi
