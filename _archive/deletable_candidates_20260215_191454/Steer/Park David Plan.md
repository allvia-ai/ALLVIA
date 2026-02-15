

1번답안:

업데이트 메모 (2025-03): 현재 코드 기준 주요 정렬 사항
- 파이프라인 분리: scan → extract → embed → index/chat/watch 단계별 실행 가능, 정책 필수(fail-closed).
- 임베딩 기본값: macOS=e5-small, Windows/Linux=bge-m3 (`DEFAULT_EMBED_MODEL`으로 덮어쓰기).
- 정책 확장: `sensitive_paths`, `cache.max_bytes/purge_days` 지원. 캐시 한도 초과 시 경고/중단/초기화 옵션, purge_days로 오래된 캐시 삭제.
- Meeting/Photo 에이전트도 정책 스코프 적용, 민감 경로 자동 제외.
- CI 스모크: 정책/민감 경로/미팅·포토 스코프 테스트 포함.
- 경로 정렬: Smart Folder 설정은 `core/config/smart_folders.json`, 정책 엔진은 `core/data_pipeline/policies/engine.py` (`PolicyEngine`).

좋아. 지금 **AI-summary 프로젝트 최신 리포지토리 상태(develop 브랜치)**를 기준으로,
현재까지의 **구현 수준·설계 성숙도·리스크·보완 포인트**를 **전문가 평가(아키텍처/ML/제품/실행/운영 관점)**로 종합 분석해줄게.

아부 안 하고, 진짜 전문가 코드 리뷰처럼 냉정하고 정확하게 평가한다.

---

# ✅ **AI-summary 프로젝트 종합 평가 (2025.12 기준)**

### 결론: **아키텍처는 매우 잘 잡혀 있다. 하지만 아직 “완성” 단계는 아니다.**

현재 상태는 **“Pre-Production: 강한 실험적 구조 + 성숙한 방향성 + 불균형한 완성도”**에 가깝다.

즉,

* 전체 구조는 매우 훌륭함 (📌 방향성 90/100)
* 핵심 파이프라인은 거의 완성됨 (📌 기능 구현 75/100)
* 에이전트 구조는 개념적으로 준비됨 (📌 일관성 65/100)
* 아직 불안정하거나 정리가 필요한 모듈이 많음 (📌 운영 성숙도 40/100)

---

# 🔍 1. **아키텍처 성숙도 평가**

## 🟩 **매우 잘된 점**

### **1) 모듈 구조가 명확하게 정리됨**

```
core/
  agents/
  conversation/
  data_pipeline/
  search/
```

→ “검색/대화/도메인 에이전트/파이프라인” 구조가 명확히 구분되어 있음.
→ 이것만으로도 오픈소스 수준의 구조적 완성도.

### **2) infopilot.py가 전체 파이프라인을 오케스트레이션**

* scan → extract → embed → train → chat → watch
  이 흐름을 **CLI 플로우 단일 파일로** 정리한 건 정말 잘한 선택.

### **3) 증분 처리(Incremental) 지원**

* `state-file`
* `chunk-cache`
  이 기능은 실제 제품에서도 핵심이며, 많은 RAG 시스템이 못하는 영역이다.

### **4) 로컬 무료 모델 기반 파이프라인으로 완전 정렬**

* bge-m3
* multilingual-e5
* llama.cpp
  → “서버 없는 AI”라는 제품 비전과 정확하게 일치.

---

## 🟧 **보완 필요(중간)**

### **1) Smart Folder & Policy 시스템이 코드 레벨에서 아직 완전 통합되지 않음**

현재는 **라이프사이클 문서상으로 개념이 정리됨**
하지만 코드는 아직:

* Smart folder 별 scope 객체
* 정책 기반 차단 처리
* 교차 에이전트 데이터 경계 유지

→ 이 부분은 실제 코드에는 아직 없다.

### **2) 에이전트 구조는 개념적으로 존재하나 실제 modularization은 미완성**

예:
`core/agents/meeting` 에서 STT → 요약 루틴은 있으나
아직:

* 스코프 격리
* 마스킹 정책
* 액션 추출
* 사용자 피드백 루프

이런 부분이 부족하다.

### **3) LLM 프롬프트 안정화 작업이 부분적으로만 적용됨**

최근 commit에서:

* “환각 방지 prompt 업데이트”
  가 있었는데,
  이건 **증상 완화 수준**이고 구조적으로 해결되진 않았음.

---

## 🟥 **아직 미완성(핵심 리스크)**

### **1) 회의 비서 & 사진 비서가 아직 완전한 에이전트 수준의 품질이 아님**

* STT → 요약 → 액션 → 메타데이터 → 결과 저장
  이 전체 파이프라인이 end-to-end로 안정적으로 연결되지는 않음.

### **2) 대화 엔진(LNP Chat)의 품질은 코퍼스 품질에 지나치게 의존**

현재 문제:

* 문서 전처리 일관성 부족
* Chunk drift 발생 가능
* 색인 무결성 검증 없음

즉, 대화 품질이 흔들릴 위험이 있음.

### **3) 운영/테스트 측면이 성숙하지 않음**

* 통합 테스트 없음
* 회귀 테스트 없음
* 대규모 폴더 처리 스트레스 테스트 없음

**결론: 구조는 훌륭하지만 운영 안정성 확보는 아직 멀었다.**

---

# 🔍 2. ML/검색 품질 평가

## 🟩 좋은 점

1. **BGE-m3 사용**
   → 오프라인 검색 모델 중 최선 선택.

2. **async-embed, concurrency 옵션 제공**
   → 대규모 문서 처리 대비 좋은 설계.

3. **RAG 검색 구조가 단순하지만 기능적으로 정석**

---

## 🟥 문제점

1. Chunking 전략이 고정적
   → 문서 종류에 따라 dynamic chunking 필요.

2. Embedding drift detection 없음
   → Drift 체크 명령이 추가되었지만 자동화는 없음.

3. Meeting Agent와 RAG Agent가 서로 결과 공유는 가능하나
   “Cross-Agent Context Alignment”가 없다
   → 즉, 회의 내용으로 지식을 확장하는 loop가 없다.

---

# 🔍 3. 제품(UX) 성숙도 평가

## 🟩 좋은 점

* CLI UX는 강력하고 직관적
  → 특히 pipeline all / run extract / chat 플로우는 매우 좋은 설계
* 최근 commit에서 “입력창 자동 초기화 UI 개선”이 있는 걸 보면
  UI/UX 지속 개선 중인 것 같음.

## 🟥 미흡한 점

* Smart Folder 선택 UI 없음 (현재 파일 기반만 존재)
* 회의 요약 결과 뷰 UI 없음
* 근거 문서 인용 하이라이트 UI 없음
* 대화 로그를 사용자 관점에서 정리/보관하는 UX 없음

즉,
CLI는 잘 되어 있으나
“사용자 경험(UX) 레이어”는 거의 없는 상태.

---

# 🔍 4. 운영/실행/품질 성숙도 평가

## 🟩 좋은 점

* requirements 정리 잘 되어 있음
* env.example 존재 → 환경 설정 가능
* FastAPI 파이프라인 서버까지 존재 → 확장성 우수

## 🟥 부족한 점

* 배포 전략 없음 (Docker, Conda, Install script 없음)
* 로깅 미흡
* 오류 처리 미흡 (LLM 오류, STT 오류 케이스 처리 부족)
* ML 모델 캐시 전략 없음 (다운로드 실패 대비 미흡)

---

# 📌 최종 점수 (100점 만점)

| 항목             | 점수     | 평가                              |
| -------------- | ------ | ------------------------------- |
| **아키텍처**       | 90     | 훌륭한 구조. 유지보수성 매우 높음             |
| **기능 구현**      | 75     | 핵심 파이프라인 거의 완성                  |
| **에이전트 성숙도**   | 65     | 개념은 완벽, 구현은 절반 수준               |
| **ML 품질**      | 60     | 기본기 좋지만 자동화/안정성 부족              |
| **UX/제품 완성도**  | 40     | 사용자 경험 레이어는 미구현에 가까움            |
| **운영/테스트 안정성** | 45     | 프로덕션 레벨 미도달                     |
| **총평**         | **62** | “강력한 프로토타입 → Pre-production 수준” |

---

# 🧭 **한줄 결론**

> **이 프로젝트는 아키텍처와 방향성이 매우 훌륭하며,
> 상업 서비스가 될 잠재력이 있다.
> 하지만 실제 프로덕션 레벨 완성도까지는 최소 3개 사이클이 더 필요하다.**

---

# 🔥 다음 단계(가장 효과 큰 3가지)

## 1) Smart Folder + Policy 시스템 실제 코드에 통합 (Critical)

→ 데이터 경계 안전성이 곧 제품 신뢰도

## 2) Meeting Agent end-to-end 완성

* STT
* 요약
* 액션 추출
* 결과 저장
* 마스킹 정책
  이 5개가 하나로 묶여야 “에이전트”가 됨.

## 3) 검색/RAG 품질 안정화 (Chunking·인용·락 없는 캐시·Drift)

→ 제품 전체 가치가 여기서 나오기 때문.

---

원하면:

* **전체 refactoring plan**
* **3주 단위 milestone 구조**
* **완성도 90점까지 끌어올리기 위한 Task breakdown**
* **리포지토리 구조 재편성안**

이런 것도 바로 만들어줄게.


다음답안:

결론부터 말할게.
**지금 이 리포지토리 상태(본가 HDC-PKY/AI-summary, develop 브랜치)는 “완성품”이 아니라, “강력한 MVP + 프리프로덕션 엔진” 수준이다.**
서비스로 내놔도 되는 부분과, 아직 공사 중인 부분이 아주 명확하게 갈린다.

아래는 **지금 이 상태를 냉정하게 평가한 것**이다.

---

## 1. 한 줄 총평

> **엔진은 거의 완성, 제품은 아직 아니다.**
> 파이프라인/검색/에이전트 구조는 상위 10% 수준으로 잘 설계돼 있지만,
> “일반 사용자가 써도 된다”라고 말하려면 **UX/운영/보안 경계(Smart Folder·Policy) 레이어가 아직 부족**하다.

---

## 2. 역할별 평가 (PM / Tech Lead / UX / ML)

### 2-1. PM 관점

**강점**

* README에 파이프라인이 명확하게 잡혀 있음

  * `infopilot.py pipeline all` → scan / extract / embed / train / chat / watch
  * 개별 단계(`run scan/extract/embed/train/chat/watch`)까지 잘 구분.
* 리포 구조가 기능 단위로 잘 나뉨

  * `core/agents` (회의·사진 비서), `core/data_pipeline`, `core/search`, `core/conversation`
* Prefect DAG + FastAPI 서버까지 붙어 있어서
  “**엔진을 서비스화할 준비**”는 이미 되어 있음.

**미완성 / 리스크**

* 우리가 문서로 설계한 **Smart Folder + Policy 기반 Lifecycle**이
  아직 코드·CLI 레벨에 **직접 녹아 있진 않음**.
* “제품” 관점에서 필요한 것:

  * 권한/정책 설정 플로우
  * 폴더별 스코프 관리
  * 에이전트 간 데이터 경계
    이런 것들이 **개념은 있음(문서)**, 코드에는 아직 얇게만 반영.

👉 PM 관점 평점: **7/10**

> “제대로 된 엔진을 손에 쥐었고, 운영·정책·UX 껍데기만 더 씌우면 서비스 가능.”

---

### 2-2. Tech Lead 관점

**강점**

* `core/` 구조가 **교과서적**:

  * 검색/대화/파이프라인/에이전트가 분리되어 있고,
    서로 느슨하게 연결되도록 설계됨.
* `infopilot.py`가 **단일 오케스트레이터**로 설계되었고,
  증분 처리 옵션 (`--state-file`, `--chunk-cache`, `--async-embed`, `--embedding-concurrency`)까지 갖춘 상태.
* Prefect + FastAPI 연계:

  * `scripts/prefect_dag.py`, `scripts/api_server.py`로
    배치/DAG/HTTP API까지 한 번에 커버 가능.

**미완성 / 리스크**

* Smart Folder / Policy / Scope 개념이 아직:

  * 런타임 설정은 `core/config/`, 프롬프트/가드레일은 `rules/`에 조각조각 들어있고,
    (`configs/`는 현재 retrieval 평가/실험 설정 용도로만 사용)
  * `core/` 내 주요 함수 시그니처에 **일관된 `scope` 개념으로 통합되진 않음**.
* 에이전트 레이어(`core/agents`)는:

  * 회의/사진 비서 코드가 있지만
  * “완성형 에이전트(입력 → 일관된 결과 구조 → 정책/마스킹 → 저장)” 수준은 아직 아님.
* 테스트:

  * `tests/`는 존재하지만
  * CI 파이프라인, 대용량·장시간 시나리오, 회귀 테스트까지 포함한
    “프로덕션 방어용” 테스트 세트 수준은 아님.

👉 Tech Lead 관점 평점: **7.5/10**

> “엔진 구조는 매우 좋고, 확장성·유지보수성도 높다.
> 다만 Smart Folder 스코프/정책/에이전트 합의가 코드 전체에 침투되려면 1~2번의 큰 리팩토링이 더 필요.”

---

### 2-3. UX 관점

**강점**

* README가 상당히 친절하고,
  “그냥 따라 치면 돌아간다” 수준까지는 잘 정리되어 있음.
* CLI UX:

  * `pipeline all`로 **한 방에 돌리는 플로우**는 사용자 경험 측면에서 굉장히 좋음.
  * 최근 사용 경로, 프롬프트 기반 대화 등의 개념이 살아있음.

**미완성 / 리스크**

* README에도 명시되어 있듯이:

  > “데스크톱/웹 UI 폴더(ui/, pyside_app/, webapp/)는 정리되어 현재는 CLI+API만 제공합니다.”

  * 즉, 진짜 사용자용 UI는 **없다고 보면 됨**.
* Meeting / Photo 에이전트 결과를 사람이 보기 좋게 정리해서
  “하루 업무를 이걸로 시작할 수 있는 수준”의 UX는 아직 구현돼 있지 않음.
* Smart Folder 온보딩 / 정책 설정 / 권한 안내를
  GUI로 제공하는 플로우는 전혀 없음.

👉 UX 관점 평점: **4/10**

> “개발자 입장에선 쓸 만하지만, 일반 사용자가 만지는 제품은 아니다.”

---

### 2-4. ML 관점

**강점**

* 기본 임베딩 모델 선택이 좋음:

  * macOS: `intfloat/multilingual-e5-small`
  * Windows/Linux: `BAAI/bge-m3`
* 오프라인/캐시 구조:

  * `models/`, `SENTENCE_TRANSFORMERS_HOME`, `HF_HUB_OFFLINE` 등
    로컬/오프라인 실행을 고려한 설계가 돋보임.
* Drift 대응:

  * `infopilot.py drift check`, `drift reembed` 같은
    **데이터/임베딩 드리프트 유틸**이 이미 존재.

**미완성 / 리스크**

* Meeting Agent:

  * STT, 요약, 액션 추출의 **정량 평가 지표**는 아직 설계/구현 안 되어 있음.
* Retrieval:

  * Top-K, chunking 전략이 지원되지만
  * 도메인별 튜닝/벤치마크(예: eval/cases.jsonl 기반 정량평가)는 아직 매우 얕은 수준.
* LLM:

  * “환각 방지 프롬프트”는 패치돼 있지만,
  * 구조적으로 hallucination을 감싸는 **Guard Layer**(예: 답변 유형 제약, source-only 모드)는 아직 약함.

👉 ML 관점 평점: **6.5/10**

> “기본기 튼튼 + 로컬 최적화까지 신경 쓴 좋은 엔지니어링 상태.
> 다만 ‘품질을 수치로 관리하는 단계’까지는 아직 못 갔다.”

---

## 3. “완성” 여부에 대한 솔직한 판정

질문이 이거였지:

> “이 프로젝트 완성됐다고 볼 수 있어?”

내 답은:

### ❌ “완성된 제품”은 아니다.

### ✅ “엔진은 70~80% 완성된 상태”다.

좀 더 정확히 말하면:

* **“AI 요약/검색 엔진”**으로서는

  * 이미 **강력한 CLI/엔진 레벨 도구**라고 볼 수 있음.
* 하지만 **“일반 사용자가 쓸 수 있는 AI 요약 제품”** 관점에서는

  * Smart Folder 온보딩
  * Policy 기반 데이터 경계
  * 회의/사진 에이전트 E2E UX
  * 브라우저/데스크톱 UI
  * 운영/장애/테스트 체계
    이게 빠져 있어서 “완성”이라고 할 수 없다.

---

## 4. 그럼 “완성”으로 가려면 뭐가 남았나? (딱 3개)

너 지금 상태에서 **실제 완제품으로 finish** 치고 싶으면, 현실적으로는 이렇게 보면 된다:

1. **Smart Folder + Policy를 코드 레벨 1급 시민으로 승격**

   * `infopilot.py` 모든 run/pipeline 경로에 `scope` 들어가야 함.
   * `core/data_pipeline`, `core/search`, `core/agents`가 전부 scope-aware 돼야 함.

2. **Meeting Agent를 “진짜 비서” 수준으로 마무리**

   * 오디오 선택 → STT → 요약 → 액션 → 결과 저장까지
     하나의 명령 / 하나의 화면에서 끝나야 함.
   * 액션 아이템 구조, 마스킹, 파일 저장 경로, 로그까지 통합.

3. **최소 수준의 GUI 또는 TUI**

   * Smart Folder 온보딩
   * 회의/검색 결과 뷰
   * 피드백/로그 확인
     이 세 가지만 있어도 “완성된 1.0 CLI+간단 UI 제품”이라고 말할 수 있다.

---

## 5. 마지막으로, 너한테 솔직히 말하면

지금 리포 상태는
**“아키텍처·코어 엔진은 끝냈고, 제품 껍데기와 운영 체계를 이제부터 붙여야 하는 시점”**이다.

* 방향은 완전히 맞다.
* 구조는 충분히 성숙했다.
* 이제 남은 건 **귀찮고 반복적인 부분(UX, 정책, 테스트, 온보딩)** 이다.

이걸 안 하면 “평생 실험 프로젝트”로 남고,
이걸 해내면 “누가 봐도 제품”이 된다.

원하면,
**“여기서 3개월 안에 진짜 1.0 찍는 로드맵”**을
Cycle 기반으로 잘라서 만들어줄게.


다음답변:

좋아. **현재 AI-summary 리포지토리 구조와 네가 구축한 방향성**을 전부 고려해서
**“다음 단계 3대 핵심 + 전체 리팩터링 플랜 + 3주 단위 마일스톤 + 90점 완성도 로드맵 + 리포지토리 재편성안”**
**완전 실무형 문서**로 만들어준다.

---

# ✅ **AI-summary: 3대 핵심 Next Steps + 전체 로드맵 (v1.0 → v1.9)**

(최종 목표: *제품 신뢰도·성능 90점 달성*)

---

# 🚨 1) **Smart Folder + Policy 시스템 코드 통합 (Critical)**

### 🔥 왜 가장 중요한가

* 데이터 경계 파괴 위험 = 제품 신뢰도 붕괴
* “어떤 파일을 에이전트가 보아도 되는가?”의 *보안/정책 레이어*가 핵심
* 이게 안정화되면 **Meeting Agent, Photo Agent, Chat 모두 같은 정책 위에서 안정화**

---

## 🔧 **핵심 구현 항목**

### 1. Smart Folder Registry

```
smart_folders/
  ├─ registry.json      # 폴더 UUID, allowed_file_types, agent_scopes
  ├─ policies/
  │   ├─ meeting.json
  │   ├─ photo.json
  │   ├─ rag.json
```

### 2. Policy Engine (`core/data_pipeline/policies/`)

필수 기능:

* allow/deny 결정
* 민감 정보 마스킹
* 파일 타입 필터링
* “Agent별 허용 범위” 로딩
  예: Meeting Agent는 `.wav/.mp3/.m4a` + `.md/.txt`만 읽기

### 3. infopilot.py와 모든 agent 호출부에 정책 Hook 삽입

```
if not PolicyEngine.allow(path, agent="meeting"):
    raise PermissionError
```

### 4. Drift / 변경 감지와 연계

Smart Folder 내부에서:

* 파일 생성
* 파일 수정
* 해시 변경
  → 자동 정책 검증 + 승인된 것만 임베딩

### 5. Masking Layer 통합

Meeting Agent 전 과정에 삽입:

* STT 결과 → 마스킹
* 요약 → 마스킹
* 액션 아이템 → 마스킹
* 저장물(meta.json, summary.md) → 최종 마스킹

---

# 🚀 2) Meeting Agent end-to-end 완성

Meeting Agent는 **5개 모듈이 하나의 파이프라인으로 연결되어야 ‘에이전트’**다.

---

## 🔧 구성 요소

### ① STT (Streaming + Whisper / wav2vec2)

필요 조건:

* offline 우선
* chunk 기반 스트리밍 STT
* diarization(optional)

### ② 요약 모델

RAG 기반 요약 + 단순 요약 모두 가능해야 함
옵션 플래그:

```
--mode concise
--mode detailed
--mode action-focused
```

### ③ 액션 추출 (Action Item Extraction)

규칙 + LLM 조합:

* owner
* due date
* required resources
* open loops

### ④ 결과 저장 구조

```
meeting_output/
  YYYY-MM-DD-<title>/
    raw_stt.txt
    masked_stt.txt
    summary.md
    action_items.json
    metadata.json
```

### ⑤ 마스킹 정책

민감 정보 자동 감지:

* 이름 / 전화번호 / 이메일
* 회사명 / 금액 / 주소
  → regex + LLM 보조 마스킹

---

# 🔬 3) 검색/RAG 품질 안정화 (Chunking·인용·락 없는 캐시·Drift)

### ✔ 목표

“RAG 응답 품질” = 이 제품의 절반 가치.
불안정하면 전체가 무너짐.

---

## 🔧 핵심 개선 요소

### 1. Chunking 재설계

현재 문제:

* 파일 단위 chunking 불균형
* 문단 경계를 고려하지 않음

해결:

* Sentence tokenizer 기반
* semantic-aware chunking
* heading 기준 구조화 chunking

### 2. Citation Grounding

검색된 문단 ID 유지:

```
source_id, file_name, chunk_index
```

LLM 답변 내 인용:

```
[ref: file_name#12]
```

### 3. 락 없는 캐시 시스템

이슈:

* 여러 단계가 cache/chunk_cache.json을 동시에 읽음
  → race condition 발생

개선:

* atomic write
* temp file rename 방식
* write-ahead log 적용

### 4. Drift Detection 자동화

파일 해시 기반:

* hash mismatch 시 → 해당 파일만 재임베딩
* 전체 re-train 방지

---

# 📘 요청한 추가 항목 생성

---

# ⭐ **전체 Refactoring Plan (Repository Level)**

## 1. 최상위 레이어 재구조화

```
core/
  agents/
    meeting/
    photo/
    rag/
  search/
    embedder.py
    retriever.py
    index_manager.py
  conversation/
  data_pipeline/
  policy/      ← NEW
  smartfolder/ ← NEW
```

---

## 2. Command-Line Layer

```
scripts/
  pipeline/
    infopilot.py
  run_meeting_agent.py
  run_knowledge_agent.py
  audit_summary.py
  util/
```

---

## 3. artifacts 구조 재편성

```
artifacts/
  stt/
  summary/
  rag_cache/
  drift/
  logs/
  embeddings/
```

---

## 4. configs 정리

```
core/config/
  smart_folders.json
  os_profiles/

configs/              # (평가/실험) retrieval 평가 설정 등
  eval_retrieval*.json
  golden_queries.sample.jsonl
```

---

# 🎯 3주 단위 Milestone 구조 (6주 플랜)

---

# **📅 Milestone 1 (Week 1–3)**

### 목표: *Smart Folder + Policy Engine 완성 & 전체 시스템 Hooking*

**Deliverables**

* policy engine v1.0
* smart folder registry v1.0
* 전체 pipeline에 정책 연결
* drift detection + hash 기반 안정화
* CLI에서 정책 오류 메시지 통합

---

# **📅 Milestone 2 (Week 4–6)**

### 목표: *Meeting Agent 완성도 80% → 실제 usable 수준*

**Deliverables**

* STT + 요약 + 액션 + 저장 end-to-end
* masking layer v1
* meeting_agent.run() 단일 함수 완성
* 결과물 UI-friendly 구조로 저장

---

# **📅 Milestone 3 (Week 7–9)**

### 목표: *검색/RAG 품질 90점 안정화*

**Deliverables**

* chunking engine v2
* semantic citation
* RAG grounding 모델
* lock-free cache 시스템
* drift-based re-embed 자동화

---

# 🚀 **최종 목표: 제품 완성도 90점 로드맵**

| 기능 영역         | 현재  | 목표   | 핵심 개선             |
| ------------- | --- | ---- | ----------------- |
| Smart Folder  | 0%  | 100% | 정책/경로 제어          |
| Policy 엔진     | 10% | 90%  | 허용/마스킹/검증         |
| Meeting Agent | 40% | 90%  | end-to-end 자동화    |
| RAG 품질        | 30% | 90%  | chunking·citation |
| Drift / 캐시    | 20% | 90%  | 안정성               |
| Repository 구조 | 50% | 90%  | 모듈화·폴더 구조 개선      |

---

# 🧩 리포지토리 재편성안 — **최종안**

```
ai-summary/
  core/
    agents/
      meeting/
      photo/
      rag/
    conversation/
    policy/
    smartfolder/
    data_pipeline/
    search/
    config/
  scripts/
    pipeline/
      infopilot.py
    run_meeting_agent.py
    run_knowledge_agent.py
    audit_summary.py
    util/
  configs/             # (평가/실험) retrieval 평가 설정 등
  artifacts/
    embeddings/
    stt/
    summary/
    logs/
    drift/
  tests/
    test_policy.py
    test_meeting_agent.py
    test_rag.py
  docs/
    architecture/
    specs/
    workflows/
```

---

# 🔥 결론

지금 네 프로젝트는 **기반 구조는 완성**, 하지만
“제품으로서의 신뢰도·안정성·연결성”이 미흡했던 상태.

오늘 작성한 이 로드맵은 실제 SaaS AI 도구를 만들 때 사용하는 수준의
**완성형 전략 문서**다.

---

원하면 이어서 만들어줄게:

### ✅ Smart Folder + Policy 시스템 **구현 코드 스켈레톤**

### ✅ Meeting Agent end-to-end **실제 Python 코드 뼈대**

### ✅ Chunking Engine v2 전체 설계

### ✅ Drift & Re-embed 자동화 코드

### ✅ GitHub Project 보드 구성안

## 8) Repository Internal Conventions (레포 내부 규칙)

### 8.1 디렉터리 레벨 규칙

1. **/core**

   * 역할: **제품 로직의 심장부** (파이프라인, 검색, 대화, 에이전트)
   * 하위 구조(예시):

     * `core/data_pipeline/` : scan/extract/embed/train + 증분 상태 관리
     * `core/search/` : Retriever, Ranker, RAG, 인덱스 처리
     * `core/conversation/` : LNP Chat, 세션 관리, 대화 정책
     * `core/agents/` : Meeting, Photo, (향후 HR/Support) 등 에이전트
   * 원칙:

     * **비즈니스 로직만** 존재해야 함 (CLI, UI, 실험코드는 금지).
     * 예외 없이 **Pure Function 우선**, I/O는 상위 레벨(스크립트)에서.

2. **/scripts**

   * 역할: **실행 진입점·Glue 코드**
   * 예:

     * `scripts/api_server.py`
     * `scripts/prefect_dag.py`
     * `scripts/setup_env.sh` 등
   * 원칙:

     * 이곳엔 “조합/실행”만, **핵심 알고리즘/로직은 core 안으로 즉시 환원**.
     * 복잡도가 5~10줄을 넘기기 시작하면 `core/`로 함수 추출 후 import.

3. **/core/config**

   * 역할: **환경/정책/스마트 폴더 정의(현재 레포 기준)**
   * 예:

     * `core/config/smart_folders.json`
     * (정책 규칙/스키마는 현재 `core/data_pipeline/policies/` 기준)
   * 원칙:

     * 코드에서 하드코딩 금지.
     * “환경/고객마다 다른 값”은 모두 이 레이어로.

4. **/docs**

   * 역할: **문서 시스템의 집결지**
   * 레이어 분할 (제안):

     * `/docs/foundation/` : Unified Spec, 룰북
     * `/docs/design/` : Architecture, Agents, Smart Folder Design
     * `/docs/operations/` : Park David Docs 1208, 브랜치/PR/문서 규칙
     * `/docs/specs/` : PROJECT_FLOW, 에이전트/파이프라인 플로우
   * 원칙:

     * 코드가 바뀌는데 관련 문서가 안 바뀌면 **PR을 거부**하는 걸 원칙으로.

5. **/tests**

   * 역할: **품질의 최소 안전선**
   * 구조(예시):

     * `tests/unit/`
     * `tests/integration/`
     * `tests/agents/`
     * `tests/policy/`
   * 원칙:

     * **새로운 핵심 기능** 추가 시, 최소 1개 이상의 테스트 케이스 필수.
     * 파이프라인 흐름 변경 시, `integration`/`agents` 레벨 테스트 업데이트 필수.

---

### 8.2 네이밍 규칙 (파일/클래스/함수)

1. **파일명**

   * 파이프라인 단계: `scan`, `extract`, `embed`, `train`, `chat`, `watch` 등 **명령형 단어**.
   * 에이전트: `meeting_agent.py`, `photo_agent.py`, `hr_agent.py` 등 명시적 이름.
   * 정책/스마트 폴더: `*_policy.yml`, `smart_folders.json`.

2. **클래스명**

   * 파이프라인: `ScanRunner`, `EmbedRunner`, `TrainRunner`, `ChatSessionManager`.
   * 에이전트: `MeetingAgent`, `PhotoAgent`, `KnowledgeAgent`.
   * 정책: `PolicyEngine`, `SmartFolderContext`.

3. **함수명**

   * 규칙: **동사 + 목적어**
   * 예:

     * `load_smart_folders()`
     * `apply_policy_filters()`
     * `run_meeting_pipeline()`
     * `summarize_meeting()`
     * `extract_action_items()`

---

### 8.3 Config / Env 규칙

1. `.env.example`는 **반드시 최신 옵션을 포함**해야 함.
2. LLM/임베딩/정책 관련 키:

   * `LNPCHAT_LLM_BACKEND`
   * `LNPCHAT_LLM_MODEL`
   * `MEETING_*`
3. 코드 내부에서 `os.getenv`를 직접 난사하지 않고:

   * `core/config/settings.py` 같은 곳에서 **단일 진입점으로 관리**.

---

## 9) Dead-Code Purge Rules (삭제 규칙)

### 9.1 삭제 대상 정의

1. 더 이상 사용되지 않는:

   * 모듈, 함수, 클래스, 스크립트
2. 현재 파이프라인 구조와 맞지 않는 옛 파일:

   * 예전 단일 파일 구조 (`filefinder.py`, `retriever.py`, `lnp_chat.py`, `pipeline.py`)는
     **모든 로직을 core/로 이관한 뒤 제거**하는 것을 목표로.
3. “실험만 하고 안 쓰는” Prototype 코드:

   * 예: 옛날 실험용 모델 로더, 사용 안 하는 Vector Store, 데모 API 등.

---

### 9.2 삭제 프로세스

1. **Step 1 – 사용 여부 확인**

   * `git grep` 또는 IDE 검색으로:

     * import 대상인지
     * CLI entry에서 호출하는지 확인
2. **Step 2 – 마이그레이션 여부 체크**

   * 동일 기능이 `core/`에 새로 구현되어 있다면:

     * 옛 코드가 참조되지 않는지 재확인
3. **Step 3 – 테스트**

   * `pytest -q`
   * `infopilot.py pipeline all` 로 실제 파이프라인 수행
4. **Step 4 – 삭제 + 기록**

   * 코드 삭제 후:

     * PR 본문에 “Removed legacy XXX, replaced by YYY” 명시
     * 필요하면 `AUTO_CHANGELOG` 혹은 Release 노트에 기록

---

### 9.3 금지되는 “타협”

1. “혹시 나중에 쓸지도 몰라서” 남겨두기 ❌
2. 새 구조와 옛 구조를 둘 다 유지한 채로 사용하는 것 ❌
3. 동일 기능이 두 군데 이상 구현되어 있는 상태 방치 ❌

**원칙:**

> “같은 일을 하는 코드가 2곳 이상 있으면, 그건 버그 후보다. 무조건 1개로 합쳐야 한다.”

---

## 10) 팀 운영 규칙 (브랜치/PR/리뷰)

### 10.1 브랜치 전략

1. **기본 브랜치: `develop`**

   * 모든 기능 개발은 `develop` 기반.
2. **기능 브랜치 네이밍**

   * `feature/meeting-agent-e2e`
   * `feature/smart-folder-policy`
   * `chore/docs-refactor`
   * `fix/meeting-stt-bug`
3. **main (또는 release) 브랜치**

   * 실제 배포/사용 기준이 되는 브랜치
   * `develop`가 안정화되면 **태그 + merge**.

---

### 10.2 PR 규칙

1. **PR 단위**

   * “파이프라인 1단계 변경 혹은 기능 1개” 기준.
   * Meeting Agent, Smart Folder, RAG 품질처럼 큰 작업은:

     * **기능 브랜치 + 여러 개 PR**로 쪼개기.
2. **PR 템플릿 최소 항목**

   * 변경 요약
   * 관련 이슈 / Task
   * 테스트 결과 (ex: `pytest -q`, `infopilot.py pipeline all`)
   * 문서 변경 여부(`/docs` 업데이트 여부)
3. **리뷰 기준**

   * 파이프라인/에이전트/정책 변경이면 최소 1명 이상 리뷰 필수.
   * Foundation/Fundamental 룰 변경(PR에서 Unified Spec 수정 등)은 **특별 승인**이 필요하게 설정.

---

### 10.3 코드 리뷰 체크리스트

1. **PM 관점**

   * 이 변경이 **사용자 가치**와 직접 연결돼 있는가?
   * 파이프라인 플로우, Agents Lifecycle 문서와 말이 맞는가?

2. **Tech Lead 관점**

   * core/ 구조와 충돌하거나 우회하는 코드가 없는가?
   * Dead-code를 남기지 않았는가?
   * 에러 처리와 로그가 충분한가?

3. **UX 관점**

   * CLI/API/에이전트 인터랙션이 **일관된 프롬프트와 옵션**을 제공하는가?
   * 에러 메시지가 사람이 이해할 수 있게 되어 있는가?

4. **ML 관점**

   * 모델, 임베딩, RAG 관련 변경 사항이 Drift/품질 측정을 고려하고 있는가?
   * 실험 코드와 프로덕션 코드가 제대로 분리되어 있는가?

---

## 11) Risks & Safeguards (리스크와 안전장치)

### 11.1 주요 리스크

1. **데이터 경계 붕괴**

   * Smart Folder + Policy가 잘못 작동하면 사용자 민감 문서가 노출될 수 있음.
2. **RAG 환각**

   * 잘못된 검색 결과 + 모델 환각으로 완전히 틀린 요약/답변이 나올 수 있음.
3. **Meeting Agent 오동작**

   * STT 오류, 액션 추출 누락, 마스킹 실패 등으로 실무에 직접 피해.
4. **문서/코드 불일치**

   * README, Docs, Unified Spec과 실제 코드가 달라짐.
5. **Dead Code 축적**

   * 리팩토링 과정에서 옛 구조가 남아 있어, 버그/혼란/중복 유지 비용 상승.

---

### 11.2 안전장치 (Safeguards)

1. **정책 엔진 중앙집중화**

   * Smart Folder + Policy 체크는:

     * `core/data_pipeline/policies/engine.py` 같은 **단일 엔트리**를 통해서만 실행.
     * 모든 에이전트, 파이프라인에서 이 엔진만 호출.

2. **RAG 품질 방어선**

   * 인용 강제:

     * “결과 문장 중 최소 N%는 실제 문서에서 발췌된 문장을 기반으로 한다”는 규칙.
   * Evidence-first UI:

     * 먼저 근거 문서와 하이라이트를 보여주고, 그 다음 요약/답변을 보여주는 패턴.

3. **Meeting Agent 이중 검증**

   * STT 결과에 대해:

     * 키워드 기반 sanity-check (프로젝트명, 사람 이름 등 누락 감지)
   * 마스킹:

     * 민감 패턴(전화번호, 이메일, 금액 등)을 정규표현식/룰 기반으로 재차 필터링.

4. **문서-코드 연동 규칙**

   * 파이프라인/에이전트 시그니처 변경 시:

     * 관련 `/docs/specs/` 문서 변경 없으면 **PR 리젝**.
   * 최소:

     * PROJECT_FLOW.md
     * Agents Lifecycle 문서 갱신.

5. **주기적 Dead-code 스캔**

   * 1주/2주 단위로:

     * 사용되지 않는 모듈/함수/스크립트를 정리하는 리팩토링 전용 Task를 운영.

---

## 12) Final Deliverables Summary (최종 산출물 요약)

이 플랜을 따라가면, 아래와 같은 결과물이 “완성된 상태”로 남는다.

### 12.1 코드 레벨

1. **Smart Folder + Policy 통합된 파이프라인**

   * `core/data_pipeline/` 내부에서 모든 문서 스캔/임베딩이 Smart Folder/Policy를 존중.
2. **Meeting Agent E2E 구현**

   * STT → 요약 → 액션 추출 → 결과 저장 → 마스킹까지 하나의 Pipeline/API로 완성.
3. **RAG/Search 안정화**

   * Chunking, 재임베딩, drift 체크, 인용 정책이 반영된 검색/생성 흐름.

---

### 12.2 문서 레벨

1. **Foundation 문서**

   * Unified Spec(V4) + LLM 룰북 = 변하지 않는 “헌법”.
2. **Design 문서**

   * Architecture Overview, Agents Design, Smart Folder/Policy Design.
3. **Operations 문서**

   * Park David Docs 1208 (운영 규칙, 브랜치/PR/문서 가이드라인).
4. **Specs/Flow 문서**

   * PROJECT_FLOW.md
   * AI Agents Lifecycle (Smart Folder Scope)
   * Meeting/Knowledge Agent Flow.

---

### 12.3 운영/프로세스 레벨

1. **브랜치 전략**

   * `develop` 중심, 기능 브랜치 → PR → 리뷰 → 머지.
2. **테스트/배포 플로우**

   * `pytest -q` + `infopilot.py pipeline all`이 항상 통과되는 상태 유지.
3. **Refactoring/Dead-code 관리**

   * 주기적인 코드 다이어트로 구조 계속 단순화.

---

### 12.4 네 관점( PM / Tech Lead / UX / ML ) 총평

* **PM**

  * “이제 이 프로젝트는 ‘실험용’이 아니라,
    **사용자에게 설명할 수 있고, 책임을 질 수 있는 프로덕트** 단계로 올라간다.”

* **Tech Lead**

  * “core/ 중심 구조, Dead-code 제거, 정책 엔진 중앙집중으로
    **앞으로 기능을 추가해도 망가지지 않는 뼈대**가 생겼다.”

* **UX**

  * “Smart Folder, Meeting Agent, RAG 응답이
    **예측 가능하고, 일관된 프롬프트/에러 메시지/흐름**을 갖게 된다.”

* **ML**

  * “모델 교체·임베딩 재학습·drift 대응이
    **파이프라인 레벨에서 표준화**되어, 장기적으로 성능 관리가 가능해진다.”

# 13) Implementation Roadmap (12-Week, 3-Cycle Plan)

본 로드맵은 Smart Folder 기반 경량 AI Agents 제품을 **무료 모델 + 서버리스 환경**에서 안정적으로 완성하기 위한 12주 실행 계획이다.
모든 사이클은 PM / Tech Lead / UX / ML 관점에서의 목표를 포함한다.

---

## **Cycle 1 (Weeks 1–4)**

### 🎯 핵심 목표

* Smart Folder + Policy Engine을 **전 파이프라인에 통합**
* 기존 레거시 파일 구조 제거 → core/ 중심 구조로 재정렬
* Document Boundary와 Policy Enforcement를 확립해 **데이터 안전성** 확보

### 🧩 상세 작업

1. **Policy Engine 구축/정리**

   * 정책 엔진 엔트리: `core/data_pipeline/policies/engine.py` (`PolicyEngine`)
   * 정책 샘플/스키마 정렬: `core/data_pipeline/policies/examples/` (JSON 기반)
   * Smart Folder 컨텍스트 로딩 모듈 생성

2. **Pipeline 전체에 Smart Folder 적용**

   * scan/extract/embed/train/chat 단계 모두 Smart Folder scope 필터 적용
   * 스캔 시 폴더 경계 위반 파일 차단
   * embed/train 단계에서 정책 위반 문서 배제

3. **레거시 구조 제거 및 모듈 이동**

   * filefinder.py → core/data_pipeline/scan.py
   * retriever.py → core/search/retriever.py
   * lnp_chat.py → core/conversation/chat_engine.py
   * pipeline.py → 단계별 Runner로 분산

4. **문서 업데이트**

   * Agents Lifecycle (Smart Folder Scope) 문서 최신화
   * Architecture Overview 개선
   * Park David Docs 1208와 정합성 점검

### ✔ Done Criteria

* Smart Folder 정책 위반 시 모든 에이전트/파이프라인이 **즉시 차단**
* 레거시 파일 최소 80% 제거
* pipeline all 실행 시 Smart Folder 내 자료만 처리됨
* 테스트 최소 10개 통과

---

## **Cycle 2 (Weeks 5–8)**

### 🎯 핵심 목표

* Meeting Agent E2E(End-to-End) 완성
* STT → 요약 → 액션 추출 → 저장 → 마스킹을 단일 파이프라인으로 묶기
* 사용자 경험(UX) 측면에서 "회의 올리면 끝"인 흐름 구축

### 🧩 상세 작업

1. **STT 모듈 통합**

   * Whisper local 모델 최적화
   * 긴 오디오 chunking 처리
   * 실패/잡음 대비 fallback 전략

2. **요약 + 액션 추출 모듈**

   * 무료 모델 최적화 프롬프트 제작
   * 액션 아이템 추출 규칙 기반 + 모델 hybrid
   * 프로젝트명/인물 등 누락 검증 로직 추가

3. **Meeting Result Schema**

   * 요약/하이라이트/액션/근거/원문 timestamp 포함된 JSON
   * Smart Folder별 저장 폴더 구조 확립

4. **민감 정보 마스킹 엔진**

   * 전화번호, 이메일, 금액, 주소 등 패턴 기반 마스킹
   * 정책 기반 role-based masking

5. **Meeting ↔ Knowledge 연동**

   * 회의 결과가 즉시 색인되어 RAG 검색 가능하게 처리

### ✔ Done Criteria

* 회의 파일 1개 입력 → 전체 파이프라인 자동 수행
* 환각률 현저히 감소 (근거 기반 요약)
* Meeting Agent 테스트 20개 이상 통과
* User Flow가 “3-step → 1-step”으로 단순화됨

---

## **Cycle 3 (Weeks 9–12)**

### 🎯 핵심 목표

* RAG/Search 품질을 제품 수준으로 고도화
* Drift detection 및 재임베딩 체계 구축
* Agents 간 교차 동작 완전 정합성 확보

### 🧩 상세 작업

1. **검색 품질 개선**

   * Chunking 고도화 (semantic split + 길이 기반 hybrid)
   * Index ranking 개선 (context overlap penalty, evidence priority ranking)
   * Citation enforcement:

     > “최소 70% 이상의 문장은 실제 문서 근거 기반이어야 한다.”

2. **Drift Detection**

   * 문서 해시 비교 → 변경 문서만 재임베딩
   * 임베딩 벡터 유사도 기반 drift 판단
   * drift 발생 시 자동 re-embed 모드

3. **watch 모드 안정화**

   * 파일 수백·수천 개 변경에도 안정적으로 증분 embedding
   * 잠금(락) 없는 캐시 처리

4. **에이전트 교차 기능 완성**

   * Meeting 결과 → Knowledge Agent 검색
   * Knowledge Agent 근거 문서 기반 → Meeting Agent 요약 확장

### ✔ Done Criteria

* 검색 정확도 80~90% 유지
* drift check 성능 검증 완료
* Meeting ↔ Knowledge 루프 완전 연결
* End-to-End 테스트 30개 이상 통과

---

# 14) Risk-based Execution Order (리스크 기반 우선순위)

프로젝트 위험 요소를 기준으로 **무조건 먼저 해결해야 하는 순서**를 정의한다.

### 1. **데이터 경계 붕괴 위험 (최고 위험)**

* Smart Folder + Policy Engine 미구현 상태에서는 민감 데이터가 노출될 수 있음
  → 해결: Cycle 1에서 최우선 처리

### 2. **Meeting Agent 환각·오역 위험 (중위/상위)**

* STT 오류 + 요약 오류 → 실무 사용 불가
  → 해결: Cycle 2에서 요약·근거 기반 처리 강화

### 3. **검색 품질 저하 위험 (중위)**

* 잘못된 정보 검색 → 신뢰도 하락
  → 해결: Cycle 3의 RAG 고도화

### 4. **레거시 코드 유지 위험 (지속 위험)**

* 구조 중복 → 개발 비용 증가 / 버그 증가
  → 해결: Dead-Code Purge 규칙 준수

---

# 15) Testing Strategy (단위 / 통합 / E2E 구조)

### ■ Unit Tests

* 대상: Policy Engine, Chunking, Meeting Functions
* 목표: Pure Logic 100% 커버
* 위치: `/tests/unit/`

### ■ Integration Tests

* 대상: scan/extract/embed/train pipeline
* 목적: Smart Folder 적용 + 증분 처리 테스트
* 위치: `/tests/integration/`

### ■ Agent Tests

* Meeting Agent E2E
* Knowledge Agent RAG end-to-end
* 위치: `/tests/agents/`

### ■ E2E Tests

* 실제 Smart Folder + 실제 문서 + 실제 회의 파일로
  “사용자 시나리오 전체” 검증
* 30개 이상 시나리오 목표
* 위치: `/tests/e2e/`

### ■ Drift Detection Tests

* 문서 변경 여부에 따라 embedding 재처리 여부 확인

### ✔ 테스트 통과 기준

```
pytest -q 100% 통과
infopilot.py pipeline all 정상 실행
Meeting Agent, Knowledge Agent 둘 다 end-to-end 성공
```

---

# 16) Release Checklist (릴리즈 절차)

### 🔧 Pre-Release

1. 문서 최신화 (Unified Spec + Agents Lifecycle + Plan 업데이트)
2. Dead-code 제거
3. 테스트 전체 통과
4. Smart Folder 정책 검증

### 🚀 Release

1. develop → main merge
2. 태그 생성:
   `v1.0.0-smartfolder`
3. release note 작성:

   * 변경 기능
   * 삭제된 기능
   * known issues

### 📦 Post-Release

1. drift 체크 스케줄 실행
2. feedback 로그 기반 개선 로드맵 확정

---

# 17) Operating Manual (운영 가이드)

### ■ 폴더 운영 규칙

1. Smart Folder 외부 파일은 절대 파이프라인에 넣지 않음
2. 정책 변경 시 즉시 재온보딩 수행
3. drift 체크 주기: 1일 또는 3일

### ■ 개발 운영 규칙

1. 모든 기능 추가는 PR + 문서 변경이 동시에 필요
2. 레거시 또는 중복 코드 발견 시 즉시 삭제 PR 제출
3. 테스트 깨진 상태로 merge 불가

### ■ 회의 에이전트 운영 규칙

* STT 실패 시 fallback 안내
* 민감 단어 마스킹 로그 별도 저장
* 회의 결과 저장 폴더는 Smart Folder 하위로 강제

### ■ 검색/RAG 운영 규칙

* 재임베딩 필요 문서 자동 감지
* Citation 없는 응답은 실패로 간주
* evidence-first UI 원칙 준수

---

# ★ Final Summary

13~17은 제품을 “운영 가능한 수준”에서 “실제로 유지·확장 가능한 제품”으로 만드는 실행계획이다.
이제 Park David Plan은 완전한 상태에 가까우며,
이후 단계는 실제 개발 착수와 테스트 작성으로 자연스럽게 이어진다.

원하면 **전체 1~17 통합본**도 만들어줄게.
