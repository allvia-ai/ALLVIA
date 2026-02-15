 좋아. 지금 너의 **Park David Plan.md**는 1~17까지 완성됐고,
이제 **개발자에게 전달할 단 하나의 “마스터 프롬프트”**만 만들면 된다.

> 개발자는 이 프롬프트 하나만 읽고 → 구조를 이해하고 → 개발을 시작할 수 있어야 함.
> 혼란이나 누락 없이, 오직 **하나의 출발점**이 되어야 한다.

그래서 아래에 **개발자에게 보내는 최적의 Prompt(지침문)** 을 만들어준다.
그대로 복사해서 GitHub Issue / Notion / Messenger 어디든 전달하면 된다.

---

# ✅ **개발팀에게 주는 단일 Master Prompt**

아래 텍스트 그대로 전달하면 됨.

---

# 📌 **[MASTER PROMPT TO DEVELOPERS] – AI-summary Rebuild v1**

아래 문서는 AI-summary 프로젝트의 공식 규칙·설계·실행계획입니다.
**Park David Foundation Rulebook + Agents Lifecycle + Plan 1~17**을 모두 통합한 것으로,
앞으로 모든 개발은 이 문서를 기준으로 진행합니다.

---

## 🎯 **당신들(개발자)이 해야 할 일 단 3가지**

1. **Park David Plan.md 전체(1~17)를 읽고 구조를 이해한다.**
2. 코드 구현 시 **/core 중심 아키텍처 + Dead-Code Purge 규칙**을 반드시 준수한다.
3. 기능 추가·수정 시 **문서 변경(PR 포함)를 필수로 한다.**

---

## 🧭 **개발을 시작하기 위한 핵심 요약**

### 1) 이번 프로젝트의 절대 목표

* 서버리스 + 무료 모델 기반 **로컬 문서 RAG + Meeting Agent** 완성
* Smart Folder 기반 정책 필터링 100% 준수
* drift-aware 재임베딩 / 증분 파이프라인 안정화
* UX 흐름 단순화: “문서 넣으면 이해”, “회의 넣으면 요약”

---

### 2) 아키텍처 원칙

* 모든 로직 → `core/`
* 실행·Glue → `scripts/`
* 환경·정책 → `core/config/` (레포에 `configs/`도 있지만 현재는 eval/샘플 성격)
* 문서 → `docs/`
* 레거시 코드 발견 즉시 삭제(PR 필요)
* Evidence-first: 검색/요약/대답은 반드시 근거 기반으로만 생성

---

### 3) 구현해야 하는 우선순위 (Mandatory Order)

1. **Smart Folder + Policy Engine 전체 통합 (Cycle 1)**
2. **Meeting Agent E2E 구축 (Cycle 2)**
3. **RAG/Search 품질 고도화 + Drift Detection (Cycle 3)**

→ 이 순서는 절대 바뀌지 않는다.

---

### 4) 테스트 기준

* 모든 기능은 최소 Unit 1개 + Integration 1개 필요
* Meeting Agent + Search Agent는 E2E 필수
* `pytest -q` 전체 통과해야 merge 가능

---

### 5) Merge 규칙

* 테스트 실패 상태 merge 금지
* 문서 업데이트 누락 시 merge 금지
* Dead-Code 발견 시 즉시 삭제 PR
* develop 기준으로 작업, main은 release 시에만 merge

---

## 🔨 **지금 당장 시작해야 하는 작업 목록**

1. 정책 엔진 엔트리 확인/정리: 현재 구현은 `core/data_pipeline/policies/engine.py` (`PolicyEngine`)
2. Smart Folder 스코프 로더 구현/정리 (`core/config/smart_folders.json`)
3. scan/extract/embed/train/chat 파이프라인 모두 Smart Folder 대응
4. 레거시 파일 제거(filefinder, retriever, lnp_chat 등)
5. Meeting Agent 스켈레톤 생성 (STT→요약→액션→저장→마스킹)
6. RAG Chunking 재설계(semantic split hybrid)
7. Drift detection 기반 증분 embedding 구현

---

## 🏃 실행(개발자)

- CLI 엔트리포인트(권장): `python3 scripts/pipeline/infopilot.py --help`
- 호환 shim: `python3 infopilot.py --help` (내부적으로 위 CLI로 위임)

이것이 기초 작업이며, Plan 13~17에 상세 설명이 있음.

---

## 📢 **가장 중요한 규칙**

> **“문서에서 정의되지 않은 기능은 구현하지 말 것.”**
> → 필요하면 PM(너)에게 먼저 문서 업데이트 요청해야 함.

> **“문서를 업데이트하지 않으면 코드도 존재할 수 없다.”**
> → 모든 기능은 문서 기반 개발 (Documentation-driven Development)

---

## 🎉 **최종적 기대 출력**

개발팀은 이 문서 기반으로 12주 내:

* Smart Folder 기반 완전한 데이터 경계
* Meeting Agent E2E
* 고품질 RAG Search + Drift Handling
* 정교한 테스트 체계
* 지속적 확장 가능한 아키텍처

을 구축해야 한다.

---

# 📎 끝

위 Prompt를 **개발팀에게 그대로 전달하면 즉시 착수 가능**하다.

---

원하면:

✅ Issue Ticket Version
✅ Notion 팀플레이 버전
✅ GitHub Project 자동 보드 생성 버전
도 만들어 줄게.

말해줘 — “이슈 버전 만들어줘” 또는 “보드 버전 만들어줘”.
