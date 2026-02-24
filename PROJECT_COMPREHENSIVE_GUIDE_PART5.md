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
