# Steer OS: Local OS Agent 종합 문서

## 목차
1. [프로젝트 개요](#1-프로젝트-개요)
2. [핵심 가치 및 철학](#2-핵심-가치-및-철학)
3. [시스템 아키텍처](#3-시스템-아키텍처)
4. [보안 및 권한 모델](#4-보안-및-권한-모델)
5. [데이터 파이프라인 및 수집](#5-데이터-파이프라인-및-수집)
6. [UI 및 사용자 인터페이스](#6-ui-및-사용자-인터페이스)
7. [주요 모듈 상세 분석](#7-주요-모듈-상세-분석)
8. [설치 및 실행 가이드](#8-설치-및-실행-가이드)
9. [테스트 및 검증 전략](#9-테스트-및-검증-전략)
10. [향후 로드맵](#10-향후-로드맵)

---

## 1. 프로젝트 개요
**Steer OS (Local OS Agent)** 는 사용자의 로컬 환경(특히 macOS)에서 실행되는 고급 자율 에이전트 시스템입니다. 이 시스템은 단순히 사용자의 질문에 답하는 챗봇을 넘어, 컴퓨터 사용 패턴을 백그라운드에서 분석하고 실제 UI 제어, 운영 체제 수준의 자동화, 그리고 셸 명령 실행을 수행합니다. 사용자는 자연어로 복잡한 작업을 지시할 수 있으며, 시스템은 이를 이해하고 실행 계획을 세운 뒤 실제 행동으로 옮깁니다.

### 1.1 주요 목표
*   **자연어 기반 OS 제어:** 사용자의 자연어 지시를 실행 가능한 OS 명령이나 UI 조작으로 변환.
*   **지능형 워크플로우 제안:** 사용자의 반복적인 업무 패턴(루틴)을 학습하여 자동화 워크플로우를 선제적으로 제안.
*   **안전한 실행 환경 보장:** 잠재적으로 위험한 작업이 수행되기 전에 강력한 권한 관리 및 확인 절차 적용.

---

## 2. 핵심 가치 및 철학
이 프로젝트의 설계는 몇 가지 핵심적인 철학적 기반 위에 세워져 있습니다: **"LLM plans, Rust enforces, native layer executes"**.

### 2.1 Execution over Chat
단순히 조언을 제공하는 챗봇이 아닌, 실질적인 OS 조작을 수행하는 에이전트.
### 2.2 Safety First
강력한 격리 도구. 대규모 언어 모델(LLM)이 직접적으로 OS 데몬에 접근할 수 없습니다. 모든 LLM 생성 출력은 엄격한 보안 매니저(Broker/Policy Engine)를 통과해야만 실제 실행계층으로 넘어갑니다.
### 2.3 Closed Loop (검증 루프)
액션을 취한 뒤 단순히 끝내는 것이 아니라, 그 액션이 시스템에 실제 원하는 변화를 가져왔는지(예: "메모장에 글이 작성되었는가?") 네이티브 API 등을 통해 능동적으로 검증(Verify)합니다. 이 단계를 통과해야만 하나의 작업이 완료된 것으로 간주됩니다.
### 2.4 운영체제 독립성과 종속성의 조화
핵심 논리(로직, 보안 정책, 분석 등)는 플랫폼 독립적인 Rust로 작성하여 안정성과 성능을 챙기고, 실제 마우스를 움직이고 창을 찾는 부분만 각 OS의 특화된(Native) API에 의존합니다.

---

## 3. 시스템 아키텍처
전체 시스템은 다음과 같은 티어(Tier)로 구분됩니다.

### 3.1 Rust Core (`core/`)
전체 시스템의 관제탑이자 실행 엔진입니다.
*   **상태 관리 (State Management):** 현재 실행 중인 계획과 세션, 액션의 상태를 추적.
*   **정책 엔진 (Policy Enforcement):** 허용/차단 목록 기반으로 실행 가능한 명령 필터링.
*   **계획 및 실행 (Planning & Execution):** LLM이 생성한 추상적 계획을 구체적인 시스템 액션으로 해독하고 셸/UI 조작 실행.
*   **API 서버 (Axum):** `127.0.0.1:5680` 포트에서 실행되며, 프론트엔드 및 CLI와 통신.

### 3.2 Web UI 및 Desktop Wrapper (`web/` & `src-tauri/`)
사용자가 에이전트와 상호 작용하는 오퍼레이터 대시보드입니다.
*   **React + Vite + TypeScript:** 최신 웹 기술 기반 프론트엔드.
*   **Tauri 프레임워크:** 웹 프론트엔드를 네이티브 데스크톱 애플리케이션(`Steer OS.app`)으로 패키징.
*   Rust Core API와 통신하여 실시간 상태, 로그, 검증 결과를 화면에 표시.

### 3.3 Data Pipeline & Collector (`collector_rs`)
백그라운드에서 데이터를 수집하고 분석하는 모듈입니다.
*   이벤트 청취기: 활성 창 전환, 클릭, 타이핑 등 시스템 이벤트를 모니터링.
*   데이터 마스킹 (Privacy Guard): 개인 정보 및 민감한 데이터 익명화.
*   로컬 저장소: 이벤트와 분석 결과를 SQLite 데이터베이스에 임시 보관.

### 3.4 Native Control Layer
실제 운영 체제와 상호 작용하는 부분입니다.
*   macOS의 경우 AppleScript, Swift 브리지, Accessibility API 등을 통해 윈도우 포커스 변경, 텍스트 입력 점검 등을 수행합니다.

---

## 4. 보안 및 권한 모델
이 시스템은 화면 캡처, 키보드 타이핑 등 시스템 전체 권한(Global-Permission)을 사용하므로 제로 트러스트(Zero Trust) 모델을 채택합니다.

### 4.1 Write Lock Mechanism (쓰기 잠금)
*   **기본 상태 (Default State):** 항상 `LOCKED` (읽기 전용). 화면 캡처나 앱 목록 조회는 가능하지만 변경은 불가.
*   **잠금 해제 (Unlock Trigger):** 사용자의 명시적 승인(CLI Confirm, UI 버튼 클릭 등) 시에만 해제됨.
*   **자동 잠금 (Auto-Lock):** 작업 완료 후, 혹은 일정 시간 동안 액션이 없으면 자동으로 다시 잠김 체제로 복귀.

### 4.2 Action Classification (액션 분류)
*   **Safe (안전):** `ui.find`, `screen.capture`, `app.list` 등 시스템 조회를 요구하는 명령. 승인 없이 자동 수행.
*   **Caution (주의):** `ui.click`, `keyboard.type` 등 창의 내용을 변경하거나 이벤트를 주입하는 명령. Write Lock 해제가 필요.
*   **Critical (위험):** `file.delete`, `cmd.exec` (sudo 등 포함), 결제 등 돌이킬 수 없는 중요한 명령. 매개 변수를 분석해 엄격히 차단하거나, 반드시 건별(2FA 방식) 사용자의 명시적 승인을 받아야 함.

### 4.3 Fail-Safe (안전 장치)
애플리케이션 폭주를 막기 위해 에이전트 실행 도중 사용자가 `Esc` 키를 연타하거나 특정 단축키(Hot Key)를 누르면 동작 중인 하위 프로세스를 즉시 강제 종료(Kill Switch) 시킬 수 있습니다.

---

## 5. 데이터 파이프라인 및 수집
이 기능은 사용자의 일상을 섀도잉하여 패턴을 도출하는 핵심 기능입니다. 백그라운드 수집 프로세스(`collector_rs`)는 극도로 가볍고 안전하게 작동하도록 설계되어 있습니다.

### 5.1 데이터 흐름
1.  **원본 데이터 생성:** 플랫폼 센서가 시스템 이벤트를 포착.
2.  **보안 전처리:** `PrivacyGuard` 모듈을 통과하며 비밀번호 필드나 제외된(Denylist) 애플리케이션의 데이터 제거/마스킹.
3.  **Local DB 저장:** 검증된 원시 데이터(Raw Log)는 `steer.db` (또는 `collector.db`)의 `events_v2` 테이블에 저장.

### 5.2 최적화된 용량 및 보존 주기 (Retention)
시스템 메모리 소모를 줄이고, 하드 디스크 용량 축적을 억제합니다.
*   **Raw 데이터 제거:** 원시 데이터(`events_v2`)는 용량을 많이 차지하므로 `STEER_COLLECTOR_RAW_RETENTION_DAYS` 설정(기본 7일)에 따라 자동으로 하드 삭제(Hard Delete)됨.
*   **데이터 압축 요약 (Aggregation):** 타이머가 5분에 한 번씩 돌아가며 "지난 5분간 어떤 앱에서 어떤 액션이 몇 번 일어났는지" 통계를 냅니다. 이 결과는 `minute_aggregates`라는 요약 전용 테이블에 저장됩니다. 요약 데이터 보존 주기는 기본 30일(`STEER_COLLECTOR_SUMMARY_RETENTION_DAYS`)입니다.

### 5.3 루틴 제안 (Routine Recommendation)
*   데이터베이스에 쌓인 일간/주간 집계 자료(`daily_summaries` 등)를 스캔하여 자주 반복되는 패턴(n-gram)을 찾고 알고리즘을 통해 루틴 후보지(`RoutineCandidate`)로 승격합니다.

---

## 6. UI 및 사용자 인터페이스
`web/` 디렉터리 하위에 구성된 UI는 단순한 채팅 창이 아닙니다. 이 모델의 아웃프론트가 되는 복합 관리 데스크입니다.

### 6.1 기술 스택
*   **Frontend:** React, TypeScript, Vite
*   **Styling:** PostCSS, TailwindCSS
*   **Desktop Binding:** Tauri (Cross-platform GUI Framework)

### 6.2 주요 화면 구성
*   **Chat View:** 사용자가 자연어 명령을 입력하고 시스템 메시지 및 LLM의 사고 과정을 스트리밍으로 확인하는 메인 화면입니다.
*   **Permission Dialog:** 위험한 명령(Critical Actions)이 감지되었을 때 에이전트가 동작을 멈추고 팝업을 띄워 사용자에게 수락/거부를 묻습니다.
*   **Routine Dashboard:** 수집기가 백그라운드에서 분석해 제안한 새로운 패턴 기반의 워크플로우를 보여주고 n8n 등과 연동할 수 있는 대시보드 화면입니다.

---

## 7. 주요 모듈 상세 분석
`core/src/` 내부에는 시스템 로직이 빼곡히 들어차 있습니다.

*   `main.rs`: 진입점 시스템 초기화 및 Axum 서버 부팅 역할
*   `api_server.rs`: 프론트엔드 및 CLI 요청을 처리하는 HTTP 엔드포인트 세트 (`/api/agent/execute`, `/api/agent/preflight` 등)
*   `db.rs`: SQLite3 기반 로컬 데이터베이스 커넥션 풀, 초기화(Migration), 쿼리 실행기
*   `llm_gateway.rs`: OpenAI 등 외부 언어 모델 API 통신 및 프롬프트 주입 담당 모듈
*   `policy.rs` & `security.rs`: 허용(Allowlist)/차단(Denylist) 리스트 정책을 확인하고 위험 명령어 수준을 정하는 엔진
*   `executor.rs` & `shell_actions.rs`: Bash 셸 환경 등에서 프로세스를 스폰하고 출력을 캡처하는 실제 액터
*   `visual_driver.rs` & `macos/`: OS 특정의 그래픽과 네이티브 바인딩을 통해 타겟 앱 활성화, 좌표 찾기, 클릭 입력 주입 담당

---

## 8. 설치 및 실행 가이드

### 8.1 필수 사양
*   OS: macOS 12 (Monterey) 이상 (Accessibility 지원 필수)
*   개발 도구: Rust 1.70+, Node.js (v18+ 권장)
*   외부 API: OpenAI API Key (`.env` 설정)

### 8.2 개발 환경 구축 및 실행
```bash
# 1. 저장소 클론 및 이동
git clone <repo_url>
cd local-os-agent/core

# 2. 환경변수 준비
cp .env.example .env
# nano .env (OPENAI_API_KEY 입력)

# 3. 배포 및 번들 릴리즈 스크립트 실행 (한 번에 앱 빌드까지)
./scripts/rebuild_and_deploy.sh

# 4. 빠른 개발 모드 (UI 통합 제외, 코어만 실행)
export STEER_API_ALLOW_NO_KEY=1
cargo run --release
```

### 8.3 백그라운드 데몬 및 n8n 통합
n8n 워크플로우 엔진 통합은 별도 도커나 npx 모드로 구동됩니다.
```bash
STEER_N8N_RUNTIME=docker docker compose up -d n8n
```

---

## 9. 테스트 및 검증 전략
명령이 실행된 후 그 결과가 의도치 않았을 때를 대비해 강력한 테스트 셋(E2E) 스크립트를 보유합니다.

### 9.1 시스템 자가 테스트
*   단위 테스트: `cargo test` 명령어를 통한 `core/` 내부 모듈 논리 무결성 확인
*   문법 린팅: `cargo clippy` 및 `npm run lint`

### 9.2 회귀 테스트 팩 시나리오 (`run_gui_regression_pack.sh` 등)
가상 데이터를 주입하여 UI가 기대하는 대로 렌더링되고, 팝업 지시가 올바른 절차를 따라가는지 확인합니다.
수많은 쉘 스크립트:
*   `run_complex_scenarios.sh`
*   `run_nl_request_with_telegram.sh`
*   `demo_run.sh` 등은 에이전트의 완성도(Score)를 채점하고 슬랙/텔레그램으로 테스트 수행 요약을 발송하는 역할을 합니다.

---

## 10. 향후 로드맵 (Roadmap)
이 프로젝트는 현재도 진화하고 있으며 향후 계획된 굵직한 피처들은 다음과 같습니다.

### 10.1 Phase 1 (완료)
*   Read-only Agent 구조 구축 (스크린 캡처, Accessibility Tree 탐색 위주)
*   기본 UI 및 Tauri 데스크톱 연동 환경 세팅 완료

### 10.2 Phase 2 (진행 중)
*   Controlled Act 락인 적용: 확인 후 클릭/글쇠 입력(Type) 수행 루프 안정화
*   Rust 기반 초경량 데이터 수집기(Collector_rs) 포팅 및 SQLite 집계 로직 완성

### 10.3 Phase 3 (예정)
*   Full Policy Engine 구현: 글로벌 정책 서버 등을 연계한 Allow/Deny List 실시간 업데이트
*   다중 복합 목표(Complex Planning) 대응: "브라우저를 켜서 오늘 뉴스를 다 긁은 다음, 요약해서 팀원 3명에게 각각 이메일로 보내줘"와 같은 연속적인 N단계 복합 태스크에 대한 컨텍스트 유지 기능 고도화.

---

*문서 최종 업데이트: 2026-02-23*
*본 문서는 Steer OS 프로젝트의 아키텍처와 소스를 기반으로 자동 생성 및 종합된 참고용 데이터입니다.*
# Steer OS: Local OS Agent 종합 가이드 (Comprehensive Guide) - 제 2부

## 5. 시스템 아키텍처 상세 분석 (System Architecture Deep Dive)

이 장에서는 Steer OS의 핵심을 이루는 4개의 거대한 구성 요소(Rust Core, Data Pipeline, Native Control Layer, User Interface)가 어떻게 유기적으로 결합하여 동작하는지 소스 코드 레벨(`core/src/`)에서 심층적으로 분석합니다.

### 5.1 Rust Core (`core/`) - 중앙 통제 및 실행 엔진
프로젝트의 심장이자 두뇌 역할을 하는 이 모듈은 안전하고 예측 가능한 실행을 보장하기 위해 강력한 상태 머신(State Machine)과 컴포넌트들로 구성되어 있습니다.

#### 5.1.1 API Server (`api_server.rs`)
모든 외부 상호작용의 관문입니다. 프론트엔드(Tauri/React) 및 CLI에서 들어오는 요청을 처리합니다.
*   **엔드포인트:** 
    *   `/api/agent/execute`: 자연어 명령을 분석하고 실행 루프를 진입시킵니다. 내부적으로 `TaskRun` 객체를 생성하고 추적합니다.
    *   `/api/agent/preflight`: OS 제어(특히 마우스/키보드 주입 등) 전에 필요한 권한(Accessibility 등)이 확보되었는지 점검합니다.
    *   `/api/system/health`: 시스템 리소스 및 상태를 모니터링합니다.

#### 5.1.2 Controller & Planner (`controller/`, `plan_builder.rs`)
단순한 챗봇과 차별화되는 가장 중요한 모듈입니다.
*   사용자의 요청이 들어오면 `llm_gateway.rs`를 통해 OpenAI API로 프롬프트를 전송합니다. 
*   하지만 이때 반환받는 것은 "이렇게 하세요"라는 텍스트가 아니라, JSON 형태의 **실행 계획(Plan)**입니다.
*   이 계획은 여러 개의 "Step"으로 분할되며, 각 Step은 액션 객체(`ActionSchema`)의 모음입니다.

#### 5.1.3 Policy & Security Engine (`policy.rs`, `security.rs`, `approval_gate.rs`)
계획이 수립되었다면, 다음은 검열 단계입니다.
*   `Action Classification:` 실행할 Step이 시스템에 무해한지(Safe), 경고 대상인지(Caution), 치명적인지(Critical)를 평가합니다.
*   `Approval Gate:` 만약 Critical한 액션(예: 파일 삭제, 계좌 이체 등)이라면, 즉시 실행을 중단(Yield)하고 API Server를 통해 클라이언트(UI)에 승인 대기(Approval Pending) 상태를 통보합니다.

#### 5.1.4 Executor (`executor.rs`, `shell_actions.rs`)
검열을 통과한 액션을 실제 로컬 프로세스나 네트워크 요청으로 탈바꿈시킵니다. Bash 셸 스크립트 실행, 로컬 앱 스폰(Spawn), HTTP 요청 등 다방면의 인터페이스를 제공합니다.

#### 5.1.5 Verification Engine (`verification_engine.rs`)
액션 실행 후, 그 결과가 "진짜로 성공했는지"를 네이티브 API와 LLM의 시각/논리 판독을 통해 교차 검증하는 최종 관문입니다.

---

### 5.2 Native Control Layer (`visual_driver.rs`, `macos/`) - 네이티브 실행망
LLM과 운영체제를 연결하는 가장 말단의 드라이버입니다. 플랫폼 한정(OS-dependent)적인 역할을 수행합니다.

*   `applescript.rs`: macOS에 내장된 AppleScript를 브릿지로 사용하여 창을 열거나, 특정 앱을 최상단(Focus)으로 올리는 등의 역할을 합니다.
*   `macos/accessibility`: macOS의 강력한 손쉬운 사용(Accessibility, AX) UI 트리를 조회합니다. 버튼, 텍스트 입력창, 메뉴 바 등 거의 모든 화면 요소를 픽셀(Pixel) 좌표가 아닌 논리적인 노드(Node ID)로 찾아내고 클릭할 수 있습니다. 

### 5.3 Data Pipeline & Collector (`collector_pipeline.rs`, `collector_rs.rs`)
백그라운드에서 인간의 행동을 학습하고 패턴을 뽑아내는 고독한 관찰자입니다.

#### 5.3.1 이벤트 로깅 (Events V2)
마우스가 클릭되거나 창이 전환될 때마다 이벤트가 발생합니다. 이 이벤트는 초경량 데몬에 의해 수집되어 SQLite 데이터베이스의 `events_v2` 테이블에 저장됩니다. (성능을 위해 기본 최대 보존기간 7일)

#### 5.3.2 데이터 요약 및 압축 (Aggregation)
로그가 무한정 쌓이는 것을 막기 위해 강력한 압축 타이머(Ticker)가 작동합니다.
*   **5분 요약 (`minute_aggregates`):** 이전 5분 동안 "카카오톡 창에서 어떤 행동을 가장 많이 했는가?"를 계산하여 1줄의 로그로 압축합니다.
*   **일일 요약 (`daily_summaries`):** 하루에 가장 많이 쓴 앱 순위와 활성 시간을 정리합니다.

#### 5.3.3 루틴 추출기 (Routine / Pattern Detector)
매일 "오전 9시에 슬랙을 확인하고 메일을 연다"는 패턴이 감지되면, 이를 `RoutineCandidate`(루틴 후보)로 등록합니다. 시스템은 이를 바탕으로 사용자에게 n8n 등의 자동화 플로우로 변환할 것을 제시합니다.

---

### 5.4 Database (`db.rs`) - 로컬 상태의 영속성
모든 상태 이력은 가볍고 빠른 SQLite에 의존합니다.
*   `task_runs`: 메인 실행 루프에 대한 추적 테이블 (계획 아이디, 성공 유무, 소요 시간 등)
*   `task_stage_runs`: 어떤 태스크 내에서 개별적으로 거친 5단계 등 세부 스텝
*   `sessions_v2` & `collector_handoff_queue`: 수집기와 메인 통제기 사이에서 행동 패턴을 주고받기 위한 교환대

---

제 2부(시스템 아키텍처 상세) 서술을 마쳤습니다. 계속 이어서 핵심 제어 로직(Execution Flow)과 보안 승인 절차(Security Lifecycle)에 대한 코어 코드를 분석하는 3부를 작성하겠습니다.

확인하셨다면 **"계속"** 이라고 입력해 주세요.

---

## 6. 제어 논리 및 실행 흐름 분석 (Execution Flow & Control Logic)

Steer OS에서 단일 명령이 어떻게 처리되고 끝맺어지는지, 즉 생애 주기(Lifecycle)를 코드 흐름을 따라 추적합니다. 

### 6.1 `TaskRun` 라이프사이클 (The Lifecycle of a TaskRun)
사용자가 "내일 제주도 날씨를 검색해서 메모장에 저장해줘"라는 명령을 내렸다고 가정해 보겠습니다.

#### 단계 1. 의도 파악 및 계획 수립 (Intent & Plan)
1.  **API 호출:** `/api/agent/execute` 로 요청이 들어옵니다.
2.  **의도 분석:** `intent_router.rs`가 "이 요청은 OS 제어 흐름인가, 단순 질문인가, 혹은 일상 대화인가?"를 분류합니다.
3.  **계획 생성:** `plan_builder.rs`가 작동합니다. LLM(`llm_gateway.rs`)에게 프롬프트를 보내 다음과 같은 추상적인 JSON 플랜(`TaskPlan`)을 받아옵니다.
    *   *Step 1:* 브라우저 열기 & 날씨 검색 (Action)
    *   *Step 2:* 검색 결과 텍스트 읽기 (Action)
    *   *Step 3:* 메모장 애플리케이션 포커스 및 텍스트 타이핑 (Action)

#### 단계 2. 실행 컨트롤러 진입 (Execution Controller)
`execution_controller.rs`가 바통을 이어받습니다. 생성된 `TaskPlan`의 Step들을 하나씩 순차(Sequential) 실행합니다.
1.  **사전 검사 (Preflight Check):** `preflight.rs`를 통해 현재 Accessibility 권한이 있는지, 키보드 입력이 가능한 상태인지 점검합니다.
2.  **권한 차단 검사 (Approval Gate):** 각 Step 내부의 하위 Action이 실행되기 직전, `approval_gate.rs`가 끼어듭니다.
    *   "브라우저 열기" -> Safe (통과)
    *   "메모장 열고 타이핑" -> Caution (Write Lock 해제 필요)
    *   여기서 Lock이 걸려있다면 액션을 멈추고 `Pending` 상태로 UI에 승인 요청을 띄웁니다. 사용자가 승인하면 멈췄던 곳부터 다시 재개(Resume)합니다.

#### 단계 3. 어댑터와 네이티브 실행 (Adapter & Execution)
`shell_actions.rs`나 `visual_driver.rs`가 호출됩니다.
*   *브라우저 제어:* macOS의 `NSWorkspace` 나 AppleScript(`osascript`)를 통해 Safari나 Chrome을 활성화합니다.
*   *UI 주입:* `macos/accessibility` 모듈을 통해 화면 요소를 덤프받고, 조건에 맞는 요소의 중앙(`(x, y)` 좌표)을 계산해 `CGEvent`를 발생시켜 진짜 마우스 클릭과 타이핑을 흉내 냅니다.

#### 단계 4. 현실 검증 (Reality Check & Verification)
이 시스템의 백미입니다. `verification_engine.rs`에서 실행 후 변경된 화면 상태를 스크린샷으로 찍거나, 다시 UI 트리를 가져옵니다.
*   LLM 비전(Vision) 기능을 통해 "화면에 제주도 날씨가 보이는가?"를 묻고 True/False 답을 받습니다.
*   결과가 성공(Success)이면 다음 Step으로 넘어가고, 실패(Failure)면 `retry_logic.rs`에 의해 복구를 시도하거나 사용자에게 실패를 보고합니다.

---

## 7. 시스템 보안 및 개인정보 보호 고도화 (Security & Privacy Hardening)
Steer OS는 단순 샌드박스를 넘어서는 3단계 방어벽을 구축하고 있습니다.

### 7.1 도구 사용 정책 (Tool Policy)
`policy.rs`는 LLM이 호출할 수 있는 함수(Tool) 목록을 통제합니다.
*   **Allowlist 기반 접근:** 시스템 파괴 명령(`rm -rf` 등)을 LLM이 무작위로 생성하더라도, `shell_actions.rs`는 사전에 정의된 안전한 명령어 패턴(e.g., `ls`, `cat 특정디렉토리`)이 아니면 OS 셸로 넘기지 않고 즉각 `CommandRejected` 에러를 반환해 버립니다.
*   **격리 모드 (Isolated Mode):** 테스트 환경이나 극도로 위험한 작업 시, 특정 폴더(`CWD` Jail) 밖으로 나가는 명령어를 무효화시킵니다.

### 7.2 외부 전송 정책 (Outbound Policy)
개인 데이터가 에이전트를 통해 밖으로 새어 나가는 것을 막습니다. `outbound_policy.rs`는 이메일, 텔레그램 메시지, 혹은 webhook 등을 통한 외부 데이터 전송 시 다음을 강제합니다:
*   수신자가 사전에 허가된(Allowlisted) 연락처인가?
*   본문에 개인 식별 정보(PII)가 포함되어 있는가? (메일 전송 전 자체 마스킹)

### 7.3 Data Privacy Guard (데이터 마스킹)
백그라운드에서 데이터를 수집하는 `collector_rs` 내부의 `PrivacyGuard`는 창 제목(Window Title)이나 입력된 텍스트에서 주민번호, 이메일 주소 패턴 등을 정규식(Regex)으로 탐지하고 즉시 암호화하거나 `[REDACTED]` 처리하여 로컬 DB에 저장합니다.

---

제 3부(제어 논리 및 실행 흐름 분석, 시스템 보안 고도화) 작성을 마쳤습니다. 
계속해서 마지막 4부(코드베이스 폴더별 역할 사전 및 향후 진화 방향 정리)를 작성하겠습니다. 

확인하셨다면 **"계속"** 이라고 입력해 주세요.

---

## 8. 전체 시스템 폴더 및 모듈 딕셔너리 (Directory & Module Dictionary)

복잡한 프로젝트를 처음 접하는 개발자를 위해, 각 서브 디렉토리 및 주요 파일의 역할을 사전식으로 정리합니다.

### 8.1 `core/` (Rust 기반 백엔드 및 코어 로직)
Rust 워크스페이스의 메인 루트입니다. 바이너리를 빌드하고 실행하는 모든 출발점입니다.
*   **`Cargo.toml`:** 전역 의존성(rusqlite, axum, tokio 등)을 관리합니다.
*   **`src/main.rs`:** 애플리케이션 진입점. 환경 변수를 읽어오고 HTTP 서버를 바인딩하여 백그라운드 데몬을 초기화합니다.
*   **`src/api_server.rs`:** Axum 라우팅 로직.
*   **`src/controller/`:** UI와 모델 간의 중간 제어 계층.
*   **`src/db.rs`:** SQLite 데이터베이스 초기화(Migration) 및 쿼리 함수 모음.
*   **`src/llm_gateway.rs`:** LLM(현재 OpenAI API 중심)과의 통신 모듈. 프롬프트를 만들고 응답을 파싱합니다.
*   **`src/visual_driver.rs`:** 실제 마우스를 움직이고 클릭하는 행위(Action)를 OS 시스템 콜로 변환하는 드라이버입니다.
*   **`src/macos/`:** macOS 한정 기능. Accessibility API와 통신하는 네이티브 브리지 역할을 합니다.
*   **`src/bin/collector_rs.rs`:** 백그라운드 데이터 수집기(Collector)의 메인 루프 바이너리입니다. `main.rs`와 별개의 프로세스로 띄울 수 있습니다.
*   **`src/collector_pipeline.rs`:** `collector_rs.rs`가 수집한 데이터를 주기적으로 집계하고 삭제 주기(Retention)를 관리하는 파이프라인 로직입니다.

### 8.2 `web/` 및 `src-tauri/` (사용자 대시보드 및 데스크톱 앱 래퍼)
사용자가 직접 보게 되는 프론트엔드입니다.
*   **`web/src/`:** React 기반의 채팅 UI, 설정 창, 대시보드 컴포넌트들이 존재합니다.
*   **`web/package.json`:** NPM 의존성 및 빌드 스크립트.
*   **`src-tauri/`:** Tauri 껍데기. 빌드된 프론트엔드 사양을 읽어 실제 macOS `.app` 번들로 묶어냅니다. Rust Core 바이너리를 이 안에 사이드카(Sidecar)로 포함시켜 배포합니다.

### 8.3 `scripts/` 및 `tests/` (운영 자동화 및 테스트 팩)
개발 생산성 확보와 배포, 시스템의 안정성 검증을 위한 셸(Shell) 스크립트 및 테스트 코드가 모여 있습니다.
*   **`scripts/rebuild_and_deploy.sh`:** Rust 코어를 빌드하고 Tauri 앱으로 패키징한 뒤, `/Applications` 폴더에 설치하는 원클릭 배포 스크립트.
*   **`scripts/run_complex_scenarios.sh`:** 에이전트가 복잡한 시나리오를 제대로 수행하는지 릴리즈 전에 완벽히 검증하는 리그레션 테스트 메인 스크립트.
*   **`scripts/demo_run.sh`:** 개발자 및 관리자가 시연(Demo)을 할 수 있도록 앱 상태를 청소(Reset)하고 화면 녹화까지 도와주는 보조 스크립트.
*   **`tests/`:** Python 등으로 작성된 엔드투엔드(E2E) 스크립트 구동 시 로컬 데몬을 띄우고 상태 코드를 점검하기 위한 테스트 프레임워크 요소.

---

## 9. 향후 프로젝트 진화 방향 (Future Evolution)

Steer OS 시스템은 현재 "수동적인 도우미"에서 "주도적인 오퍼레이터"로 진화하는 과도기(Phase 2)에 있습니다. 향후 다음과 같은 방향으로 더 거대하고 정밀한 시스템이 구축될 예정입니다.

1.  **자체 호스팅된 로컬 모델 (Fully Local LLM):** 현재는 판단과 계획(Planning)을 외부 OpenAI API에 크게 의존하고 있으나, 보안(Privacy) 극대화를 위해 향후 고성능 Mac(M 시리즈 칩셋) 내부에서 Apple MLX나 `llama.cpp`를 구동하여 외부 네트워크 없이 독립 생존 가능한 하이브리드 로컬 에이전트를 구축할 것입니다.
2.  **Multimodal & Vision 융합:** 단지 윈도우 안에 무슨 텍스트가 있는가를 넘어, 바탕 화면 스크린샷 자체를 비전 모델(Vision Model)로 직접 분석하고(예: "우측 하단에 있는 저 빨간색 버튼을 눌러") 화면상의 기하학적 형상을 이용한 직접 제어 모드를 추가할 예정입니다.
3.  **Cross-Platform 확장:** 현재는 macOS의 접근성(Accessibility)에 단단히 결합되어 있으나, 윈도우(UI Automation) 및 리눅스 시스템(X11/Wayland 기반 제어기) 아키텍처 지원을 Rust Core 레벨에서 추상화해 나갈 로드맵을 가지고 있습니다.

## 결문 (Conclusion)
**Steer OS**는 단순한 기술적 데모가 아닙니다. 인공지능이 인간의 로컬 워크스페이스(Workspace)에 들어와 어떻게 인간의 손과 발을 대신할 수 있는지를 보여주는 실증적인 프로토타입이며, 동시에 철저히 분리된 권한 제어와 검증 레이어가 어떻게 인공지능의 "환각(Hallucination)"과 폭주 시스템으로부터 운영체제를 보호하는지 증명하는 소프트웨어 아키텍처의 정본입니다.

---
*(이 가이드라인은 계속적으로 업데이트되며 개발자의 편의를 도울 것입니다. End of Guide. 끝)*


## 10. 핵심 비즈니스 로직 - API Server 및 LLM Gateway 심층 해부

프로젝트의 중심부인 `core/src/api_server.rs`와 `core/src/llm_gateway.rs` 모듈은 각각 진입점과 지능(Intelligence)을 담당합니다. 

### 10.1 API Server (`api_server.rs`)

`api_server.rs`는 사용자의 프론트엔드 또는 외부 시스템에서 보내는 모든 명령의 관문입니다. 이 모듈은 Axum 웹 프레임워크를 기반으로 매우 가볍고 비동기적으로(Asynchronous) 설계되었습니다.

#### 10.1.1 주요 엔드포인트 라우팅
*   `POST /api/agent/execute`: 에이전트를 가동시키는 가장 핵심적인 API입니다.
    *   **입력 (Request):** 사용자의 자연어 명령 (예: "데스크탑에 있는 파일 3개를 압축해서 이메일로 보내줘.")
    *   **처리 흐름:** 
        1. 요청을 받자마자 유니크한 `TaskRun ID`를 발급합니다.
        2. 이 ID를 통해 데이터베이스(`task_runs` 테이블)에 초기 상태를 `PENDING`으로 기록합니다.
        3. `execution_controller` 의 런타임 루프 속으로 이 작업을 비동기 전달합니다.
    *   **출력 (Response):** 작업이 끝날 때까지 기다리지 않고 202 Accepted 상태코드와 함께 Task ID를 즉시 반환하여, 프론트엔드가 실시간으로 폴링(Polling)하거나 웹소켓으로 상태를 구독할 수 있게 합니다.

*   `POST /api/agent/preflight`: 실행 전 점검(Preflight) 기능을 제공합니다.
    *   UI에서 본격적인 작업을 지시하기 전에, 현재 운영체제 환경에서 윈도우 접근성(Accessibility) 권한이나 디스크 권한이 충분히 확보되어 있는지를 먼저 점검합니다.
    *   실패할 경우, 에러 메시지를 통해 사용자에게 권한 허용을 유도하는 가이드를 제공합니다.

*   `POST /api/agent/approve` & `/api/agent/reject`:
    *   위험한 작업(Critical Action)이 감지되어 시스템이 `Blocked` 상태로 대기 중일 때, 사용자가 승인 또는 거절 버튼을 누르면 이 엔드포인트로 신호가 들어옵니다.
    *   신호를 받은 서버는 내부의 `Singleton Lock`과 상태 머신을 업데이트하여, 멈췄던 작업을 재개(Resume)시키거나 취소(Abort)합니다.

#### 10.1.2 동시성 관리 및 잠금(Locking) 체계
*   **Singleton Lock:** 두 개의 에이전트 인스턴스가 동시에 마우스를 움직이면 제어권 충돌이 발생합니다. API 서버는 명령을 수신하면 즉시 글로별 `Lock`을 획득합니다. 다른 명령이 들어오면 대기 큐(Queue)에 넣거나 409 Conflict 에러를 반환하여 마우스와 키보드의 소유권을 단일 작업자(Single Worker)에게만 부여합니다.

---

### 10.2 LLM Gateway (`llm_gateway.rs`)

시스템의 '두뇌'인 거대 언어 모델(현재 OpenAI 기반)과 소통하는 창구입니다. 단순한 REST API 감싸개(Wrapper)가 아니라 복잡한 지침과 컨텍스트를 조립하는 프롬프트 공학(Prompt Engineering)의 정수입니다.

#### 10.2.1 구조적 프롬프팅 (Structured Prompting)
LLM이 자유분방한 텍스트로 대답하면 컴퓨터 프로그램이 파싱할 수 없습니다. `llm_gateway.rs`는 LLM에게 엄격한 **JSON 스키마**를 요구합니다.
*   에이전트에게 주어지는 메인 시스템 프롬프트(System Prompt)에는 에이전트가 호출할 수 있는 함수(도구, Tool)들의 명세서(`ActionSchema`)가 포함됩니다.
*   예를 들어, "당신은 `ui.click`, `keyboard.type`, `shell.exec` 도구를 사용할 수 있습니다. 응답은 무조건 이 도구들의 배열 형태인 JSON으로 반환하세요." 라고 강제합니다.
*   만약 LLM이 JSON 형식을 어기면, `llm_gateway.rs`는 스스로 에러 메시지를 덧붙여 다시 LLM에게 전송(Self-Correction)하여 올바른 JSON을 받아냅니다. 최대 3회의 Retry 로직이 구현되어 있습니다.

#### 10.2.2 Context Window 최적화
에이전트가 화면 안의 요소들을 알기 위해서는 현재 화면의 UI 트리(접근성 노드)를 문자로 바꿔서 LLM에게 보내야 합니다.
*   macOS의 전체 접근성 트리는 수만 개의 문자로 이루어질 수 있기 때문에, LLM의 한 번 입력 글자 수 제한(Context Window)을 금방 초과하거나 비용이 폭증합니다.
*   `llm_gateway.rs` 내부에는 **Context Pruner(문맥 압축기)** 가 존재합니다. 화면 밖으로 벗어난 버튼, 보이지 않는 투명한 박스 등을 전부 쳐내고(Pruning), 사용자의 명령과 직접 관련이 있을 법한 뼈대만 간추려 LLM에게 전송합니다.

#### 10.2.3 토큰 사용량 모니터링
모든 LLM 응답은 데이터베이스(SQLite)의 일일 사용량 테이블에 저장됩니다. (과금 방어를 위한 서킷 브레이커 설정 포함)

---

## 11. 데이터 지속성 - 로컬 데이터베이스 심층 가이드 (`db.rs`)

LLM은 기억(상태)이 없습니다. Steer OS가 세션을 유지하고 사용자의 맥락을 이어가게 해주는 핵심은 빠르고 독립적인 SQLite 기반의 `db.rs` 모듈입니다.

### 11.1 주요 테이블 구조와 역할 (Schema Definition)

#### 11.1.1 `task_runs` & `task_stage_runs` 테이블
작업의 생애 주기를 완벽하게 트래킹합니다.
*   `run_id`: UUID 형식의 기본 키 (Primary Key)
*   `intent`: 사용자가 원래 지시한 원본 자연어 텍스트
*   `status`: `pending`, `planning`, `executing`, `verifying`, `blocked_on_approval`, `completed`, `failed` 와 같은 상태 머신의 명시적 기록.
추후 시스템 디버깅 시 "어제 실패한 메일 발송 작업은 어느 단계(Execution vs Verifying)에서 뻗었는가?"를 정확하게 감사(Audit)할 수 있습니다.

#### 11.1.2 `nl_approval_decisions` 테이블 (보안 보증)
*   사용자의 결재(결정) 이록을 암호학적으로 남기는 곳입니다.
*   `plan_id`, `action`, `status` (`approved`, `rejected`), `expires_at` 등이 저장되며, 특정 디렉토리에 대한 삭제 권한을 1시간 동안만 부여(Allowlist expiration)하는 등의 시간 기반 임시 승인(Temporal Lease) 메커니즘을 지원합니다.

#### 11.1.3 `routine_candidates` & `recommendations` 테이블
수집기(Collector)가 만들어준 데이터를 보관하는 창고.
*   이 테이블에는 사용자가 "보통 3일 주기로 슬랙의 특정 채널에 리포트를 올린다"는 내용이 `support`(빈도수)와 `confidence`(신뢰도 점수) 밭으로 나뉘어 들어옵니다.
*   `workflow_json`이라는 컬럼에 LLM이 분석한 "이 과정을 자동화할 수 있는 n8n 호환 워크플로우 명세서"가 대기하고 있습니다. 사용자가 수락(Accept)하면 이 JSON이 꺼내져서 시스템 룰로 승격됩니다.

### 11.2 동시성 제어 및 최적화
데이터베이스 락 메커니즘:
*   `busy_timeout(5초)`: 수집기 데몬(`collector_rs`)이 이벤트를 5분에 한 번씩 대량으로 쓸(Bulk Insert) 때, API 서버(`main.rs`)가 데이터베이스 락(Lock)에 부딪혀 죽지 않도록 대기 시간을 허용합니다.
*   **WAL (Write-Ahead Logging) Mode:** SQLite의 동시성 성능을 위해 적용된 모드. 읽기와 쓰기가 서로 차단되지 않고 부드럽게 돌아가도록 설정되어 있습니다.

*(5~11장까지, 시스템 코어부의 실무적 로직과 DB까지 깊숙이 내려온 기술 문서를 추가했습니다. 여기서부터는 운영체제 제어의 최전선(Visual Driver 등)에 대한 작성을 이어나갑니다.)*



---

## 12. Native Control Layer - OS 시스템 통제 심층 분석 (`visual_driver.rs` 및 `macos/`)

운영체제라는 굳건한 성벽 밖에서 LLM이 지시를 내린다면, 그 성문을 열고 들어가 진짜 사용자의 손처럼 마우스를 움직이고 키보드를 두드리는 물리적(Software-physical) 작용점은 바로 `Native Control Layer`입니다. Steer OS는 현재 macOS의 네이티브(Native) 환경에 극도로 최적화된 저수준(Low-level) 바인딩 코드를 운용합니다.

### 12.1 Visual Driver (`visual_driver.rs`)의 핵심 원리
Visual Driver는 이름 그대로 시스템의 '눈(Vision)'과 '손(Action)'을 추상화한 래퍼(Wrapper) 계층입니다.
*   **관측 (Observation):** 현재 화면에 어떤 창이 떠 있는지, 그 창 안에 어떤 버튼과 텍스트 필드가 있는지를 운영체제로부터 읽어옵니다. (스크린샷 캡처 및 UI 트리의 논리적 덤프)
*   **행동 (Action):** 계산된 `(x, y)` 화면 좌표로 마우스 포인터를 순간 이동시켜 클릭하거나, 활성화된 입력창에 키보드 이벤트를 발생시킵니다. 시스템 커널 레벨의 이벤트를 합성(Synthesize)하기 때문에, 매크로나 보안 프로그램이 막기 힘든 진짜 사용자 이벤트와 동일하게 취급됩니다.

### 12.2 macOS 손쉬운 사용 (Accessibility, AX API) 브릿지
화면 렌더링을 뚫고 내부 컴포넌트들을 제어하기 위해, Apple C/Objective-C 기반의 `AXUIElement` 리플렉션 기술이 사용됩니다. `src/macos/accessibility` 폴더 하위 모듈들이 이 결합을 담당합니다.

#### 12.2.1 UI 트리 덤프 (AX Tree Dumping)
LLM이 화면을 완벽하게 이해하고 판단하려면 화면의 시각적인 픽셀 구조를 글자(JSON)로 번역해야 합니다.
1. 운영체제의 `AXUIElementCopyAttributeValues` 함수 등을 호출하여, 현재 화면 최상단에 있는 앱(Frontmost Application)을 타겟팅합니다.
2. 타겟 앱의 윈도우 안에 존재하는 버튼, 텍스트 필드, 체크박스, 스크롤바 등을 트리 검색 엔진(DFS 알고리즘)으로 재귀적(Recursive) 탐색합니다.
3. 이 과정에서 `Role` (컴포넌트 종류: 예 `AXButton`), `Title` (버튼 이름 내용), `Frame` (화면상의 절대 x, y 좌표 및 너비/높이) 정보를 추출해 거대한 트리 형태의 JSON 구조체로 직렬화(Serialize)합니다.

#### 12.2.2 좌표 기반 논리적 클릭 (Heuristic Action Injector)
단순히 마우스를 고정된 픽셀 좌표(`(100, 200)`)로 보내는 전통적인 단순 매크로 방식은 화면 해상도나 창 크기 변화에 극도로 취약합니다. Steer OS는 논리적 요소를 수학적으로 환산하는 방식을 채택했습니다.
1. LLM이 UI 트리를 보고 "전송 버튼 (Node ID: 52)을 누르라"고 JSON 응답으로 지시합니다.
2. `visual_driver`는 메모리에 덤프된 해당 트리의 Node 52의 `Frame` 요소를 확인하여 사각형 영역(`x: 100, y: 200, width: 50, height: 20`) 임을 런타임에 파악해냅니다.
3. 이 사각형 영역의 **정중앙 좌표인 `(125, 210)`** 을 수학적으로 계산합니다.
4. macOS CoreGraphics 내장 객체인 `CGEventCreateMouseEvent`를 호출하여 정확히 이 중앙 픽셀에 `MouseDown` 및 `MouseUp` 왼쪽 클릭 이벤트를 인간의 속도(약 0.05초 간격 딜레이)와 유사하게 발생시킵니다.

### 12.3 AppleScript 시스템 연동 (`applescript.rs`)
기본 AX(Accessibility) UI 트리 API가 커버하지 못하는 글로벌 수준의 OS 제어도 통합됩니다.
*   예: 시스템 볼륨 조절(`Set Volume`), 특정 백그라운드 앱 강제 활성화(`Activate Safari`), 메뉴바 상단 옵션 직접 호출, 앱에 특정 URL 즉시 띄우기 등.
*   `std::process::Command` 객체를 통해 `osascript` 바이너리와 AppleScript 코드를 조합한 프로세스를 스폰(Spawn)하여 OS에 글로벌 지시를 하달합니다.

---

## 13. 백그라운드 수집기 (Collector) 및 애그리게이션 파이프라인 (Aggregation Pipeline)

사용자의 명시적인 명령을 기다리는 수동적 단계를 넘어서서, Steer OS를 사용자 맞춤형 "선제적 예측 비서"로 진화시키는 핵심 심장부입니다.

### 13.1 `collector_rs` (초경량 독립 실행 데몬 시스템)
이 바이너리는 메인 에이전트 실행 루프와 완전히 분리되어 `Local Agent`의 자회사격으로 24시간 백그라운드 환경에서 동작합니다. 데몬 특성상 램 누수나 CPU 스파이크를 방지하기 위해 단일 스레드 기반 I/O(tokio)로 최소한의 자원만 점유합니다.
*   **이벤트 감지 흐름:** macOS의 `CGEventTapCreate` 혹은 시스템 접근성 콜백을 통해 사용자가 어떤 앱 윈도우를 언제 포커스(Activate) 했는지, 마우스를 초당 몇 회 눌렀고 키보드 빈도는 어땠는지 철저하게 메타데이터화합니다.
*   **Zero-Knowledge 필터 (PrivacyGuard):** 수집 시스템은 절대 키로거(Keylogger) 형태의 평문 저장을 허용하지 않습니다. 브라우저의 암호 입력 필드(SecureTextField), 은행 및 증권 앱, 브라우저의 시크릿 탭(Incognito Mode)에서 일어나는 모든 액션과 텍스트 내용은 즉각 버려집니다. 추가적으로 주민번호, 카드번호 등의 패턴은 내부 Regex 엔진을 통과하자마자 Masking 처리(`***`)됩니다.

### 13.2 병합 및 압축 엔진 (Aggregation Ticker)
수집된 방대한 날것(Raw)의 로그 데이터(`events_v2`)를 분석이 가능한 의미론적(Semantic) 메타데이터로 압축하는 `collector_pipeline.rs` 의 메인 루프입니다.

*   **1단계 - 5분 Ticker (Minute Aggregation):** 매 5분 정각에 깨어나 "가장 최근 5분에 발생한 3,500개의 이벤트"를 하나로 묶어 그룹화합니다. "크롬 점유 3.5분, 슬랙 점유 1.5분, 전체 클릭 210회" 라는 단순한 통계 요약표 `minute_aggregates` 1줄로 경량화하여 삽입(Insert)합니다.
*   **2단계 - Daily Ticker (Daily Summary):** 매일 자정 무렵에 하루치 총 데이터를 정산하여 어제 하루 동안 어떤 작업에 가장 시간을 많이 소모했는지 `daily_summaries` 컬럼에 최종 집계 1건으로 보장(Archive)합니다. 이것이 LLM에 전송될 "일간 다이제스트 리포트"의 원료가 됩니다.
*   **3단계 - 가비지 컬렉터 (TTL Manager):** 원본 Raw 로그는 SQLite 용량을 폭증시킬 우려가 있습니다. `events_v2`의 원본 로그는 정확히 7일(환경변수 `STEER_COLLECTOR_RAW_RETENTION_DAYS`로 조절)이 경과하면 자동 실행되는 SQLite 커맨드 연산을 통해 영구 삭제(Hard Delete) 조치됩니다.

---

*(6부 - 핵심인 네이티브 C/Rust 계층 연결부와 데이터 파이프라인 로직에 대한 기술 백서를 무사히 작성했습니다. 이어지는 7부부터는 시스템의 건전성을 뒷받침하는 E2E 자율 테스팅 및 n8n 외부 자동화 도구 통합 편을 서술하겠습니다.)*



---

## 14. 무결성 보장 - End-to-End 테스팅 및 검증 시나리오 (E2E Testing & Verification)

Steer OS 에이전트는 운영체제를 파괴할 수도 있는 권한을 통제하므로, 코드가 조금만 변경되더라도 기존 동작을 파괴하지 않는지(Regression) 완벽히 보증해야 합니다. `tests/`와 `scripts/` 디렉터리에 구축된 방대한 자동화 테스트 인프라는 단순한 단위 테스트(Unit Test)를 넘어섭니다.

### 14.1 E2E 릴리즈 테스트 설계 (`run_gui_regression_pack.sh` 등)
이 테스트 팩은 가상의 빈 화면(격리된 GUI 공간) 띄워놓고 에이전트에게 지시를 내립니다.
*   **시나리오 기반 주입:** "Spotlight를 열어서 계산기를 켜고 100+250을 계산한 뒤, 결과 창을 스크린샷으로 캡처하라"는 일련의 자연어를 투입합니다.
*   **검증자(Verifier) 독립 실행:** 에이전트가 "완료했습니다"라고 보고하면 이 테스트 스크립트는 이를 믿지 않습니다. Python 기반의 독립적인 비전(Vision)/OCR 검증 스크립트가 방금 찍힌 스크린샷을 분석하여 화면에 "350"이라는 글씨가 렌더링되어 있는지 크로스체크합니다.
*   **Fail Fast:** 검증에 실패하거나, 허용되지 않은 폴더(예: `/System`) 외부로 접근하려 하면 즉시 테스트 런을 중단(Abort)하고 에러 리포트를 출력합니다.

### 14.2 AI 자기 주도 벤치마킹 (`run_complex_scenarios.sh`)
LLM의 추론 능력이 릴리즈마다 떨어지는 현상(Model Degradation)을 막기 위한 도구입니다.
*   에이전트에게 10개의 고난이도 복합 문제(예: "현재 실행 중인 크롬 탭을 모두 읽고 쇼핑몰이 켜져 있으면 끄라")를 연속으로 부여합니다.
*   프롬프트가 최적화되었는지, 불필요한 API 콜을 연발하여 토큰 비용이 낭비되지 않았는지 각 단계마다 타임스탬프와 토큰 사용량을 SQLite 데이터베이스(`task_stage_runs`)에 세밀하게 기록하여 이전 버전과 성능을 벤치마킹합니다.

### 14.3 실시간 알림 리포트 (Telegram / Slack 통보)
*   복합 테스트가 밤새 무인으로 실행된 후, 혹은 실제 사용자가 에이전트에게 엄청나게 긴 루틴(예: "퇴근 전 데일리 리포트 정리해서 메일 보내고 시스템 종료해")을 시켰을 경우의 피드백 채널입니다.
*   테스트나 실행이 완료되면 즉시 `telegram_notifier.py` 나 Rust 내장 웹훅 발송기가 "시나리오 성공: 9/10, 토큰 사용: 15,000, 1건의 UI 매칭 실패 발생" 과 같은 인간이 읽기 편한 요약 브리핑을 모바일 메신저로 쏴줍니다. (이를 통해 "내가 명령한 게 멈췄나?" 하는 불안감을 해소합니다.)

---

## 15. N8N 워크플로우 엔진 통합 및 스마트 루틴 고도화 (Workflow Automation & Routine Scheduling)

Steer OS의 백그라운드 수집기(Collector)가 "당신은 매주 월요일 아침 9시에 엑셀을 켜고 작주 실적을 취합하는 패턴이 있습니다"라고 알아냈다면, 이를 사람의 개입 없이 실제 '스케줄된 자동화 코드'로 변환하는 외부 브릿지 모듈입니다.

### 15.1 n8n 플랫폼과의 시너지
n8n은 노드 기반(Node-based)의 오픈소스 워크플로우 자동화 도구입니다. Steer OS는 n8n과 결합하여 약점을 보완합니다.
*   **에이전트의 약점:** 에이전트는 무언가를 "수동"으로 시켜야 트리거(Trigger)됩니다.
*   **n8n의 강점:** 매주 월요일 9시(Cron 스케줄링)나 특정 이메일이 왔을 때(WebHook) 스스로 시작될 수 있습니다.
*   **연계(Integration):** n8n에서 "특정 이메일 도착"을 트리거로 잡은 뒤, n8n의 HTTP Request 노드가 로컬에 떠 있는 Steer OS의 API (`POST /api/agent/execute`)를 찔러 "이메일 첨부파일 다운로드 받아서 데스크탑 폴더에 정리해"라는 자연어 명령을 하달합니다.

### 15.2 Routine Candidate의 자동 JSON 컴파일링
*   사용자가 `Routine Dashboard` 화면에서 수집기가 제안한 루틴을 "수락(Approve)"하면, Steer OS Core는 그 일련의 행동들을 n8n이 이해할 수 있는 형태의 **n8n Workflow JSON 포맷**으로 실시간 트랜스파일(Transpile) 해냅니다.
*   이 생성된 JSON은 자동으로 n8n 서버에 Deploy(배포)되며, 사용자는 코드를 한 줄도 짜지 않고 자신의 일상을 자동화하는 봇(Bot)을 시스템에 영구적으로 안착시키게 됩니다.

### 15.3 AI Digest 메커니즘 
단순한 조회를 넘어, 정보의 정제를 담당합니다.
사용자가 브라우저나 문서를 읽고 있을 때 백그라운드에서 해당 텍스트들을 모아둡니다(`events_v2`).
이후 사용자가 "방금 내가 1시간 동안 조사한 논문 자료들을 3글 요약해서 텔레그램으로 보내줘"라고 명령하면,
*   에이전트는 데이터베이스에서 지난 1시간 창 활성화(Window Focus) 기록 중 텍스트에 해당하는 부분만 간추려 `llm_gateway.rs`로 즉시 넘기고, 
*   응답을 받아 텔레그램 API로 푸시-알림(Push Notification)을 보내는 10초 컷 다이제스트 파이프라인을 가동합니다.

---

*(제 7부-자동화 테스팅 및 n8n 통합 부분 작성을 마무리했습니다. 이어서 문서의 대단원을 장식할 최종 결론(Conclusion) 파트와 부록(Appendix - 주요 환경 변수 정리)을 이어붙여 거대한 문서를 깔끔하게 종결짓겠습니다.)*



---

## 16. 결문 (Conclusion) 및 프로젝트의 의의

Steer OS 시스템은 아직 기술적 여명기에 있는 인공지능이 인간의 로컬 워크스페이스(Workspace)에 어떻게 안전하고 유능하게 진입할 수 있는지를 보여주는 실증적인 아키텍처입니다.
*   단순한 "질문-답변"형 AI 인스턴스를 넘어,
*   LLM의 언어적 추론 능력과 Rust/C의 네이티브 통제력을 결합하여 환각 현상(Hallucination)에 의한 OS 붕괴를 원천 차단하고,
*   사용자의 키보드와 마우스를 대신 쥐고 움직여주는 **'진정한 의미의 Agent(대리인)'**로 기능합니다.

이 가이드 문서에 묘사된 파이프라인(Plan -> Preflight -> Appove -> Execute -> Verify)은 어떤 플랫폼이나 LLM의 버전 교체에도 흔들리지 않을 견고한 소프트웨어 공학적 유산(Legacy)이 될 것입니다.

---

## 17. 부록 (Appendix): 주요 환경 변수 스펙 시트 (Environment Variables)

Steer OS `.env` 설정에 사용되는 코어 변수 사전입니다. 서버 트러블슈팅이나 정책 변경 시 최우선으로 참고하십시오.

### 17.1 에이전트 구동 및 LLM 관련
*   `OPENAI_API_KEY`: GPT-4o 등 OpenAI 모델 접근용 필수 키.
*   `STEER_MODEL_NAME`: 에이전트의 주력 두뇌로 쓸 모델 이름 (기본값: `gpt-4o`).
*   `AGENT_PORT`: Rust Core Axum 서버가 리스닝할 포트 번호 (기본값: `5680`).
*   `STEER_API_ALLOW_NO_KEY`: 개발 환경에서 프론트엔드가 토큰 없이 코어 API에 접근할 수 있게 허용하는 플래그 (`1` or `0`).

### 17.2 데이터 파이프라인 (Collector) 관련
*   `STEER_DB_PATH`: 시스템 SQLite 데이터베이스 파일들이 저장될 절대 경로 (기본값: `~/.local/share/steer/`).
*   `STEER_COLLECTOR_AGG_INTERVAL_SEC`: 이벤트 압축을 수행할 백그라운드 타이머의 주기 초 단위 (기본값: `300` -> 5분).
*   `STEER_COLLECTOR_RAW_RETENTION_DAYS`: `events_v2` 원시 로그를 보존할 최대 일수 (기본값: `7`).
*   `STEER_COLLECTOR_SUMMARY_RETENTION_DAYS`: `minute_aggregates` 등의 압축된 요약본을 보관할 일수 (기본값: `30`).

### 17.3 보안 (Security) 및 로깅 관련
*   `RUST_LOG`: 시스템 데몬의 로깅 수준 디버깅용 제어. (`info`, `debug`, `steer=trace` 등)
*   `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`: 주요 시나리오 테스트 결과나 일일 요약을 쏠 메신저 봇 키.

---
**[문서 끝 (End of Steer OS Comprehensive Whitepaper)]**



---

## [별첨 심화 파트] Steer OS 딥다이브 (Deep Dive Series)

사용자의 "계속" 요청에 따라, 기존의 1~17장 통합 가이드에서 구체적으로 다루지 못했던 핵심 기술 모듈들의 밑바닥(Under the hood) 설계를 파트별로 심층 해부(Deep Dive)합니다.

---

## 18. 심층 해부 1: LLM 프롬프트 엔지니어링 및 컨텍스트 관리 (Prompt & Context)

Steer OS가 "그냥 텍스트를 내뱉는 AI"가 아닌 "내 명령을 따르는 확실한 디바이스 제어기"로 작동하기 위해서는 고도의 프롬프트 엔지니어링(Prompt Engineering) 메커니즘이 강제되어야 합니다. `llm_gateway.rs` 모듈을 중심으로 일어나는 기법들을 분석합니다.

### 18.1 거대 시스템 프롬프트 (Core System Prompt)의 해부
모든 요청이 들어올 때마다 에이전트에게 주입되는 베이스 프롬프트(Base Prompt)는 다음과 같은 3단계 레이어 구조를 가집니다.

1.  **Persona & Directive (페르소나 및 지시사항):**
    *   "당신은 macOS 시스템을 제어하는 최고 권한의 AI 에이전트(Steer OS)입니다."
    *   "사용자의 질문에 언어적으로만 대답하지 말고, 반드시 제공된 Tool(도구)들을 호출하여 물리적으로 문제를 해결하십시오."
    *   "행동을 취하기 전에는 항상 화면을 관찰(Observe)하고, 행동 후에는 반드시 결과를 검증(Verify)해야 합니다."

2.  **State Mappings (상태 주입):**
    *   단순 프롬프트뿐만 아니라 실시간 OS 상태가 주입됩니다.
    *   "현재 시간: 2026-02-23 21:50:18"
    *   "현재 활성화된 앱: Safari"
    *   "현재 클립보드 내용: (비어있음)"
    이러한 **동적 변수 치환(Dynamic Variable Substitution)** 을 통해 LLM은 현재 자신이 어떤 환경에 놓여있는지를 인지하게 됩니다.

3.  **JSON Schema Enforcement (구조화 강제):**
    *   에이전트는 절대로 일반 텍스트(Plain Text)로만 응답해서는 안 됩니다.
    *   `ActionSchema`라는 엄격한 JSON 규격이 프롬프트 마지막에 첨부되며, LLM의 응답은 항상 이 JSON 배열 포맷을 따라야 한다고 지시받습니다. (이 과정에서 OpenAI의 Function Calling API나 Structured Output 기능이 적극 활용됩니다.)

### 18.2 UI 트리 압축 알고리즘 (Context Window Pruning)
LLM에게 화면 정보를 전달하는 것은 매우 까다로운 작업입니다. 일반적인 1080p 화면 하나에도 수만 개의 UI 노드(Node)가 존재할 수 있습니다. 이를 모두 텍스트로 바꾸면 수십만 토큰이 소모되어, 비용 폭탄과 응답 지연(Timeout)을 연발하게 됩니다.

Steer OS는 `Context Pruner`를 가동하여 이를 다음과 같이 해결합니다.
*   **Off-screen 제거:** 화면 좌표(x, y) 상 렌더링 범위 바깥에 있는 노드(스크롤해서 내려가야 보이는 부분 등)는 1차적으로 텍스트 덤프에서 삭제(Drop)합니다.
*   **투명하거나 크기가 0인 노드 제거:** 시스템 내부 레이아웃을 잡기 위해 보이지 않게 존재하는 더미 박스(Dummy Box)들은 제거합니다.
*   **중요도(Weight) 기반 샘플링:** 사용자가 "전송 버튼 찾아줘"라고 했다면, 자연어 유사도(Embedding) 비교를 통해 텍스트에 '전송', '보내기', 'Submit' 등의 단어가 포함된 노드와 그 부모 노드 위주로만 UI 트리를 잘라내어 컨텍스트 윈도우 크기를 90% 이상 획기적으로 압축합니다.

### 18.3 자기 교정(Self-Correction) 및 Retry 루프
아무리 강력한 LLM이라도 가끔 JSON 괄호를 빼먹거나 존재하지 않는 Tool을 호출할 때가 있습니다. (Hallucination)
1.  LLM이 응답을 반환하면 `llm_gateway.rs` 모듈 내의 파서(Parser)가 `serde_json`을 통해 역직렬화(Deserialize)를 시도합니다.
2.  이때 에러가 발생하면 파서는 패닉(Panic)을 일으켜 뻗어버리는 대신, 에러 메시지 자체를 문자열로 직렬화합니다.
    *(예: "Error: Missing parenthesis at line 4", "Error: Tool 'magic_click' does not exist")*
3.  그리고 이 에러 메시지를 다시 LLM에게 그대로 전송하며 "방금 네가 보낸 JSON이 이런 에러를 일으켰으니 다시 고쳐서 보내라"고 지시(Self-Correction Prompt)합니다. 
4.  이 과정은 백그라운드에서 최대 3회 반복되며, 대부분의 경우 LLM은 자신의 실수를 깨닫고 완벽한 코드를 다시 짜서 보냅니다. 사용자는 이 재시도 과정을 전혀 눈치채지 못합니다.



---

## 19. 심층 해부 2: 상태 머신과 실행 제어기 (State Machine & Executor Controller)

명령이 하달되었을 때 에이전트가 도중에 멈추거나 뻗지 않고(Crash-free) 끝까지 태스크를 완수하게 만드는 원동력은 `execution_controller.rs` 내부에 구현된 강력한 비동기 상태 머신(Finite State Machine, FSM)입니다.

### 19.1 TaskRun 상태 전이도 (State Transition Map)
모든 태스크(`TaskRun`)는 생명 주기 동안 엄격한 상태 변화를 거칩니다.
1.  **`PENDING`:** 사용자가 명령을 API로 전송한 직후의 대기열 상태.
2.  **`PLANNING`:** LLM과 통신하여 실행 계획(JSON)을 수립 중인 상태.
3.  **`EXECUTING`:** 계획된 개별 Step들(Action)을 하나씩 꺼내어 실행하는 상태.
4.  **`BLOCKED_ON_APPROVAL`:** 치명적인 액션(Critical Action)을 실행하기 전, 사용자의 명시적 승인(Write Lock 해제)을 기다리며 **실행이 일시 정지(Yield)** 된 상태.
5.  **`VERIFYING`:** 하나의 Step이 끝난 후, 원래 의도한 대로 OS 상태가 변했는지 검증하는 상태.
6.  **`COMPLETED` / `FAILED`:** 검증을 마친 최종 종착지.

### 19.2 비동기 실행 루프 (The Asynchronous Execution Loop)
`execution_controller.rs`의 런타임은 `tokio::spawn` 퓨처(Future) 위에서 동작합니다.
*   가장 큰 특징은 이 루프가 블로킹(Blocking)되지 않고, 언제든지 일시 정지(Suspend)하고 다른 작업을 처리하다가 다시 돌아올 수 있는 코루틴(Coroutine) 구조를 취한다는 것입니다.
*   **Approval Interrupt (승인 인터럽트):** `BLOCKED` 상태에 빠지면, 워커 스레드는 해당 태스크를 메모리에 들고 잠든(Sleep) 상태가 아닙니다. 코루틴 컨텍스트 자체를 데이터베이스(`task_runs` 테이블)에 영속화 시키고 스레드를 즉시 반환(Return)하여 시스템 리소스(RAM/CPU) 점유율을 0%로 떨어뜨립니다.
*   시간이 지나 사용자가 UI에서 "승인(Approve)" 버튼을 누르면, API 서버가 데이터베이스의 상태를 `EXECUTING`으로 바꾸고 다시 런타임 큐에 집어넣어 작업을 완벽히 재개(Resume)시킵니다.

### 19.3 안전한 재시도 (Idempotent Retry) 및 오류 복구 (Recovery)
OS 네이티브 제어는 변수가 많습니다. (예: 브라우저 로딩이 늦게 완료됨, 알림창이 갑자기 떠서 버튼을 가림)
*   에이전트가 마우스를 클릭했지만 검증(`VERIFYING`) 단계에서 "원하는 결과 창이 뜨지 않았다"고 판별되면, 시스템은 즉시 `FAILED` 처리하지 않습니다.
*   실패한 단계와 에러 메시지를 수집하여 `Recovery Prompt`를 생성합니다. "방금 클릭했는데 이런 에러가 났어. 다른 방법을 찾아봐." 라는 지시와 함께 LLM에게 재수립(Re-plan)을 요청합니다.
*   이러한 자가 치유(Self-healing) 로직 덕분에 Steer OS는 돌발적인 OS 팝업 창이나 로딩 지연 현상을 맞이하더라도 유연하게 우회 경로를 탐색해 냅니다.



---

## 20. 심층 해부 3: macOS Accessibility (AXUIElement) 딥다이브

AI가 화면을 조작하려면 픽셀(Pixel) 좌표 너머의 논리적 계층(DOM과 같은 구조)을 읽을 수 있어야 합니다. Steer OS는 macOS에 내장된 `Accessibility API`를 C 언어 바인딩을 통해 Rust로 끌어와 완벽하게 추상화했습니다.

### 20.1 CoreFoundation과 Rust의 완벽한 결합
`src/macos/accessibility.rs`는 `core-foundation` 및 `core-graphics` 크레이트(Crate)를 사용하여 macOS 고유의 C API를 안전한 Rust 코드로 감쌉니다.
*   **메모리 관리 (Memory Safety):** C 계층의 포인터 오류(Segfault)나 메모리 누수(Leak)를 막기 위해 Rust의 `Drop` 트레이트(Trait)와 소유권(Ownership) 모델을 적극 적용했습니다. C에서 가져온 `AXUIElementRef`는 Rust 블록을 벗어나는 순간 자동으로 해제됩니다.
*   **타입 캐스팅 (Type Casting):** Apple의 `CFString`, `CFBoolean`, `CFNumber` 등의 난해한 데이터 구격을 Rust의 순수 `String`, `bool`, `f64` 타입으로 런타임에 안전하게 통역해 줍니다. 이렇게 변환된 평문 데이터 위에서만 LLM 프롬프트가 동작합니다.

### 20.2 재귀적 트리 생성기 (Recursive Tree Builder)
윈도우 창 안에 어떤 버튼이 있는지 알려주는 'API 함수'는 세상에 존재하지 않습니다. 우리가 가진 것은 오직 "내가 가진 자식(Children) 노드들을 줘"라는 쿼리뿐입니다.
1. `AXSystemWideElement`를 획득합니다. (전체 시스템의 최상위 루트 노드)
2. `AXFrontmostApp` 속성을 쿼리하여 지금 사용자 눈앞에 띄워진 가장 앞쪽의 앱 핸들(Handle)을 얻어냅니다.
3. 해당 앱의 `AXMainWindow`를 가져옵니다.
4. 여기부터 루트 삼아 **깊이 우선 탐색 (DFS)** 기반으로 모든 자식 요소(`AXChildren`)를 순회합니다. 이 과정에서 `Role`, `Subrole`, `Title`, `Value` 등의 핵심 메타데이터를 추출합니다.

### 20.3 보정된 프레임 계산 로직 (Corrected Frame Math)
`Accessibility API`가 반환하는 요소의 좌표(`AXPosition`)와 크기(`AXSize`)는 모니터의 레티나(Retina) 디스플레이 배율이나 다중 모니터 설정에 따라 엄청난 오차를 가질 수 있습니다.
*   **좌표계 변환 (Coordinate Swap):** macOS는 모니터 좌측 상단을 (0,0)으로 보기도 하고, 좌측 하단을 (0,0)으로 보기도 하는 등 API마다 혼용된 좌표계를 씁니다. Steer OS 내부의 수학 유틸리티는 이를 철저히 계산하여 일관된 화면 기반 픽셀 좌표계로 통합 보정합니다.
*   에이전트가 "저장 버튼을 누르겠다"고 선언하면 이 보정된 좌표의 정중앙에 마우스 커서 이벤트를 쏴서 백발백중으로 버튼을 클릭해 냅니다.



---

## 21. 심층 해부 4: SQLite 마이그레이션과 상태 관리 (Database Migrations & Persistance)

Steer OS는 서버-클라이언트 모델을 차용하지만, 완전히 로컬에 격리된 환경에서 동작하므로 `PostgreSQL` 같은 무거운 DBMS를 쓸 수 없습니다. 대신 가볍고 충돌에 강한 `SQLite`를 극한까지 튜닝하여 비동기 환경에 맞게 아키텍처를 설계했습니다.

### 21.1 마이그레이션 전략 (Schema Evolution)
에이전트 시스템은 개발이 진행됨에 따라 필연적으로 데이터베이스 스키마가 끊임없이 바뀝니다 (새로운 피처 추가, 테이블 정규화 등).
*   `db.rs` 내부에는 하드코딩된 `.sql` 파일 묶음 대신, **코드 레벨 마이그레이션 매니저**가 내장되어 있습니다.
*   에이전트가 켜질 때마다 `PRAGMA user_version`을 쿼리하여 현재 사용자의 DB 버전을 확인합니다.
*   만약 코드 상의 최신 버전(예: `v5`)보다 로컬 DB 파일(`steer.db`) 버전(예: `v2`)이 낮을 경우, 시스템은 자동으로 `v3 -> v4 -> v5` 업그레이드 스크립트를 순차적으로 실행(Apply)하여 스키마를 최신 상태로 원클릭 패치합니다. 사용자는 아무런 수동 복구(Migration CLI) 작업 없이 최신 기능에 호환되는 DB를 유지할 수 있습니다.

### 21.2 WAL 분리 모드와 연결 풀링 (Connection Pooling)
*   **WAL (Write-Ahead Logging) Mode:** 수집기(Collector)가 백그라운드에서 분당 천 개의 이벤트를 쓸 때, 프론트엔드 UI가 루틴 목록을 읽어오면 일반적인 SQLite는 `SQLITE_BUSY` 에러(DB Lock)를 토해내고 뻗어버립니다. 이를 막기 위해 WAL 모드가 활성화되어 읽기와 쓰기가 서로 영향을 주지 않는 동시성을 보장합니다.
*   **`r2d2_sqlite` 연결 풀 (Pool):** 웹 서버(`Axum`)의 비동기 처리 구조에 맞춰 여러 개의 DB 커넥션을 미리 열어두고 풀(Pool)에서 돌려 씁니다. 수없이 많은 HTTP 요청이 오더라도 병목 없이 쿼리를 실행할 수 있습니다.

### 21.3 `serde_json`를 이용한 NoSQL 하이브리드 패턴
관계형 테이블(Relational Table)만으로는 LLM이 내려주는 "동적으로 변하는 JSON 페이로드"를 다루기 벅찹니다.
*   따라서 `TaskPlan` 내부의 `ActionSchema`, 또는 `RoutineCandidate`의 `workflow_json` 등의 컬럼들은 SQLite의 `TEXT` 타입으로 지정하여, 실제로는 NoSQL 문서(Document)처럼 원시 JSON 텍스트를 통째로 집어넣고 꺼낼 때 `serde_json`으로 파싱(직렬화/역직렬화)하는 유연성을 확보했습니다.



---

## 22. 심층 해부 5: 사용자 인터페이스와 데스크톱 앱 아키텍처 (React & Tauri)

강력한 백엔드 엔진의 입출력을 담당하는 UI는 단순한 텍스트 터미널이 되어서는 안 됩니다. Steer OS는 React 기반의 유려한 웹 인터페이스와 Tauri의 크로스 플랫폼 데스크톱 바인딩을 결합하여, 네이티브 앱과 같은 사용자 경험(Native-like UX)을 제공합니다.

### 22.1 React 애플리케이션 아키텍처 (`web/src/`)
프론트엔드는 전역 상태(Global State) 관리와 실시간 양방향 통신에 최적화되어 있습니다.
*   **컴포넌트 주도 개발 (CDD):** `ChatView`, `RoutineDashboard`, `ApprovalDialog` 등 기능별로 컴포넌트가 격리되어 있습니다. 특히 폴링(Polling) 컴포넌트는 타이머를 통해 백엔드의 `/api/agent/execute` 상태(PENDING -> EXECUTING -> COMPLETED)를 1초마다 실시간으로 가져와 화면을 리렌더링(Re-render) 합니다.
*   **LLM Streaming 렌더러:** 에이전트가 긴 생각을 반환할 때, 서버 전송 이벤트(SSE, Server-Sent Events) 혹은 청크 단위의 웹소켓을 받아 타자기가 쳐지듯 유려하게 화면에 스트리밍 애니메이션을 부여하는 커스텀 훅(Hook)이 내장되어 있습니다.

### 22.2 Tauri를 이용한 네이티브 OS 결합 (`src-tauri/`)
웹앱(Web application)만으로는 시스템 트레이(상단 메뉴바 아이콘)나 OS 내부 디렉토리 제어, 단축키 시스템을 완벽히 흡수할 수 없습니다. 
*   이를 위해 `Tauri` 데스크톱 프레임워크를 사용했습니다. Electron보다 메모리 타격이 훨씬 적은 Rust 기반의 백엔드 프레임워크입니다.
*   빌드 스크립트(`scripts/rebuild_and_deploy.sh`)를 실행하면, 웹 프론트엔드가 HTML/JS 정적 에셋으로 빌드되어 Tauri 앱(`.app`) 내부에 임베드(Embedded) 됩니다.
*   **백그라운드 포팅 (Sidecar Pattern):** 앱이 실행되면, 패키지 내부에 숨겨져 있는 가벼운 `core` 서버(Rust)가 사이드카 프로세스로 보이지 않게 스폰(Spawn)됩니다. 프론트엔드는 브라우저 환경이 아니라 `localhost:5680` 로 내부 루프백 통신만 수행하여 보안과 성능을 모두 확보합니다.
*   단축키(Global Hotkey) 바인딩: 사용자가 언제 어느 화면에 있든 `Cmd+Shift+Space` 등을 눌러 챗 모달(Overlay Modal)을 즉시 팝업 시킬 수 있는 커스텀 네이티브 리스너가 연동되어 있습니다.

---

[끝. 추가 Deep Dive 시리즈가 모두 갱신되었습니다.]
