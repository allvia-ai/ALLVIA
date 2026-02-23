#!/usr/bin/env bash
set -euo pipefail

API_URL="${STEER_API_URL:-http://127.0.0.1:5680}"
DB_PATH="${STEER_DB_PATH:-$HOME/Library/Application Support/steer/steer.db}"
OUT_FILE="${1:-/tmp/steer_dynamic_requests_$(date +%Y%m%d_%H%M%S).tsv}"
MAX_TIME_SEC="${STEER_SIM_MAX_TIME_SEC:-35}"
POLL_TIMEOUT_SEC="${STEER_SIM_POLL_TIMEOUT_SEC:-25}"
POLL_INTERVAL_SEC="${STEER_SIM_POLL_INTERVAL_SEC:-1}"
SIM_SUITE="${STEER_SIM_SUITE:-multilingual}"
PROMPTS_FILE="${STEER_SIM_PROMPTS_FILE:-}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required."
  exit 1
fi

if [ ! -f "$DB_PATH" ]; then
  echo "DB not found: $DB_PATH"
  exit 1
fi

sqlq() {
  sqlite3 -cmd ".timeout 5000" "$DB_PATH" "$1"
}

is_in_progress_status() {
  local s="${1:-}"
  case "$s" in
    running|queued|accepted|started|retrying|business_incomplete)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

declare -a PROMPTS=()

load_prompts_from_file() {
  local file="$1"
  while IFS= read -r line || [ -n "$line" ]; do
    # trim
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    if [ -z "$line" ]; then
      continue
    fi
    if [[ "$line" == \#* ]]; then
      continue
    fi
    PROMPTS+=("$line")
  done < "$file"
}

load_default_multilingual_prompts() {
  PROMPTS+=(
    "노트 열어줘"
    "Open Notes app"
    "Abre notas"
    "Ouvre Notes"
    "メモを開いて"
    "打开 Notes"
    "크롬 열어줘"
    "Open Calculator"
    "메모장 열고 'hello dynamic user'라고 써줘"
    "Open Notes and write 'dynamic hello'"
    "Abre notas y escribe 'hola dinamica'"
    "Ouvre Notes et écris 'bonjour dynamique'"
    "メモを開いて '動的テスト' と書いて"
    "오늘 할 일 5개 정리해줘"
    "Sports news 5 headlines summary"
    "今日のニュースを3つ要約して"
    "Résume les e-mails importants du jour"
    "Resúmeme las tareas más urgentes de hoy"
  )
}

load_varied_prompts() {
  PROMPTS+=(
    "크롬 열어줘"
    "노트 열고 오늘 할 일 5개 써줘"
    "캘린더 열고 오늘 일정 요약해줘"
    "메일 열고 중요 메일 3개 요약해줘"
    "스포츠 뉴스 5개 요약해줘"
    "AI 뉴스 5개 요약해줘"
    "파인더 열고 다운로드 폴더 보여줘"
    "디스코드 열어줘"
    "오늘 업무 우선순위 3개 정리해줘"
    "회의 준비 체크리스트 7개 만들어줘"
    "노트에 프로젝트 리스크 5개 적어줘"
    "오늘 받은 메일 중 답장 필요한 것만 뽑아줘"
    "유튜브에서 rust async 튜토리얼 찾아줘"
    "내일 일정 준비물 목록 만들어줘"
    "텍스트 요약 모드로 긴 문장 3개 축약해줘"
    "업무 자동화 아이디어 5개 제안해줘"
    "코딩 시작 전에 해야 할 준비 5단계 알려줘"
    "오늘 마감 작업만 따로 목록으로 정리해줘"
  )
}

if [ -n "$PROMPTS_FILE" ]; then
  if [ ! -f "$PROMPTS_FILE" ]; then
    echo "Prompts file not found: $PROMPTS_FILE"
    exit 1
  fi
  load_prompts_from_file "$PROMPTS_FILE"
elif [ "$SIM_SUITE" = "varied" ]; then
  load_varied_prompts
else
  load_default_multilingual_prompts
fi

if [ "${#PROMPTS[@]}" -eq 0 ]; then
  echo "No prompts to simulate."
  exit 1
fi

echo -e "idx\tprompt\thttp_code\tresponse_ms\trun_id\tapi_status\taccepted\tfinal_status\tfinal_wait_sec\ttimeout_hits\tprimary_failed_markers\trecovery_markers" >"$OUT_FILE"

idx=0
for prompt in "${PROMPTS[@]}"; do
  idx=$((idx + 1))
  payload="$(jq -nc --arg g "$prompt" '{goal:$g}')"
  run_id=""
  api_status="curl_or_parse_error"
  http_code="000"
  response_ms=""
  accepted="false"
  final_status=""
  final_wait_sec="0"
  timeout_hits=0
  primary_failed_markers=0
  recovery_markers=0
  tmp_body="$(mktemp /tmp/steer_sim_body.XXXXXX)"

  for attempt in 1 2; do
    curl_meta="$(curl --max-time "$MAX_TIME_SEC" -sS \
      -o "$tmp_body" -w "%{http_code}\t%{time_total}" \
      -X POST "$API_URL/api/agent/goal/run" \
      -H 'Content-Type: application/json' \
      -d "$payload" || true)"
    resp="$(cat "$tmp_body" 2>/dev/null || true)"
    http_code="$(printf '%s' "$curl_meta" | awk -F '\t' '{print $1}')"
    response_sec="$(printf '%s' "$curl_meta" | awk -F '\t' '{print $2}')"
    if [ -z "$http_code" ]; then
      http_code="000"
    fi
    if [ -n "${response_sec:-}" ]; then
      response_ms="$(awk -v s="$response_sec" 'BEGIN {printf("%.0f", s*1000)}')"
    else
      response_ms=""
    fi
    run_id="$(printf '%s' "$resp" | jq -r '.run_id // empty' 2>/dev/null || true)"
    api_status="$(printf '%s' "$resp" | jq -r '.status // .error // "curl_or_parse_error"' 2>/dev/null || echo "curl_or_parse_error")"
    if [ "$api_status" = "accepted" ]; then
      accepted="true"
    fi
    if [ -n "$run_id" ] || [ "$api_status" != "curl_or_parse_error" ]; then
      break
    fi
    sleep 1
  done

  if [ -n "$run_id" ]; then
    elapsed=0
    final_status="$(sqlq "select status from task_runs where run_id='$run_id' limit 1;" || true)"
    while [ "$elapsed" -lt "$POLL_TIMEOUT_SEC" ]; do
      if [ -n "$final_status" ] && ! is_in_progress_status "$final_status"; then
        break
      fi
      sleep "$POLL_INTERVAL_SEC"
      elapsed=$((elapsed + POLL_INTERVAL_SEC))
      final_status="$(sqlq "select status from task_runs where run_id='$run_id' limit 1;" || true)"
    done
    final_wait_sec="$elapsed"
    if [ -z "$final_status" ]; then
      final_status="missing"
    elif is_in_progress_status "$final_status"; then
      final_status="running_timeout"
    fi
    timeout_hits="$(sqlq "select count(*) from task_stage_runs where run_id='$run_id' and lower(coalesce(details,'')) like '%timeout%';")"
    primary_failed_markers="$(sqlq "select count(*) from task_stage_runs where run_id='$run_id' and details like '%PLAN_PRIMARY_FAILED%';")"
    recovery_markers="$(sqlq "select count(*) from task_stage_runs where run_id='$run_id' and details like '%PLAN_RECOVERY_ACTION%';")"
  else
    final_status="$api_status"
  fi

  echo -e "${idx}\t${prompt}\t${http_code}\t${response_ms}\t${run_id}\t${api_status}\t${accepted}\t${final_status}\t${final_wait_sec}\t${timeout_hits}\t${primary_failed_markers}\t${recovery_markers}" >>"$OUT_FILE"
  printf '[%d] %s => http=%s status=%s run=%s final=%s\n' "$idx" "$prompt" "$http_code" "$api_status" "$run_id" "$final_status"
  rm -f "$tmp_body"
done

echo "Saved: $OUT_FILE"
cat "$OUT_FILE"
