#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
OUT_DIR="scenario_results/gui_regression_pack_${TIMESTAMP}"
mkdir -p "$OUT_DIR"

REPEAT="${STEER_GUI_REG_PACK_REPEAT:-1}"
SCENARIOS="${STEER_GUI_REG_SCENARIOS:-1,2,3,4,5}"
if ! [[ "$REPEAT" =~ ^[0-9]+$ ]] || [ "$REPEAT" -lt 1 ]; then
    echo "❌ invalid STEER_GUI_REG_PACK_REPEAT=${REPEAT}"
    exit 1
fi

echo "🧪 GUI regression pack start"
echo " - repeat: ${REPEAT}"
echo " - scenarios: ${SCENARIOS}"
echo " - output: ${OUT_DIR}"
echo " - mode: approve-assumed + mock external integrations"

PASS_COUNT=0
FAIL_COUNT=0

for i in $(seq 1 "$REPEAT"); do
    ITER_LOG="${OUT_DIR}/iteration_${i}.log"
    ITER_STATUS="${OUT_DIR}/iteration_${i}.status"
    echo ""
    echo "===== iteration ${i}/${REPEAT} ====="

    set +e
    STEER_TEST_MODE=1 \
    STEER_TEST_ASSUME_APPROVED=1 \
    STEER_N8N_MOCK=1 \
    STEER_REQUIRE_TELEGRAM_REPORT=0 \
    STEER_NODE_CAPTURE_ALL=1 \
    STEER_FAIL_ON_FALLBACK=1 \
    STEER_REQUIRE_MAIL_SUBJECT=1 \
    STEER_REQUIRE_SENT_MAILBOX_EVIDENCE=1 \
    STEER_SEMANTIC_REQUIRE_RUST_CONTRACT=1 \
    STEER_SEMANTIC_REQUIRE_NONEMPTY=1 \
    STEER_INPUT_GUARD_MAX_PAUSES=20 \
    STEER_INPUT_GUARD_MAX_PAUSE_SECONDS=180 \
    STEER_SCENARIO_IDS="${SCENARIOS}" \
    bash run_complex_scenarios.sh >"${ITER_LOG}" 2>&1
    EXIT_CODE=$?
    set -e

    if [ "$EXIT_CODE" -eq 0 ]; then
        echo "✅ iteration ${i}: pass"
        PASS_COUNT=$((PASS_COUNT + 1))
        printf 'PASS|%s\n' "${EXIT_CODE}" > "${ITER_STATUS}"
    else
        echo "❌ iteration ${i}: fail (exit=${EXIT_CODE})"
        FAIL_COUNT=$((FAIL_COUNT + 1))
        printf 'FAIL|%s\n' "${EXIT_CODE}" > "${ITER_STATUS}"
    fi
done

SUMMARY_FILE="${OUT_DIR}/summary.txt"
{
    echo "timestamp=${TIMESTAMP}"
    echo "repeat=${REPEAT}"
    echo "scenarios=${SCENARIOS}"
    echo "pass=${PASS_COUNT}"
    echo "fail=${FAIL_COUNT}"
    if [ "$FAIL_COUNT" -gt 0 ]; then
        echo "status=failed"
    else
        echo "status=success"
    fi
} >"${SUMMARY_FILE}"

JUNIT_FILE="${OUT_DIR}/junit.xml"
{
    echo '<?xml version="1.0" encoding="UTF-8"?>'
    echo "<testsuite name=\"gui_regression_pack\" tests=\"${REPEAT}\" failures=\"${FAIL_COUNT}\" timestamp=\"$(date -u +"%Y-%m-%dT%H:%M:%SZ")\">"
    for i in $(seq 1 "$REPEAT"); do
        ITER_LOG="${OUT_DIR}/iteration_${i}.log"
        ITER_STATUS="${OUT_DIR}/iteration_${i}.status"
        STATUS_LINE="$(cat "${ITER_STATUS}" 2>/dev/null || echo "FAIL|999")"
        STATUS_KIND="${STATUS_LINE%%|*}"
        EXIT_CODE="${STATUS_LINE##*|}"
        if [ "${STATUS_KIND}" = "PASS" ]; then
            echo "  <testcase name=\"iteration_${i}\"/>"
        else
            echo "  <testcase name=\"iteration_${i}\">"
            echo "    <failure message=\"exit=${EXIT_CODE}\"><![CDATA["
            tail -n 120 "${ITER_LOG}" 2>/dev/null || true
            echo "    ]]></failure>"
            echo "  </testcase>"
        fi
    done
    echo "</testsuite>"
} > "${JUNIT_FILE}"

echo ""
echo "📊 Regression summary"
echo " - pass: ${PASS_COUNT}"
echo " - fail: ${FAIL_COUNT}"
echo " - summary: ${SUMMARY_FILE}"
echo " - junit: ${JUNIT_FILE}"

if [ "$FAIL_COUNT" -gt 0 ]; then
    exit 1
fi

exit 0
