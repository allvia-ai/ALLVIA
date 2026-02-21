#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CORE_DIR="${ROOT_DIR}/core"
BIN="${CORE_DIR}/target/debug/local_os_agent"
OUT_DIR="${OUT_DIR:-/tmp/steer_run_scope_validation}"
mkdir -p "${OUT_DIR}"

GOAL1='메모장 열어서 박대엽이라고 써줘 마지막 줄에 "RUN_SCOPE_TEST_01"를 정확히 입력하세요.'
GOAL2='사파리 열어서 구글에서 요즘 유행하는 AI 기사 검색해줘. 그리고 그 중에 3개 기사 찾아서 LLM으로 내용 요약한 다음에 노션에 저장해줘 마지막 줄에 "RUN_SCOPE_TEST_02"를 정확히 입력하세요.'
GOAL3='n8n 접속해서 추천 기능 테스트 실행해줘 워크플로 캔버스에 "RUN_SCOPE_TEST_03" 타이핑해줘'

COMMON_ENV=(
  STEER_FORCE_DETERMINISTIC_GOAL_AUTOPLAN=1
  STEER_DETERMINISTIC_GOAL_AUTOPLAN=1
  STEER_ACTION_IDEMPOTENCY=0
  STEER_TEST_MODE=1
  STEER_ALLOW_DETERMINISTIC_FALLBACK=1
  STEER_REQUIRE_PRIMARY_PLANNER=0
  STEER_MAX_STEPS=14
  STEER_PLANNER_PLAN_TIMEOUT_SEC=15
  STEER_PLANNER_SUPERVISOR_TIMEOUT_SEC=4
  STEER_SUPERVISOR_BYPASS_SAFE=1
)

echo "[build] cargo build --bin local_os_agent"
(
  cd "${CORE_DIR}"
  cargo build --bin local_os_agent >/dev/null
)

run_goal() {
  local goal="$1"
  local log="$2"
  (
    cd "${CORE_DIR}"
    env "${COMMON_ENV[@]}" "${BIN}" surf "${goal}" >"${log}" 2>&1 || true
  )
}

extract_run_id() {
  local log="$1"
  rg -o 'run_id=[^ ]+' "${log}" | tail -n1 | cut -d= -f2 || true
}

extract_session_id() {
  local log="$1"
  rg -o 'Saved: [a-f0-9-]+' "${log}" | tail -n1 | awk '{print $2}' || true
}

note_scope_check() {
  osascript <<'APPLESCRIPT'
tell application "Notes"
  set hits to every note whose body contains "RUN_SCOPE_TEST_01"
  if (count of hits) is 0 then return "NOT_FOUND"
  repeat with n in hits
    set b to body of n
    if b contains "박대엽" then
      return "FOUND_OK||" & (name of n)
    end if
  end repeat
  return "FOUND_NO_NAME"
end tell
APPLESCRIPT
}

note_scope_dump() {
  osascript <<'APPLESCRIPT'
tell application "Notes"
  set hits to every note whose body contains "RUN_SCOPE_TEST_01"
  if (count of hits) is 0 then return "NOT_FOUND"
  set latestNote to item 1 of hits
  repeat with n in hits
    try
      if (modification date of latestNote) < (modification date of n) then
        set latestNote to n
      end if
    end try
  end repeat
  set noteName to ""
  set noteBody to ""
  try
    set noteName to name of latestNote as text
  end try
  try
    set noteBody to body of latestNote as text
  end try
  return "NAME=" & noteName & linefeed & "BODY=" & noteBody
end tell
APPLESCRIPT
}

focus_latest_scope_note() {
  osascript <<'APPLESCRIPT'
tell application "Notes"
  activate
  set hits to every note whose body contains "RUN_SCOPE_TEST_01"
  if (count of hits) = 0 then return "NOT_FOUND"
  set latestNote to item 1 of hits
  repeat with n in hits
    try
      if (modification date of latestNote) < (modification date of n) then
        set latestNote to n
      end if
    end try
  end repeat
  show latestNote
end tell
delay 1
APPLESCRIPT
}

focus_notion_and_find_marker() {
  osascript <<'APPLESCRIPT'
tell application "System Events"
  if exists (application process "Notion") then
    tell application "Notion" to activate
  else if exists (application process "ChatGPT Atlas") then
    tell application "ChatGPT Atlas" to activate
  else if exists (application process "Safari") then
    tell application "Safari" to activate
  end if
end tell
delay 0.8
tell application "System Events"
  keystroke "f" using {command down}
  delay 0.2
  keystroke "RUN_SCOPE_TEST_02"
  delay 0.8
end tell
APPLESCRIPT
}

capture_frontmost() {
  local path="$1"
  screencapture -x "${path}" >/dev/null 2>&1 || true
}

capture_text_file_proof() {
  local src="$1"
  local out="$2"
  osascript <<APPLESCRIPT >/dev/null 2>&1 || true
tell application "TextEdit"
  activate
  open POSIX file "${src}"
end tell
delay 0.9
APPLESCRIPT
  screencapture -x "${out}" >/dev/null 2>&1 || true
  osascript <<'APPLESCRIPT' >/dev/null 2>&1 || true
tell application "TextEdit"
  try
    close front window saving no
  end try
end tell
APPLESCRIPT
}

echo "[step1] notes typing"
STEP1_PASS=0
STEP1_RUNID=""
STEP1_CHECK="NOT_FOUND"
for attempt in 1 2 3 4; do
  LOG="${OUT_DIR}/step1_attempt_${attempt}.log"
  run_goal "${GOAL1}" "${LOG}"
  STEP1_RUNID="$(extract_run_id "${LOG}")"
  STEP1_CHECK="$(note_scope_check || true)"
  echo "[step1] attempt=${attempt} check=${STEP1_CHECK}"
  if [[ "${STEP1_CHECK}" == FOUND_OK* ]]; then
    STEP1_PASS=1
    break
  fi
  sleep 1
done
STEP1_NOTE_DUMP="$(note_scope_dump || true)"
printf '%s\n' "${STEP1_NOTE_DUMP}" > "${OUT_DIR}/step1_note_body.txt"
focus_latest_scope_note >/dev/null 2>&1 || true
capture_frontmost "${OUT_DIR}/step1_after.png"

echo "[step2] ai news -> notion"
STEP2_LOG="${OUT_DIR}/step2.log"
run_goal "${GOAL2}" "${STEP2_LOG}"
STEP2_RUNID="$(extract_run_id "${STEP2_LOG}")"
STEP2_SESSION="$(extract_session_id "${STEP2_LOG}")"
STEP2_MARKER="none"
if [[ -n "${STEP2_SESSION}" ]] && [[ -f "${HOME}/.steer/sessions/${STEP2_SESSION}.json" ]]; then
  STEP2_MARKER="$(
    jq -r '.steps[]?.description' "${HOME}/.steer/sessions/${STEP2_SESSION}.json" \
      | rg -n 'RUN_SCOPE_TEST_02' -m1 || true
  )"
  STEP2_TYPED_PAYLOAD="$(
    jq -r '.steps[]? | select((.description // "") | contains("RUN_SCOPE_TEST_02")) | .description' \
      "${HOME}/.steer/sessions/${STEP2_SESSION}.json" | head -n 80
  )"
else
  STEP2_TYPED_PAYLOAD="none"
fi
STEP2_FRONTMOST="$(
  osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null || true
)"
printf '%s\n' "${STEP2_TYPED_PAYLOAD}" > "${OUT_DIR}/step2_typed_payload.txt"
capture_text_file_proof "${OUT_DIR}/step2_typed_payload.txt" "${OUT_DIR}/step2_typed_payload.png"
focus_notion_and_find_marker >/dev/null 2>&1 || true
capture_frontmost "${OUT_DIR}/step2_after.png"

echo "[step3] n8n prompt path"
STEP3_LOG="${OUT_DIR}/step3.log"
run_goal "${GOAL3}" "${STEP3_LOG}"
STEP3_RUNID="$(extract_run_id "${STEP3_LOG}")"
STEP3_SESSION="$(extract_session_id "${STEP3_LOG}")"
STEP3_MARKER="none"
if [[ -n "${STEP3_SESSION}" ]] && [[ -f "${HOME}/.steer/sessions/${STEP3_SESSION}.json" ]]; then
  STEP3_MARKER="$(
    jq -r '.steps[]?.description' "${HOME}/.steer/sessions/${STEP3_SESSION}.json" \
      | rg -n "Opened URL 'http://localhost:5678/workflow/new'|Opened URL 'http://localhost:5678/'|Typed 'RUN_SCOPE_TEST_03'" || true
  )"
  STEP3_TYPED_PAYLOAD="$(
    jq -r '.steps[]? | select((.description // "") | contains("RUN_SCOPE_TEST_03")) | .description' \
      "${HOME}/.steer/sessions/${STEP3_SESSION}.json"
  )"
else
  STEP3_TYPED_PAYLOAD="none"
fi
printf '%s\n' "${STEP3_TYPED_PAYLOAD}" > "${OUT_DIR}/step3_typed_payload.txt"
capture_text_file_proof "${OUT_DIR}/step3_typed_payload.txt" "${OUT_DIR}/step3_typed_payload.png"
capture_frontmost "${OUT_DIR}/step3_after.png"

echo "[step3-api] recommendation approve -> workflow exists"
PORT=15680
if lsof -tiTCP:${PORT} -sTCP:LISTEN >/dev/null 2>&1; then
  lsof -tiTCP:${PORT} -sTCP:LISTEN | xargs kill -9 || true
fi
N8N_KEY="$(
  sqlite3 "${HOME}/.n8n/database.sqlite" \
    "SELECT apiKey FROM user_api_keys ORDER BY createdAt DESC LIMIT 1;" \
    | tr -d '\r\n'
)"

(
  cd "${CORE_DIR}"
  env \
    STEER_API_PORT="${PORT}" \
    STEER_LOCK_SCOPE="codex-${PORT}" \
    STEER_API_ALLOW_NO_KEY=1 \
    STEER_DISABLE_EVENT_TAP=1 \
    STEER_PREFLIGHT_SCREEN_CAPTURE=1 \
    STEER_PREFLIGHT_AX_SNAPSHOT=1 \
    STEER_TEST_MODE=1 \
    STEER_CLI_LLM=disabled \
    STEER_LLM_FALLBACK_CHAIN=local,codex \
    STEER_N8N_MINIMAL_ON_LLM_FAILURE=1 \
    N8N_API_KEY="${N8N_KEY}" \
    "${BIN}" >"${OUT_DIR}/api_server.log" 2>&1
) &
API_PID=$!

for _ in {1..35}; do
  if curl -fsS --max-time 2 "http://127.0.0.1:${PORT}/api/system/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

curl -sS --max-time 30 -X POST "http://127.0.0.1:${PORT}/api/patterns/analyze" >/dev/null || true
ALL="$(curl -sS --max-time 20 "http://127.0.0.1:${PORT}/api/recommendations?status=all" || echo '[]')"
RID="$(echo "${ALL}" | jq -r '[.[] | select(.status=="pending") | .id] | max // empty')"
if [[ -z "${RID}" ]]; then
  RID="$(echo "${ALL}" | jq -r '[.[] | .id] | max // empty')"
fi
AP="$(curl -sS --max-time 180 -X POST "http://127.0.0.1:${PORT}/api/recommendations/${RID}/approve" || true)"
WFID="$(echo "${AP}" | jq -r '.workflow_id // .id // empty' 2>/dev/null || true)"
WF="none"
if [[ -n "${WFID}" ]]; then
  WF="$(
    curl -sS --max-time 20 -H "X-N8N-API-KEY: ${N8N_KEY}" \
      "http://localhost:5678/api/v1/workflows/${WFID}" \
      | jq -c '{id,name,createdAt,updatedAt,active,isArchived}'
  )"
fi
echo "${AP}" | jq . > "${OUT_DIR}/step3_approve_response.json" 2>/dev/null || printf '%s\n' "${AP}" > "${OUT_DIR}/step3_approve_response.json"
printf '%s\n' "${WF}" > "${OUT_DIR}/step3_workflow_check.json"
capture_text_file_proof "${OUT_DIR}/step3_workflow_check.json" "${OUT_DIR}/step3_workflow_check.png"
kill "${API_PID}" >/dev/null 2>&1 || true

echo "STEP1_PASS=${STEP1_PASS}"
echo "STEP1_CHECK=${STEP1_CHECK}"
echo "STEP1_RUNID=${STEP1_RUNID}"
echo "STEP2_RUNID=${STEP2_RUNID}"
echo "STEP2_MARKER=${STEP2_MARKER}"
echo "STEP2_FRONTMOST=${STEP2_FRONTMOST}"
echo "STEP3_RUNID=${STEP3_RUNID}"
echo "STEP3_MARKER=${STEP3_MARKER}"
echo "STEP3_APPROVE=$(echo "${AP}" | jq -c . 2>/dev/null || echo '{}')"
echo "STEP3_WORKFLOW=${WF}"
echo "OUT_DIR=${OUT_DIR}"
