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

normalize_scenario_ids() {
    local raw="$1"
    local normalized=""
    local token=""
    for token in $(printf '%s' "$raw" | tr ',/' ' '); do
        token="$(printf '%s' "$token" | tr -d '[:space:]')"
        case "$token" in
            1|2|3|4|5)
                if [[ " $normalized " != *" $token "* ]]; then
                    normalized="${normalized} ${token}"
                fi
                ;;
            *)
                ;;
        esac
    done
    printf '%s\n' "${normalized# }"
}

evaluate_iteration_log() {
    local log_file="$1"
    local selected_ids="$2"
    local selected_count="$3"
    local reasons=()

    if [ ! -f "$log_file" ]; then
        reasons+=("missing_log_file")
    fi

    if grep -Eiq "focus_recovery_failed|cmd_n_loop_guard_block" "$log_file"; then
        reasons+=("focus_or_cmdn_loop_detected")
    fi

    local bad_mail_lines=""
    bad_mail_lines="$(grep -E 'MAIL_SEND_PROOF\|status=' "$log_file" 2>/dev/null | grep -Ev 'MAIL_SEND_PROOF\|status=sent_confirmed\|' || true)"
    if [ -n "$bad_mail_lines" ]; then
        reasons+=("mail_send_non_confirmed_status")
    fi

    local sent_count=0
    sent_count="$(grep -Ec 'MAIL_SEND_PROOF\|status=sent_confirmed\|' "$log_file" 2>/dev/null || true)"
    if ! [[ "$sent_count" =~ ^[0-9]+$ ]]; then
        sent_count=0
    fi
    if [ "$sent_count" -lt "$selected_count" ]; then
        reasons+=("mail_send_proof_count_lt_selected(${sent_count}<${selected_count})")
    fi

    local id=""
    for id in $selected_ids; do
        local line=""
        line="$(grep -E "RUN_ATTEMPT\\|phase=scenario_${id}_final_judgement\\|" "$log_file" 2>/dev/null | tail -n 1 || true)"
        if [ -z "$line" ]; then
            reasons+=("scenario_${id}_missing_final_judgement")
            continue
        fi

        local status=""
        local details=""
        local semantic_missing=""
        local mail_proof=""
        status="$(printf '%s\n' "$line" | sed -n 's/.*|status=\([^|]*\).*/\1/p')"
        details="$(printf '%s\n' "$line" | sed -n 's/.*|details=\(.*\)|ts=.*/\1/p')"
        semantic_missing="$(printf '%s\n' "$details" | sed -n 's/.*semantic_missing=\([^,]*\).*/\1/p')"
        mail_proof="$(printf '%s\n' "$details" | sed -n 's/.*mail_proof=\([^,]*\).*/\1/p')"

        if [ "$status" != "success" ]; then
            reasons+=("scenario_${id}_status_${status:-unknown}")
        fi
        if [ "$semantic_missing" != "0" ]; then
            reasons+=("scenario_${id}_semantic_missing_${semantic_missing:-unknown}")
        fi
        if [ "$mail_proof" != "sent_confirmed" ]; then
            reasons+=("scenario_${id}_mail_proof_${mail_proof:-unknown}")
        fi
    done

    if [ "${#reasons[@]}" -gt 0 ]; then
        printf '%s\n' "${reasons[@]}"
        return 1
    fi
    printf '%s\n' "ok"
    return 0
}

SELECTED_SCENARIO_IDS="$(normalize_scenario_ids "$SCENARIOS")"
if [ -z "$SELECTED_SCENARIO_IDS" ]; then
    echo "❌ invalid STEER_GUI_REG_SCENARIOS=${SCENARIOS} (allowed: 1,2,3,4,5)"
    exit 1
fi
SELECTED_SCENARIO_COUNT="$(printf '%s\n' "$SELECTED_SCENARIO_IDS" | wc -w | tr -d ' ')"
SCENARIOS_CSV="$(printf '%s\n' "$SELECTED_SCENARIO_IDS" | tr ' ' ',')"

echo "🧪 GUI regression pack start"
echo " - repeat: ${REPEAT}"
echo " - scenarios: ${SCENARIOS_CSV}"
echo " - output: ${OUT_DIR}"
echo " - mode: approve-assumed + mock external integrations"
echo " - quality gate: scenario final judgement + mail send proof + focus/cmd+n guard"

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
    STEER_EXEC_FOCUS_HANDOFF=1 \
    STEER_EXEC_FOCUS_HANDOFF_RETRIES=3 \
    STEER_EXEC_FOCUS_HANDOFF_FINDER_BRIDGE=1 \
    STEER_AX_SNAPSHOT_STRICT=1 \
    STEER_PREFLIGHT_AX_SNAPSHOT=1 \
    STEER_SEMANTIC_REQUIRE_RUST_CONTRACT=1 \
    STEER_SEMANTIC_REQUIRE_NONEMPTY=1 \
    STEER_SEMANTIC_FAIL_ON_TRUNCATION=1 \
    STEER_INPUT_GUARD_MAX_PAUSES=20 \
    STEER_INPUT_GUARD_MAX_PAUSE_SECONDS=180 \
    STEER_SCENARIO_IDS="${SCENARIOS_CSV}" \
    bash run_complex_scenarios.sh >"${ITER_LOG}" 2>&1
    EXIT_CODE=$?
    set -e

    if [ "$EXIT_CODE" -eq 0 ]; then
        QUALITY_REPORT=""
        if QUALITY_REPORT="$(evaluate_iteration_log "${ITER_LOG}" "${SELECTED_SCENARIO_IDS}" "${SELECTED_SCENARIO_COUNT}")"; then
            echo "✅ iteration ${i}: pass"
            PASS_COUNT=$((PASS_COUNT + 1))
            printf 'PASS|%s|quality=ok\n' "${EXIT_CODE}" > "${ITER_STATUS}"
        else
            echo "❌ iteration ${i}: fail (quality gate)"
            echo "   - ${QUALITY_REPORT}" | tr '\n' ' '
            FAIL_COUNT=$((FAIL_COUNT + 1))
            printf 'FAIL|%s|quality=%s\n' "${EXIT_CODE}" "$(printf '%s' "${QUALITY_REPORT}" | tr '\n' ',' | sed 's/,$//')" > "${ITER_STATUS}"
        fi
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
    echo "scenarios=${SCENARIOS_CSV}"
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
        IFS='|' read -r STATUS_KIND EXIT_CODE STATUS_META <<< "$STATUS_LINE"
        STATUS_KIND="${STATUS_KIND:-FAIL}"
        EXIT_CODE="${EXIT_CODE:-999}"
        STATUS_META="${STATUS_META:-}"
        if [ "${STATUS_KIND}" = "PASS" ]; then
            echo "  <testcase name=\"iteration_${i}\"/>"
        else
            echo "  <testcase name=\"iteration_${i}\">"
            if [ -n "$STATUS_META" ]; then
                echo "    <failure message=\"exit=${EXIT_CODE};${STATUS_META}\"><![CDATA["
            else
                echo "    <failure message=\"exit=${EXIT_CODE}\"><![CDATA["
            fi
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
