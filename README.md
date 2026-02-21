# Local OS Agent (Rust Native)

**사용자 행동 기반 자동화 에이전트** - 컴퓨터 사용 패턴을 분석하여 자동화를 추천하고 실행합니다.

[![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust)](https://www.rust-lang.org/)
[![macOS](https://img.shields.io/badge/macOS-000000?style=flat&logo=apple)](https://www.apple.com/macos/)

## ✨ 주요 기능

| 기능 | 명령어 | 설명 |
|:---|:---|:---|
| Shadow | (자동) | 백그라운드 행동 데이터 수집 |
| Routine | `routine` | 일일 루틴 분석 |
| Recommend | `recommend` | 자동화 스크립트 제안 |
| Control | `control <app> <cmd>` | 앱 내부 제어 |
| Workflow | `build_workflow <prompt>` | n8n 자동화 생성 |
| Exec | `exec <cmd>` | 셸 명령 실행 |
| Status | `status` | 시스템 리소스 확인 |

## 🚀 설치

```bash
# 1. Clone
git clone <repo_url>
cd local-os-agent/core

# 2. 환경변수 설정
cp .env.example .env
# .env 파일에 OPENAI_API_KEY 입력

# 3. 빌드
cargo build --release

# 4. 실행 (Accessibility 권한 필요)
export STEER_API_ALLOW_NO_KEY=1
./target/release/local_os_agent

# UI 데모 모드(백그라운드 간섭 최소화)
STEER_API_ALLOW_NO_KEY=1 STEER_DISABLE_EVENT_TAP=1 ./target/release/local_os_agent

# 복구 + 백그라운드 재기동(권장)
./scripts/recover_runtime.sh
```

## 📦 Release

To build a production-ready application (binary/bundle):

```bash
./scripts/rebuild_and_deploy.sh
```

This script automates:
1.  **Core Build**: Builds `core/target/release/local_os_agent`.
2.  **Sidecar Sync**: Copies the latest server binary into `web/src-tauri/binaries/core-aarch64-apple-darwin`.
3.  **Bundle Build**: Runs `npm run tauri build` from `web/`.
4.  **Clean Deploy**: Replaces `/Applications/Steer OS.app` and verifies API health.

Runbook: `docs/BUILD_DEPLOY_RUNBOOK.md`

Fast dev loop (no packaging during iteration):

```bash
./scripts/validate_core_cli.sh --goal "메모장 열어서 박대엽이라고 써줘"
# then only once at the end:
./scripts/rebuild_and_deploy.sh
```

## 🛡️ Self-Healing
The agent includes a supervisor script to ensure high availability:

```bash
./scripts/steer-guardian.sh
```
This restarts the core process automatically if a crash occurs.

## 📋 필수 요구사항

- **macOS 12+** (Monterey 이상)
- **Accessibility 권한**: 시스템 환경설정 → 개인정보 보호 → 손쉬운 사용 → 터미널 체크
- **Rust 1.70+**
- **OpenAI API Key** (LLM 분석용)

## 🐳 n8n 실행 모드 (macOS 권장)

기본 런타임은 Docker입니다.

- 기본값: `STEER_N8N_RUNTIME=docker`
- 대체값: `STEER_N8N_RUNTIME=npx` 또는 `STEER_N8N_RUNTIME=manual`
- Docker 자동기동: `STEER_N8N_AUTO_START=1` (docker 모드 기본값)
- Docker compose 파일 경로 override: `STEER_N8N_COMPOSE_FILE=/abs/path/docker-compose.yml`
- Docker 모드에서는 CLI fallback 기본 비활성 (`STEER_N8N_ALLOW_CLI_FALLBACK=0`)
- `npx --tunnel`은 테스트/CI 전용 기본 정책입니다.
  - 사용: `STEER_N8N_USE_TUNNEL=1`
  - 비테스트 허용(명시 opt-in): `STEER_N8N_ALLOW_NPX_TUNNEL_NON_TEST=1`

빠른 시작:

```bash
docker compose up -d n8n
# n8n API URL 기본값: http://localhost:5678/api/v1
```

## 🛡️ 보안

- `exec` 명령어는 위험한 키워드(`rm`, `sudo` 등)가 포함되면 차단됩니다.
- 기본적으로 **Write Lock**이 활성화되어 있습니다. `unlock` 명령어로 해제하세요.

## 📂 프로젝트 구조

```
core/src/
├── main.rs          # CLI 및 메인 루프
├── analyzer.rs      # 행동 패턴 분석기
├── db.rs            # SQLite 저장소
├── policy.rs        # 보안 정책 엔진
├── executor.rs      # 셸 명령 실행
├── llm_gateway.rs   # OpenAI 연동
├── notifier.rs      # macOS 알림
├── monitor.rs       # 시스템 모니터링
├── applescript.rs   # 앱 제어
├── n8n_api.rs       # n8n 워크플로우 API
├── visual_driver.rs # UI 자동화 폴백
└── macos/           # 네이티브 macOS 바인딩
```

## 🧪 테스트

```bash
cargo test
```

GUI 회귀 테스트 팩(승인 가정 + 외부 연동 mock):

```bash
bash run_gui_regression_pack.sh
# 반복 실행 예시
STEER_GUI_REG_PACK_REPEAT=3 bash run_gui_regression_pack.sh
# 시나리오 일부만 실행(예: 1,3,5)
STEER_GUI_REG_SCENARIOS=1,3,5 bash run_gui_regression_pack.sh
```

## 📡 데이터 수집/패턴 모니터링 (steer/jy 병합)

`steer/jy`의 데이터 수집 파이프라인을 현재 저장소에 통합했습니다.

- Collector core: `src/collector/`
- OS sensors: `src/sensors/`
- Runtime config: `configs/config.yaml`, `configs/config_run2.yaml`
- DB migrations: `migrations/*.sql`
- Rust batch bins: `build_sessions_rs`, `build_routines_rs`, `build_handoff_rs`
- Python batch entrypoints(호환 유지, 내부 Rust 라우팅): `scripts/build_sessions.py`, `scripts/build_routines.py`, `scripts/build_handoff.py`
- Legacy Python originals: `scripts/legacy/*.py`

빠른 시작:

```bash
python -m pip install -r requirements-collector.txt
PYTHONPATH=src python scripts/init_db.py --config configs/config.yaml
PYTHONPATH=src python -m collector.main --config configs/config.yaml
```

패턴 분석/모니터링 배치(Rust 권장):

```bash
cargo build --manifest-path core/Cargo.toml \
  --bin build_sessions_rs --bin build_routines_rs --bin build_handoff_rs
STEER_DB_PATH=./steer.db ./core/target/debug/build_sessions_rs --since-hours 6 --use-state
STEER_DB_PATH=./steer.db ./core/target/debug/build_routines_rs --days 3 --min-support 2 --use-state
STEER_DB_PATH=./steer.db ./core/target/debug/build_handoff_rs --skip-unchanged --keep-latest-pending
# 또는 3단계 일괄 실행
bash scripts/run_pipeline_rs.sh configs/config.yaml
```

Python 호환 경로(동일 인자, 내부적으로 Rust 바이너리 호출):

```bash
python scripts/build_sessions.py --since-hours 6 --use-state
python scripts/build_routines.py --days 3 --min-support 2 --use-state
python scripts/build_handoff.py --skip-unchanged --keep-latest-pending
PYTHONPATH=src python scripts/print_stats.py --config configs/config.yaml
```

## 🦀 Rust Collector (권장 기본 경로)

Python collector 대신 Rust 단일 바이너리로 수집/보안필터/집약을 실행할 수 있습니다.

- Binary: `/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/core/src/bin/collector_rs.rs`
- Endpoints: `POST /events`, `GET /health`, `GET /stats`
- Built-in jobs:
  - 시작 시 패턴 워크플로우 생성 (`workflows/workflow_YYYY-MM-DD.json`)
  - 5분 단위 집약 (`minute_aggregates`)
  - 일일 요약 갱신 (`daily_summaries`)
  - retention cleanup (`events_v2`, `minute_aggregates`)

실행:

```bash
cargo build --manifest-path core/Cargo.toml --bin collector_rs
STEER_DB_PATH=./steer.db STEER_COLLECTOR_PORT=8080 ./core/target/debug/collector_rs
# 또는 (기본 Rust 경로)
bash scripts/run_local.sh
```

주요 환경변수:

- `STEER_COLLECTOR_HOST` (default: `127.0.0.1`)
- `STEER_COLLECTOR_PORT` (default: `8080`)
- `STEER_COLLECTOR_AGG_INTERVAL_SEC` (default: `300`)
- `STEER_COLLECTOR_RAW_RETENTION_DAYS` (default: `7`)
- `STEER_COLLECTOR_SUMMARY_RETENTION_DAYS` (default: `30`)
- `STEER_STARTUP_MIN_EVENTS` (default: `100`)
- `STEER_STARTUP_PATTERN_THRESHOLD` (default: `3`)
- `STEER_WORKFLOW_OUTPUT_DIR` (default: `workflows`)
- `STEER_DB_PATH` (collector/pipeline 공통 DB 경로)
- `STEER_PRIVACY_RULES_PATH` (handoff privacy rules 경로 override)
- `STEER_SEMANTIC_REQUIRE_RUST_CONTRACT` (`run_nl_request_with_telegram.sh`에서 의미검증 토큰 추출을 Rust 계약 파서로 강제, 기본 `1`)
- `STEER_SEMANTIC_ALLOW_HEURISTIC_FALLBACK` (Rust 계약 파서 실패 시 휴리스틱 토큰 추출 허용 여부, 기본 `0`)
- `STEER_SEMANTIC_FAIL_ON_TRUNCATION` (의미검증 토큰 상한 절단 시 실패 승격 여부, 기본 `1`)
- `STEER_SEMANTIC_ALLOW_LOG_EVIDENCE` (의미검증에서 로그-only 증거 허용 여부, 기본 `0`)
- `STEER_SEMANTIC_ALLOW_SCENARIO_FALLBACK` (`run_complex_scenarios.sh`에서 Rust 계약 실패 시 시나리오 내장 토큰 fallback 허용, 기본 `0`)
- `STEER_REQUIRE_MAIL_SUBJECT` (메일 성공 판정 시 subject 필수, 기본 `1`)
- `STEER_REQUIRE_MAIL_SEND` (메일 DoD를 강제할지 여부. 기본 `0`; 시나리오 계약/요청문에서 메일 발송이 감지되면 자동으로 활성화)
- `STEER_REQUIRE_SENT_MAILBOX_EVIDENCE` (메일 성공 판정 시 sent mailbox 증거 필수, 기본 `1`)
- `STEER_INPUT_GUARD_MAX_PAUSES` (입력 가드 최대 pause 횟수, 기본 `40`)
- `STEER_INPUT_GUARD_MAX_PAUSE_SECONDS` (입력 가드 누적 pause 허용 시간(초), 기본 `300`)
- `STEER_SCENARIO_IDS` (`run_complex_scenarios.sh` 실행 시 대상 시나리오 선택, 예: `1,3,5`, 기본 `1,2,3,4,5`)
- `STEER_GUI_REG_SCENARIOS` (`run_gui_regression_pack.sh`에서 반복 실행할 시나리오 집합, 기본 `1,2,3,4,5`)
- `STEER_FOCUS_RECOVERY_MAX_RETRIES` (UI 액션 후 타깃 앱 포커스 복구 재시도 횟수, 기본 `2`)
- `STEER_FOCUS_RECOVERY_PROFILE` (포커스 복구 전략: `standard|aggressive`, 기본 `standard`)
- `STEER_EXEC_FOCUS_HANDOFF` (`execution_controller` 단계 전 포커스 이탈 자동 복구 사용 여부, 기본 `1`)
- `STEER_EXEC_FOCUS_HANDOFF_RETRIES` (`execution_controller` 포커스 자동 복구 재시도 횟수, 기본 `2`, 범위 `1..6`)
- `STEER_EXEC_FOCUS_HANDOFF_RETRY_MS` (`execution_controller` 복구 재시도 간격(ms), 기본 `220`, 범위 `80..1200`)
- `STEER_EXEC_FOCUS_HANDOFF_FINDER_BRIDGE` (복구 시 Finder handoff bridge 사용 여부, 기본 `1`)
- `STEER_AX_SNAPSHOT_STRICT` (접근성 snapshot에서 focused app/window 누락 시 fail-closed 여부; 기본: `STEER_SCENARIO_MODE=1` 또는 `STEER_TEST_MODE=1`일 때 `1`)
- `STEER_AX_SNAPSHOT_FOCUS_RETRIES` (접근성 snapshot focus 재시도 횟수, 기본 `2`, 범위 `0..8`)
- `STEER_AX_SNAPSHOT_RETRY_MS` (접근성 snapshot 재시도 간격(ms), 기본 `120`, 범위 `20..2000`)
- `STEER_BROWSER_SNAPSHOT_FOCUS_RECOVERY` (브라우저 snapshot 0-elements 시 브라우저 앱 자동 활성화 복구 사용 여부, 기본 `1`)
- `STEER_BROWSER_SNAPSHOT_RETRIES` (브라우저 snapshot 재시도 횟수, 기본 `2`, 범위 `0..8`)
- `STEER_BROWSER_SNAPSHOT_RETRY_MS` (브라우저 snapshot 재시도 간격(ms), 기본 `160`, 범위 `30..2000`)
- `STEER_BROWSER_SNAPSHOT_RECOVERY_APPS` (snapshot 복구 시 활성화할 브라우저 후보 목록, CSV, 기본 `Safari,Google Chrome,Arc`)
- `STEER_DISABLE_DOWNLOAD_WATCHER` (`1`이면 core의 Downloads 파일 감시 비활성; 기본 `0`)
- `STEER_DOWNLOADS_DIR` (core 파일 감시 대상 경로 override, 미설정 시 `~/Downloads`)
- `STEER_DISABLE_APP_WATCHER` (`1`이면 core의 전면 앱 감시 비활성; 기본 `0`)
- `STEER_STRICT_ACTION_ERRORS` (`1`이면 모든 액션의 `status!=success`를 즉시 실행 오류로 승격; 기본 `0`)
- `STEER_ABORT_ON_EXECUTION_ERROR` (플래너 루프에서 `Critical ... failed` 액션 오류를 즉시 중단/실패로 승격; 기본 `1`)
- `STEER_OUTBOUND_MAIL_STRICT` (메일 발송 시 아웃바운드 정책 검사 활성화, 기본 `1`)
- `STEER_OUTBOUND_MAIL_REQUIRE_SINGLE_RECIPIENT` (메일 발송 대상 단일 수신자 강제, 기본 `1`)
- `STEER_OUTBOUND_MAIL_REQUIRE_GOAL_TARGET_MATCH` (요청문 내 수신자와 실제 발송 수신자 일치 강제, 기본 `1`)
- `STEER_OUTBOUND_MAIL_REQUIRE_BODY` (메일 본문 길이 >0 강제, 기본 `1`)
- `STEER_OUTBOUND_MAIL_REQUIRE_SENT_CONFIRMED` (`sent_confirmed` 상태만 성공으로 인정, 기본 `1`)
- `STEER_OUTBOUND_MAIL_REQUIRE_SUBJECT` (메일 제목 비어있음 금지, 기본 `1`)
- `STEER_PREFLIGHT_FOCUS_HANDOFF` (실행 전 Finder 전면 전환 검증으로 포커스 소유권 확인; 기본 `1`)
- `STEER_PREFLIGHT_AX_SNAPSHOT` (`/api/agent/preflight`에서 native accessibility snapshot 점검 포함 여부; 기본 `1`)
- `STEER_AX_SNAPSHOT_FOCUS_RETRIES` (AX snapshot focused app 재시도 횟수, 기본 `2`)
- `STEER_AX_SNAPSHOT_RETRY_MS` (AX snapshot focused app 재시도 간격(ms), 기본 `120`)
- `STEER_AX_SNAPSHOT_WINDOW_RETRIES` (AX snapshot focused window 재시도 횟수, 기본 `3`)
- `STEER_AX_SNAPSHOT_WINDOW_RETRY_MS` (AX snapshot focused window 재시도 간격(ms), 기본 `90`)
- `STEER_AX_SNAPSHOT_FALLBACK_APP` (focused app/window 미검출 시 activate 시도 앱명, 기본 `Finder`)
- `STEER_PREFLIGHT_FOCUS_RETRIES` (preflight Finder focus handoff 재시도 횟수, 기본 `3`)
- `STEER_PREFLIGHT_FOCUS_RETRY_SLEEP_SEC` (preflight focus handoff 재시도 간격(초), 기본 `0.25`)
- `STEER_COMPLETION_SCORE_PASS` (`/api/agent/execute` 완성도 점수 pass 임계값(0~100), 기본 `75`)
- `STEER_ALLOW_COLLECTOR_DB_MISMATCH` (`workflow_intake`에서 collector/core DB 불일치 허용, 기본 `0`)
- `STEER_APPROVAL_ASK_FALLBACK` (승인 대기 시 실행 fallback 정책: `ask|deny|allow-once`, 시나리오 스크립트 기본 `deny`)
- `STEER_APPROVAL_ALLOW_ONCE_NON_TEST` (`STEER_APPROVAL_ASK_FALLBACK=allow-once`를 테스트/CI 외 환경에서 허용할지 여부, 기본 `0`)
- `STEER_N8N_ALLOW_NPX_CLI` (`n8n` CLI 바이너리가 없을 때 `npx -y n8n` CLI fallback 허용, 기본 `0`)
- `STEER_N8N_ALLOW_NPX_CLI_NON_TEST` (`STEER_N8N_ALLOW_NPX_CLI=1`을 테스트/CI 외에서 허용할지 여부, 기본 `0`)
- `STEER_N8N_ALLOW_NPX_CLI_REMOTE` (remote `N8N_API_URL`에서 `npx` CLI fallback 허용 여부, 기본 `0`)
- `STEER_N8N_ALLOW_NPX_NON_TEST` (`STEER_N8N_RUNTIME=npx`를 테스트/CI 외 환경에서 강제 허용할지 여부, 기본 `0`)
- `STEER_N8N_USE_TUNNEL` (`npx` 런타임에서 `--tunnel` 사용 여부, 기본 `0`)
- `STEER_N8N_ALLOW_NPX_TUNNEL_NON_TEST` (`STEER_N8N_USE_TUNNEL=1`을 테스트/CI 외 환경에서 허용할지 여부, 기본 `0`)
- `STEER_N8N_HTTP_RETRY_ATTEMPTS` (n8n API HTTP 재시도 횟수, 기본 `4`)
- `STEER_N8N_HTTP_RETRY_MIN_BACKOFF_MS` (n8n API HTTP 최소 backoff(ms), 기본 `400`)
- `STEER_N8N_HTTP_RETRY_MAX_BACKOFF_MS` (n8n API HTTP 최대 backoff(ms), 기본 `10000`)
- `STEER_N8N_HTTP_RETRY_JITTER` (n8n API HTTP 재시도 지터 비율 `0.0~0.5`, 기본 `0.1`)
- `STEER_TELEGRAM_RETRY_ATTEMPTS` (Telegram 전송 재시도 횟수 override, 기본 `4`)
- `STEER_TELEGRAM_RETRY_MIN_DELAY_MS` (Telegram 전송 최소 backoff(ms), 기본 `400`)
- `STEER_TELEGRAM_RETRY_MAX_DELAY_MS` (Telegram 전송 최대 backoff(ms), 기본 `30000`)
- `STEER_TELEGRAM_RETRY_JITTER` (Telegram 전송 재시도 지터 비율 `0.0~0.5`, 기본 `0.1`)
- `STEER_DIAGNOSTIC_EVENTS` (구조 진단 이벤트 JSONL 기록 활성화, 기본 `1`)
- `STEER_DIAGNOSTIC_EVENTS_PATH` (구조 진단 이벤트 출력 경로, 기본 `scenario_results/diagnostic_events.jsonl`)
- `STEER_DIAGNOSTIC_EVENT_TAIL` (텔레그램 리포트에 포함할 진단 이벤트 tail 라인 수, 기본 `8`)
- `STEER_ALLOW_MULTI_SCHEDULER` (`Scheduler::start()` 중복 호출 허용 여부, 기본 `0`)
- `STEER_LOCK_SCOPE` (싱글턴 락 스코프 override; 기본은 현재 작업 디렉토리 해시)

## 🔍 릴리즈 워크트리 안전 체크

릴리즈 전 실수 푸시 방지를 위해 다음 스크립트를 사용하세요.

```bash
./scripts/check_release_worktree.sh
# staged 파일만 기준으로 검사
./scripts/check_release_worktree.sh --staged
# allowlist 자동 생성(현재 변경 파일 기준)
./scripts/check_release_worktree.sh --bootstrap-allowlist
```

- `STEER_RELEASE_MAX_CHANGED_FILES` (허용 변경 파일 수 상한, 기본 `40`)
- `STEER_RELEASE_ALLOWLIST_FILE` (허용 파일 목록 경로, 기본 `.release-allowlist`)
- `STEER_RELEASE_REQUIRE_ALLOWLIST` (dirty 워크트리에서 allowlist 파일 강제 여부, 기본 `1`)
- `STEER_RELEASE_DIFF_MODE` (검사 대상: `worktree|staged`, 기본 `worktree`)
- `STEER_RELEASE_WORKTREE_REPORT` (리포트 출력 경로 override)

allowlist 템플릿: `.release-allowlist.example`
- `STEER_ALLOW_LOCK_DISABLED_NON_TEST` (`run_nl_request_with_telegram.sh`/`run_complex_scenarios.sh`에서 `STEER_LOCK_DISABLED=1`을 테스트/CI 외 경로에서 예외 허용할지 여부, 기본 `0`)
- `STEER_MAIL_STRICT_DRAFT_CHECK` (Mail 전송 시 대상 draft id/marker/body를 엄격 검사, 기본 `1`)
- `STEER_CMD_N_WINDOW_FLOOD_LIMIT` (최근 히스토리에서 `Created new item` 감지 상한; 초과 시 `cmd+n` 강제 차단, 기본 `3`)
- `STEER_CMD_N_WINDOW_FLOOD_WINDOW` (`cmd+n` 창 폭증 감지 시 검사할 최근 history window 크기, 기본 `96`)
- `STEER_CMD_N_WINDOW_FLOOD_LIMIT_MAIL` (`Mail` 앱 대상 `cmd+n` 폭증 상한 override, 미설정 시 기본 `1`)
- `STEER_INPUT_GUARD_MAX_NEW_ITEMS` (실행 중 새 창 생성 이벤트 실시간 상한; 초과 시 InputGuard가 즉시 중단, 기본 `STEER_MAX_NEW_ITEM_ACTIONS` 또는 `6`)
- `STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT` (Mail 자동 draft 선택 허용 상한. 초과 시 `ambiguous_draft`로 차단, 기본 `8`)
- `STEER_API_KEY` (API 인증 키. 미설정 시 기본적으로 모든 요청 거부)
- `STEER_API_ALLOW_NO_KEY` (`1`이면 no-key 로컬 개발 모드 허용. 단, `STEER_TEST_MODE=1` 또는 `STEER_DEV_LOCAL_MODE=1` 필요)
- `STEER_DEV_LOCAL_MODE` (로컬 개발 실행 컨텍스트 표시)
- `STEER_API_DEV_HEADER_VALUE` (no-key 개발 모드에서 필수 `X-Steer-Dev` 헤더 값. 기본값 없음, 반드시 명시 필요)
- `STEER_OUTBOUND_TELEGRAM_STRICT` (텔레그램 발송 정책 검사 활성화, 기본 `1`)
- `STEER_OUTBOUND_TELEGRAM_REQUIRE_TEXT` (텔레그램 메시지 본문 비어있음 금지, 기본 `1`)
- `STEER_OUTBOUND_TELEGRAM_MAX_MESSAGE_CHARS` (텔레그램 메시지 최대 허용 길이, 기본 `120000`)
- `STEER_OUTBOUND_TELEGRAM_ALLOW_TARGET_IDS` (허용된 텔레그램 대상 chat_id CSV)
- `STEER_OUTBOUND_TELEGRAM_DENY_TARGET_IDS` (차단된 텔레그램 대상 chat_id CSV)
- `STEER_OUTBOUND_TELEGRAM_REQUIRE_REPORT_SHAPE` (리포트 메시지에 `상태:`/`근거:` 필수 강제, 기본 `0`)
- `STEER_TELEGRAM_REQUIRE_SEND` (정책 차단 시 텔레그램 전송을 실패로 승격할지 여부, 기본 `0`)

운영 점검 API:

- `GET /api/system/db-paths` : core DB 경로와 collector DB 경로 및 mismatch 여부 확인
- `GET /api/system/lock-metrics` : singleton lock 획득/차단/복구 텔레메트리 확인
- `GET /api/workflow/provision-ops` : workflow provisioning 상태 로그 조회
- `POST /api/agent/execute` 응답에 `resume_token` 포함 (manual/approval/blocked 시 재개 식별 토큰)
- `POST /api/agent/execute` 요청에서 `resume_token`/`resume_from` 입력 가능 (토큰 우선, plan_id/step 범위 검증 수행)
- `POST /api/agent/execute` 응답에 `completion_score` 포함 (score/label/pass/reasons)
- `POST /api/agent/preflight/fix` : Finder 전면 복구/권한 설정 화면 열기/격리 모드 준비(`prepare_isolated_mode`) 같은 즉시 조치 실행
- 동일 `plan_id`에 대한 `/api/agent/execute` 동시 실행은 `409 plan_execution_in_progress`로 직렬화

실행 로그:

- 시나리오 실행 스크립트는 `RUN_ATTEMPT`와 함께 `RUN_ATTEMPT_JSON|{...}` 구조 로그를 기록

Windows 서비스 실행(`scripts/run_core.ps1`)은 기본값이 `-CollectorImpl rust`입니다.

## 🎬 시연 빠른 실행

```bash
# 1) 원클릭 시연(사전점검 + 상태정리 + 녹화)
./scripts/demo_run.sh --preset news_telegram

# 2) 사용자 지정 자연어로 원클릭 시연
./scripts/demo_run.sh --prompt "오늘 받은 메일 5개 요약해줘"

# 3) 수동 단계로 실행하려면
./scripts/demo_prep.sh
./scripts/demo_state_reset.sh
./scripts/record_demo_preset.sh news_telegram
```

- 기본값은 **UI 경로 우선**이며(`STEER_DEMO_USE_UI=1`), UI submit 실패 시 fallback 자동 실행은 기본 `OFF`(`STEER_UI_FALLBACK_RUN=0`)입니다.
- `demo_run.sh`/`demo_ready_check.sh`는 시연 시작 전 `Steer OS` 앱 설치 여부를 먼저 확인합니다.
- `demo_run.sh`는 기본적으로 core API가 내려가 있으면 자동 기동을 시도합니다(`STEER_DEMO_START_CORE_IF_DOWN=1`, 로그: `scenario_results/demo_videos/core_autostart.log`).
- 자동 기동 대기 시간은 `STEER_DEMO_CORE_BOOT_TIMEOUT_SEC`(기본 45초)로 조정할 수 있습니다.
- `demo_ready_check.sh`는 `goal-run` 미지원 코어에서도 legacy goal 경로를 함께 점검해 시연 가능 여부를 판단합니다.
- `demo_state_reset.sh`는 기본적으로 Finder 포커스 복구 + Mail 초안창 정리 + Notes/TextEdit 중복 창 정리를 수행합니다.
- `demo_state_reset.sh`의 Notes/TextEdit 정리는 앱을 전면 활성화하지 않고 백그라운드로 창만 정리해 시연 화면 점프를 줄입니다.
- 필요 시 `STEER_DEMO_RESET_MAIL_OUTGOING=0`, `STEER_DEMO_RESET_NOTES_WINDOWS=0`, `STEER_DEMO_RESET_TEXTEDIT_WINDOWS=0`로 개별 비활성화할 수 있습니다.
- `record_ui_nl_demo.sh`는 기본적으로 `both`(enter+버튼) 전송을 사용하고, 미감지 시 재시도는 `auto` 모드(enter/button 자동 전환)를 사용합니다.
- `record_ui_nl_demo.sh`는 단일 실행 락(`/tmp/steer_ui_nl_demo.lock`)을 사용해 중복 녹화 실행을 차단합니다.
- 기본 재시도 횟수는 3회이며, 재시도 간격은 `STEER_UI_RETRY_INTERVAL_SEC`(시연 기본 4초)로 조정할 수 있습니다.
- `record_ui_nl_demo.sh`는 시연 기본값으로 `AX` 입력 주입(`STEER_UI_SET_VALUE_MODE=ax`)을 사용하고, 필요 시 입력 검증 재시도(`STEER_UI_INPUT_VERIFY_RETRIES`)를 수행합니다.
- `type` 폴백은 기본 비활성(`STEER_UI_ENABLE_TYPE_FALLBACK=0`)이며, 한글/비ASCII 프롬프트는 기본적으로 `AX`만 사용합니다.
- 기본값으로 `STEER_UI_REQUIRE_INPUT_MATCH=1`이 적용되어, UI 입력값이 요청문과 다르면 시연을 중단합니다.
- 기본값으로 `STEER_UI_ALLOW_INPUT_UNAVAILABLE=0`이 적용되어, 입력 필드 검증이 불가능한 상태를 성공으로 간주하지 않습니다.
- 기본값으로 `STEER_UI_REQUIRE_RUN_DETECTION=1`이 적용되어, UI에서 실제 run 감지가 안 되면 스크립트를 실패로 종료합니다.
- 기본값으로 `STEER_UI_REQUIRE_SUCCESS_STATUS=1`이 적용되어, run 감지 후에도 최종 상태가 `completed/success/finished`가 아니면 시연을 실패로 종료합니다.
- `record_ui_nl_demo.sh`는 시작 시 core status API를 먼저 확인하고, 준비되지 않으면 녹화를 시작하지 않고 즉시 실패합니다(정지 화면 영상 방지).
- 동시에 다른 작업이 있는 환경에서는 `STEER_UI_MATCH_TASK_RUN_PROMPT=1`(기본)로 같은 프롬프트의 run만 감지합니다.
- UI 실행 감지는 `task-runs` API를 우선 사용하고, 필요 시 기존 `nl_request_*.log` 감지로 내려갑니다.
- 필요 시 `STEER_UI_SUBMIT_METHOD=button|enter|both`, `STEER_UI_RETRY_SUBMIT_METHOD=auto|button|enter|both`로 전송 방식을 바꿀 수 있습니다.
- `STEER_UI_DETECT_WINDOW_SEC`로 UI 제출 후 run 감지 대기 시간을 조정할 수 있습니다(시연 기본 12초).
- `STEER_UI_MAX_RUN_IDLE_SEC`(시연 기본 25초) 동안 run 상태 변화가 없으면 시연을 `stalled`로 종료해 빈 화면 녹화를 줄입니다.
- 코어에 `/api/agent/goal/run`이 없어도 런처는 자동으로 legacy(`/api/agent/goal`) 경로로 폴백합니다.
- 시연 기본값은 과도한 증거 노이즈를 줄이기 위해 `STEER_NODE_CAPTURE_ALL=0`으로 설정됩니다(필요 시 `1`로 재활성화).
- 텔레그램 추가 이미지 첨부는 기본 0장(`STEER_TELEGRAM_EXTRA_IMAGE_MAX=0`)으로 제한됩니다.
- 텔레그램 결과는 시연 기본값으로 초간단 요약 모드(`STEER_TELEGRAM_SUPER_COMPACT=1`)를 사용합니다.
- 출력 파일은 `scenario_results/demo_videos`에 저장됩니다.

## 📜 라이선스

MIT License
