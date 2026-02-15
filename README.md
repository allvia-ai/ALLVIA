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
./target/release/core
```

## 📦 Release

To build a production-ready application (binary/bundle):

```bash
./scripts/build_release.sh
```

This script automates:
1.  **Frontend Build**: Compiles React/Vite assets.
2.  **Core Build**: Compiles Rust sidecar (steer-core).
3.  **Bundle**: Generates `.app` (macOS) or `.exe` in `desktop/src-tauri/target/release/bundle`.

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
- `STEER_SEMANTIC_ALLOW_LOG_EVIDENCE` (의미검증에서 로그-only 증거 허용 여부, 기본 `0`)
- `STEER_SEMANTIC_ALLOW_SCENARIO_FALLBACK` (`run_complex_scenarios.sh`에서 Rust 계약 실패 시 시나리오 내장 토큰 fallback 허용, 기본 `0`)
- `STEER_REQUIRE_MAIL_SUBJECT` (메일 성공 판정 시 subject 필수, 기본 `1`)
- `STEER_REQUIRE_SENT_MAILBOX_EVIDENCE` (메일 성공 판정 시 sent mailbox 증거 필수, 기본 `1`)
- `STEER_INPUT_GUARD_MAX_PAUSES` (입력 가드 최대 pause 횟수, 기본 `40`)
- `STEER_INPUT_GUARD_MAX_PAUSE_SECONDS` (입력 가드 누적 pause 허용 시간(초), 기본 `300`)
- `STEER_SCENARIO_IDS` (`run_complex_scenarios.sh` 실행 시 대상 시나리오 선택, 예: `1,3,5`, 기본 `1,2,3,4,5`)
- `STEER_GUI_REG_SCENARIOS` (`run_gui_regression_pack.sh`에서 반복 실행할 시나리오 집합, 기본 `1,2,3,4,5`)
- `STEER_FOCUS_RECOVERY_MAX_RETRIES` (UI 액션 후 타깃 앱 포커스 복구 재시도 횟수, 기본 `2`)
- `STEER_FOCUS_RECOVERY_PROFILE` (포커스 복구 전략: `standard|aggressive`, 기본 `standard`)
- `STEER_STRICT_ACTION_ERRORS` (`1`이면 모든 액션의 `status!=success`를 즉시 실행 오류로 승격; 기본 `0`)
- `STEER_ABORT_ON_EXECUTION_ERROR` (플래너 루프에서 `Critical ... failed` 액션 오류를 즉시 중단/실패로 승격; 기본 `1`)
- `STEER_PREFLIGHT_FOCUS_HANDOFF` (실행 전 Finder 전면 전환 검증으로 포커스 소유권 확인; 기본 `1`)
- `STEER_COMPLETION_SCORE_PASS` (`/api/agent/execute` 완성도 점수 pass 임계값(0~100), 기본 `75`)
- `STEER_ALLOW_COLLECTOR_DB_MISMATCH` (`workflow_intake`에서 collector/core DB 불일치 허용, 기본 `0`)
- `STEER_APPROVAL_ASK_FALLBACK` (승인 대기 시 실행 fallback 정책: `ask|deny|allow-once`, 시나리오 스크립트 기본 `deny`)
- `STEER_N8N_ALLOW_NPX_CLI` (`n8n` CLI 바이너리가 없을 때 `npx -y n8n` CLI fallback 허용, 기본 `0`)

운영 점검 API:

- `GET /api/system/db-paths` : core DB 경로와 collector DB 경로 및 mismatch 여부 확인
- `GET /api/workflow/provision-ops` : workflow provisioning 상태 로그 조회
- `POST /api/agent/execute` 응답에 `resume_token` 포함 (manual/approval/blocked 시 재개 식별 토큰)
- `POST /api/agent/execute` 응답에 `completion_score` 포함 (score/label/pass/reasons)
- `POST /api/agent/preflight/fix` : Finder 전면 복구/권한 설정 화면 열기/격리 모드 준비(`prepare_isolated_mode`) 같은 즉시 조치 실행
- 동일 `plan_id`에 대한 `/api/agent/execute` 동시 실행은 `409 plan_execution_in_progress`로 직렬화

실행 로그:

- 시나리오 실행 스크립트는 `RUN_ATTEMPT`와 함께 `RUN_ATTEMPT_JSON|{...}` 구조 로그를 기록

Windows 서비스 실행(`scripts/run_core.ps1`)은 기본값이 `-CollectorImpl rust`입니다.

## 📜 라이선스

MIT License
