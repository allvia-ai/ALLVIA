# STEER OS 기술스택/아키텍처 의사결정 플레이북 (고도화)

작성일: 2026-02-24  
대상: 기술면접, 기업평가, 내부 설계 리뷰

## 1. 이 문서의 목적
- "무엇을 썼다"가 아니라 "왜 이 선택이 지금 최적이었는가"를 설명한다.
- 각 기술 선택을 **대안 비교 + 배제 근거 + 리스크 수용 + 전환 트리거**로 기록한다.
- 코드/운영 근거가 있는 결정만 남긴다.

---

## 2. 의사결정 품질 기준 (Fitness Functions)
모든 선택은 아래 기준으로 평가한다.

1. 안전성: 위험 동작이 기본 차단되는가  
2. 검증성: 완료를 로그가 아니라 증거로 판정 가능한가  
3. 복구성: 실패 후 재개/복구가 가능한가  
4. 운영성: 배포/롤백/헬스체크가 재현 가능한가  
5. 진화성: 요구 변화 시 교체 비용을 감당 가능한가

---

## 3. 글로벌 제약

- 제품 목표: 로컬 OS 실행 에이전트 + 검증 가능한 완료
- 보안 원칙: LLM 출력은 untrusted, 정책 게이트 필수
- 단계: 부트캠프 과제에서 상용화 후보로 진화 중
- 현실 제약: 팀 규모 제한, 빠른 반복 개발 필요, macOS 중심 구현

---

## 4. 핵심 의사결정 (ADR Mini)

## ADR-01. Core 언어를 Rust로 선택
### Context
- 문제: 로컬 OS 제어 + 장기 실행 + 정책 강제 + 안정성 동시 달성
- 제약: 런타임 안전성, 네이티브 제어 결합, 성능

### Options
- A) Rust  
- B) Python + FastAPI  
- C) Node.js + NestJS  
- D) Go

### Decision
- A) Rust

### Why Not Others
- Python/Node: 개발속도는 유리하지만 네이티브 제어와 장기 안정성에서 런타임 리스크가 커짐.
- Go: 충분히 경쟁력 있으나 현재 코드자산/팀 러닝커브/모듈 결합도 측면에서 Rust가 더 낮은 전환비용.

### Trade-offs Accepted
- 팀 온보딩 난이도 증가
- 구현 속도 일부 희생

### Mitigation
- 모듈 분리(`controller`, `policy`, `verification`, `db`)
- 스크립트/문서로 운영 반복작업 표준화

### Revisit Trigger
- 신규 팀원의 온보딩 리드타임이 지속적으로 과도할 때
- 로컬 제어 비중이 낮아지고 클라우드 API 조합 비중이 압도적일 때

### Evidence
- `core/Cargo.toml`, `PROJECT_BIBLE.md`

---

## ADR-02. Runtime을 Tokio로 선택
### Context
- API 서버, 외부 연동, 실행 루프를 동시에 다뤄야 함.

### Options
- A) Tokio  
- B) std thread 기반 수동 동시성  
- C) async-std

### Decision
- A) Tokio

### Why Not Others
- std thread 수동 설계는 복잡성/버그 리스크 증가.
- async-std는 생태계 성숙도/호환성 측면에서 Tokio 대비 불리.

### Trade-offs
- async 디버깅 난이도

### Mitigation
- retry/resume/logging 표준화

### Revisit Trigger
- 특정 I/O 패턴에서 Tokio 병목이 반복적으로 확인될 때

### Evidence
- `core/Cargo.toml`, `core/src/execution_controller.rs`

---

## ADR-03. API 서버를 Axum으로 선택
### Context
- 로컬 루프백 API로 UI/CLI/자동화 엔진을 연결해야 함.

### Options
- A) Axum  
- B) Actix-web  
- C) Warp/Rocket

### Decision
- A) Axum

### Why Not Others
- Actix는 성능 강점이 있으나 현재 요구에서 Axum의 타입/미들웨어 균형이 더 적합.
- Warp/Rocket은 팀 익숙도/운영 표준 관점에서 우선순위가 낮음.

### Trade-offs
- 고성능 튜닝 여지는 Actix 대비 제한될 수 있음

### Mitigation
- 로컬 API 모델 최적화 + 병목 발견 후 선택적 튜닝

### Revisit Trigger
- API P95가 목표치 미달이 반복되고 프레임워크 레벨 병목이 확정될 때

### Evidence
- `core/src/api_server.rs`, `docs/ARCHITECTURE.md`

---

## ADR-04. 저장소를 SQLite(rusqlite bundled)로 시작
### Context
- 로컬 퍼스트 제품에서 설치/재현성/속도 중요.

### Options
- A) SQLite bundled  
- B) PostgreSQL  
- C) MySQL/DuckDB

### Decision
- A) SQLite bundled

### Why Not Others
- 서버형 DB는 초기 운영 복잡도와 배포 마찰이 큼.
- 현재 단계에서 동시성/규모 요구가 SQLite 한계를 아직 강하게 넘지 않음.

### Trade-offs
- 멀티테넌시/원격 협업 확장에 제약

### Mitigation
- 스키마/상태전이 중심 설계로 추후 DB 분리 가능성 확보

### Revisit Trigger
- 동시 사용자/원격 협업 요구가 제품 핵심으로 전환될 때

### Evidence
- `core/Cargo.toml`, `core/src/db.rs`, `docs/ROLLOUT_CHECKLIST.md`

---

## ADR-05. Desktop Shell을 Tauri 2로 선택
### Context
- 로컬 OS 제어 제품은 데스크톱 앱 배포가 필요.

### Options
- A) Tauri  
- B) Electron  
- C) Pure Web

### Decision
- A) Tauri

### Why Not Others
- Electron은 메모리/번들 오버헤드가 큼.
- Pure Web은 OS 네이티브 제어 한계.

### Trade-offs
- Tauri 생태계 의존 및 플랫폼별 빌드 관리 필요

### Mitigation
- 배포 스크립트/런북 표준화

### Revisit Trigger
- 크로스플랫폼 네이티브 기능 요구가 Tauri 한계를 지속 초과할 때

### Evidence
- `web/src-tauri/Cargo.toml`, `docs/BUILD_DEPLOY_RUNBOOK.md`

---

## ADR-06. UI를 React + TypeScript + Vite로 선택
### Context
- 운영자 UI를 빠르게 반복하고 API 상태를 안정적으로 표시해야 함.

### Options
- A) React+TS+Vite  
- B) Next.js  
- C) Vue/Svelte

### Decision
- A) React+TS+Vite

### Why Not Others
- Next.js SSR 강점은 로컬 데스크톱 UI에서 가치가 제한적.
- Vue/Svelte 전환은 팀 학습/마이그레이션 비용 증가.

### Trade-offs
- 상태 복잡도 증가 시 구조 관리 필요

### Mitigation
- React Query + 타입 + 컴포넌트 분리

### Revisit Trigger
- UI 복잡도가 급증해 현재 패턴으로 일관성 유지가 어려울 때

### Evidence
- `web/package.json`, `web/src/lib/api.ts`

---

## ADR-07. UI primitives로 Radix + Tailwind + CVA
### Context
- 접근성과 커스터마이징을 둘 다 잡아야 함.

### Options
- A) Radix+Tailwind+CVA  
- B) MUI  
- C) Chakra/Ant

### Decision
- A) Radix+Tailwind+CVA

### Why Not Others
- 완성형 프레임워크는 빠르지만 장기적으로 디자인/변형 제어가 제약될 수 있음.

### Trade-offs
- 컴포넌트 설계 책임이 팀에 남음

### Mitigation
- 변형 규칙(CVA), 재사용 UI 패턴 통일

### Revisit Trigger
- 컴포넌트 중복/파편화가 관리 불가 수준이 될 때

### Evidence
- `web/package.json`

---

## ADR-08. 로컬 제어를 macOS native bindings + AppleScript로 구현
### Context
- 브라우저 밖 OS 작업까지 자동화해야 함.

### Options
- A) CoreGraphics/Accessibility + AppleScript  
- B) Browser automation only  
- C) AppleScript only

### Decision
- A) 하이브리드 네이티브 제어

### Why Not Others
- 브라우저 전용은 OS 전체 작업 자동화 불가.
- AppleScript-only는 세밀한 제어/복구성에서 한계.

### Trade-offs
- macOS 종속성 증가

### Mitigation
- macOS 전용 로직 모듈 격리

### Revisit Trigger
- Windows/Linux 확장이 매출 핵심 요구가 될 때

### Evidence
- `core/src/macos/*`, `core/src/applescript.rs`

---

## ADR-09. 워크플로우 오케스트레이션에 n8n 도입
### Context
- 외부 SaaS 연동 자동화를 빠르게 실험해야 함.

### Options
- A) n8n  
- B) Temporal  
- C) 직접 워크플로우 엔진 구현

### Decision
- A) n8n (docker 기본)

### Why Not Others
- Temporal은 강력하지만 초기 설계/운영 비용이 큼.
- 직접 구현은 고위험/고비용.

### Trade-offs
- 장기 트랜잭션/대규모 오케스트레이션에서 한계 가능

### Mitigation
- retry/backoff/reconcile로 운영 안정성 보강

### Revisit Trigger
- SAGA급 보장/대규모 장기 워크플로우 요구가 반복될 때

### Evidence
- `docker-compose.yml`, `core/src/n8n_api.rs`

---

## ADR-10. LLM은 OpenAI 중심 + 검증/복구 계층 결합
### Context
- 추론 품질과 개발 속도를 동시에 확보해야 함.

### Options
- A) OpenAI 중심  
- B) 로컬 LLM 전면  
- C) 멀티벤더 동시 운영

### Decision
- A) OpenAI 중심, fallback 경로 보유

### Why Not Others
- 로컬 전면은 품질/운영 복잡도 리스크.
- 멀티벤더 동시 운영은 운영복잡도 급상승.

### Trade-offs
- 외부 벤더 비용/의존성

### Mitigation
- context pruning, 정책 검증, evidence gate, fallback 경로

### Revisit Trigger
- 비용 구조 악화 또는 벤더 리스크가 제품 신뢰도를 해칠 때

### Evidence
- `core/src/llm_gateway.rs`, `core/src/context_pruning.rs`, `core/Cargo.toml` (fastembed 제거 코멘트)

---

## ADR-11. 실행 신뢰성을 위한 상태모델: resume token + approval/manual states
### Context
- 자동실행 중 중단/승인/정책차단을 안전하게 처리해야 함.

### Options
- A) 상태 기반 재개 모델  
- B) 실패 시 전체 재실행  
- C) 수동 복구

### Decision
- A) 상태 기반 재개 모델

### Why Not Others
- 전체 재실행은 side effect 중복 위험.
- 수동 복구는 운영비용 급증.

### Trade-offs
- 상태머신 복잡도 증가

### Mitigation
- token shape 검증 + step range 검증 + 상세 로그

### Revisit Trigger
- 상태 복잡도 폭증으로 디버깅 비용이 제품 가치보다 커질 때

### Evidence
- `core/src/execution_controller.rs`, `core/src/api_server.rs`

---

## ADR-12. idempotency/reconcile: claim token + TTL + workflow_provision_ops
### Context
- 외부 연동에서 중복 생성/유령 상태 방지 필요.

### Options
- A) claim/reconcile 모델  
- B) 낙관적 재시도

### Decision
- A) claim/reconcile

### Why Not Others
- 낙관적 재시도는 중복/정합성 문제를 운영으로 떠넘김.

### Trade-offs
- DB 상태전이/정리 로직 복잡화

### Mitigation
- stale claim 회수, reconcile 루프, 상태 인덱싱

### Revisit Trigger
- reconcile 비용이 커지고 운영 복잡도가 임계점 도달 시

### Evidence
- `core/src/recommendation_executor.rs`, `core/src/db.rs`

---

## ADR-13. 보안 아키텍처: fail-closed 성향(정책/아웃바운드/릴리즈 게이트)
### Context
- 에이전트의 오작동 비용이 큼.

### Options
- A) Fail-closed 중심  
- B) Fail-open + 사후 모니터링

### Decision
- A) Fail-closed 중심

### Why Not Others
- 사후 탐지는 사고 이후 대응이라 비용이 큼.

### Trade-offs
- 초기 사용자 경험 마찰(차단/승인 요청 증가)

### Mitigation
- preflight/fix, 명확한 차단 사유, 재개 토큰 UX

### Revisit Trigger
- 차단율이 과도해 업무 완수율을 지속적으로 해칠 때

### Evidence
- `core/src/policy.rs`, `core/src/security.rs`, `core/src/outbound_policy.rs`, `core/src/release_gate.rs`

---

## ADR-14. 파이프라인은 Python+Rust 병행(과도기)
### Context
- 실험 속도와 운영 안정성을 동시에 가져가야 함.

### Options
- A) Python+Rust 병행  
- B) Rust 단일화 즉시 전환

### Decision
- A) 과도기 병행

### Why Not Others
- 즉시 단일화는 실험 속도/기존 자산 활용을 포기해야 함.

### Trade-offs
- 운영 복잡도/언어 이원화

### Mitigation
- 단계별 Rust 전환 경로 유지, 실행 스크립트 표준화

### Revisit Trigger
- 이원화 유지비용이 기능 개발 속도보다 커질 때

### Evidence
- `src/collector/*`, `core/src/bin/collector_rs.rs`, `scripts/run_pipeline_rs.sh`

---

## ADR-15. 테스트 전략: 계층 혼합 검증 (Rust + Python + 시나리오 스크립트)
### Context
- 이종 스택과 로컬 제어 특성상 단일 테스트로 충분하지 않음.

### Options
- A) 계층 혼합 검증  
- B) 단일 프레임워크 통일

### Decision
- A) 계층 혼합 검증

### Why Not Others
- 단일 프레임워크 통일은 이상적이지만 현재 비용 대비 효과가 낮음.

### Trade-offs
- 테스트 파이프라인 관리 복잡도

### Mitigation
- 최소 검증 기준(compile/test/lint/build/pytest) 명시

### Revisit Trigger
- 테스트 유지비용이 과도하거나 flaky 비율이 통제 불가일 때

### Evidence
- `PROJECT_BIBLE.md`, `tests/*`, `run_gui_regression_pack.sh`

---

## 5. 아키텍처 선택을 더 깊게 보는 관점

## 5.1 시스템 형태
- 선택: 모듈러 모놀리스(core) + 데스크톱 셸 + 분리된 UI
- 이유:
  - 초기 단계에서 분산 마이크로서비스보다 운영 단순성과 디버깅 속도가 중요.
- 배제:
  - 초기 마이크로서비스: 운영 오버헤드가 제품학습 속도를 압도.

## 5.2 데이터 흐름 철학
- 선택: "입력 -> 계획 -> 실행 -> 검증 -> 증거 저장" 폐루프
- 이유:
  - 에이전트의 본질 리스크(잘못된 실행)를 줄이는 최소 구조.
- 배제:
  - 실행 성공 로그만으로 완료 판정하는 구조.

## 5.3 동시성 전략
- 선택: 안전 직렬화 우선
- 이유:
  - 로컬 UI/OS 제어는 충돌 비용이 큼.
- 배제:
  - 무조건 병렬 처리.

## 5.4 운영 전략
- 선택: 스크립트+런북+체크리스트 기반 운영
- 이유:
  - 팀 확장 전에도 재현 가능한 운영 프로세스 확보.
- 배제:
  - 개인 지식 의존형 수동 운영.

---

## 6. 향후 아키텍처 진화 시나리오

1. Throughput 확장 단계  
- lock scope 세분화, 안전 병렬 구간 분리

2. 멀티테넌트 단계  
- SQLite -> 서버형 DB 분리, 인증/권한 경계 강화

3. 고신뢰 오케스트레이션 단계  
- n8n 중심에서 Temporal-class 재평가

4. 멀티OS 단계  
- macOS 특화 모듈을 추상화 계층 뒤로 이동

---

## 7. 발표용 한 줄 결론
- 우리는 기술을 "유행"으로 고른 게 아니라,  
  **실행 안전성 + 검증 가능한 완료 + 운영 복구성**이라는 제약 아래  
  가장 낮은 리스크로 제품학습 속도를 유지할 수 있는 조합을 선택했다.

