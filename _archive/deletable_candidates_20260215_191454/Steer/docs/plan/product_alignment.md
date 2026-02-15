# Roadmap v3 Alignment Guide

이 문서는 리팩토링 이후 코드와 문서가 동일한 방향을 유지하도록 제품 기획 정보를 통합한 문서입니다.  
`README`, `docs/architecture/overview.md`, `docs/agents/*`와 내용이 상충하지 않도록 기준을 명시합니다.

## 1. 비전 & 핵심 원칙
- **로컬 우선(Offline-first)**: 가능한 모든 파이프라인은 로컬 자원으로 동작하고, 클라우드 자원은 명시적으로 opt-in.
- **스마트 폴더 중심 UX**: 정책, 권한, 캐시 전략은 폴더 단위로 관리하며, 사용자가 정책 없이 전체 접근 권한을 줄 때의 동작도 정의해야 합니다.
- **모듈형 에이전트**: 문서·회의·사진 비서는 독립 모듈이지만 동일한 오케스트레이터(`core/agents`)와 설정(`core/config`)을 공유합니다.
- **명령어 기반 호출**: 사용자는 `/search`, `/meeting`, `/photo` 명령어를 통해 기능을 명시적으로 실행하며, 일반 대화는 문답에 집중합니다.

## 2. 릴리스 사이클 (Roadmap v3)
| Cycle | 중심 과제 | 관련 코드/문서 | 상태 |
| --- | --- | --- | --- |
| Cycle 0 | 코어 파이프라인 `scan → train → chat`, 정책 엔진, 캐시 초기화 | `core/data_pipeline`, `docs/agents/document/*` | ✅ 완료 (배포 중) |
| Cycle 1 | 지식·검색 비서 고도화, 정책 스코프 전환, Work Center 진입점 | `core/agents/document`, `ui/screens/conversation_screen.py` | ⏳ 진행 중 (LNPChat·정책 스코프 반영, Work Center 남음) |
| Cycle 2 | 회의 비서 STT/요약 파이프라인, 액션/결정 추적, 통합 캐시 | `core/agents/meeting`, `docs/agents/meeting/*` | ⏳ 진행 중 (파이프라인 구현 완료, 캐시/민감 폴더/알림 미완) |
| Cycle 3 | 사진 비서 태깅/중복/베스트샷, GPU/ONNX 플로우 | `core/agents/photo`, `docs/agents/photo/*` | ⏳ 진행 중 (MVP 존재, 태깅/ONNX 최적화 예정) |
| Cycle 4 | 운영·하이브리드: 감사, 권한, 오프로딩, 리소스 모니터링 | `core/infra`, `docs/plan/product_alignment.md` | 🔜 계획 (설계만 존재) |

모든 신규 기능은 위 표의 Cycle과 연결하여 README/문서/코드 주석을 업데이트합니다.

## 3. 즉시 우선 과제 (캐시 · 보안 · UX)
### 3.1 스마트 폴더 · 캐시 정책 재정의
- 이전에 언급된 **폴더별 2GB 상한** 가정은 폐기합니다. 스마트 폴더별로 최대 용량·정리 주기를 직접 설정할 수 있도록 설계합니다.
- 코드/문서 정렬:
  - `core/config/smart_folders.json`: 캐시 옵션 필드 정의 및 문서화.
  - `core/search/retriever.py`: 인덱스 캐시 감시/정리 로직과 연동하는 TODO 명시.
  - 전역 모드(폴더 미선택)에서는 `data/cache/` 전체를 대상으로 사용량을 표시하고 관리 정책을 UI에 노출합니다.

### 3.2 민감 폴더 제어
- 전체 디렉터리 접근 권한을 부여했을 때 제외할 경로(`sensitive_paths`)를 정책으로 등록할 수 있어야 합니다.
- 정책 엔진(`core/data_pipeline/policies/engine.py`)에 해당 필드를 추가하고, 스캐너·에이전트에서 자동으로 건너뛰도록 합니다.
- UI에서는 폴더 인스펙터에 “민감 폴더 제외” 토글을 제공하고, 설정 변경 시 정책 JSON을 즉시 갱신합니다.

### 3.3 폴더 인스펙터 & 캐시 표시
- Work Center/설정 화면의 폴더 인스펙터에서 각 폴더의 캐시 사용량과 민감 폴더 상태를 한눈에 보여줍니다.
- “캐시량”은 `data/cache/<폴더 식별자>` + STT/요약 캐시 디렉터리 합계로 계산합니다.
- UI 명세는 `docs/guides/ui_help.md`, `docs/ux/smart_folder_glass_ui.md`와 동일하게 유지하고, 실제 계산 함수(`core/search/retriever.py` 또는 전용 유틸)와 연결합니다.

## 4. 검증 & 전달
### 4.1 테스트 체크리스트
| 시나리오 | 체크 포인트 |
| --- | --- |
| 스마트 폴더 캐시 | 폴더 추가/삭제 후 캐시 사용량이 UI·로그에서 일관되게 표시되는지 확인 |
| 전체 접근 모드 | 폴더 미선택 상태에서 스캔/학습/대화 실행 후 캐시 용량 정책이 지켜지는지 확인 |
| 민감 폴더 제외 | 제외 경로가 스캔/요약 대상에서 빠지고 감사 로그에 기록되는지 확인 |
| 정책 파일 변경 | `smart_folders.json` 수정 후 즉시 반영되는지, 잘못된 정책은 오류로 드러나는지 확인 |
| 회의/사진 비서 캐시 | 동일 입력 재실행 시 캐시가 재사용되고, 모델 변경 등 무효화 조건이 제대로 동작하는지 확인 |

테스트는 `pytest` 기반 자동화와 수동 QA를 병행하고, 결과를 `results/` 폴더에 누적합니다.

### 4.2 다음 배포 목표
1. 스마트 폴더 정책 스키마에 캐시 용량·민감 폴더 필드를 추가하고 문서 반영.
2. 폴더 인스펙터 UI에 캐시량·민감 폴더 토글을 노출하고 측정 로거 구현.
3. 구형 문서/코드(2GB 상한 가정 등)를 제거하고 본 문서와 불일치하는 부분 리팩토링.
4. 테스트 스위트를 위 체크리스트와 연동해 CI에 편입.

## 5. 향후 확장 제안 (메모리·검토 필요)
> 아래 항목은 **Full Assistant Edition** 업그레이드 계획(v2)에서 가져온 아이디어입니다.  
> 제조사 요구(저메모리, 안정성)와 충돌할 수 있으니 도입 전에 반드시 용량/비용을 재검토하세요.

### 5.1 검색 정확도 & 모델 세분화
- **Semantic Reranker**: `core/search/reranker.py` 추가, `config/model_config.json`으로 threshold 관리 *(검토: RAM + GPU 500MB 내)*.
- **Agent별 모델 설정**: `config/agent_meeting.json`, `agent_photo.json`, `agent_knowledge.json`으로 모델 경로/옵션 분리 *(검토: 유지보수 증가)*.
- **Model Manager 확장**: 지연 로딩, GPU/CPU 자동 전환, 캐시 관리 (`core/model_manager.py`) *(검토: 초기화 복잡도)*.

### 5.2 UI & 배포
- **데스크톱 Wrapping**: CustomTkinter 앱을 PyInstaller로 패키징 *(Atlas UI 기준)*.
- **빌드 스크립트**: `scripts/build_desktop_ui.bat/.ps1`에서 `--sign-cmd`/`-SignCommand`로 코드 서명까지 자동화.

#### 5.2.1 단계별 계획 요약
| 단계 | 작업 | 설명 |
| --- | --- | --- |
| **1️⃣ Retriever 개선** | BM25 + BGE 구조에 `semantic rerank` 추가 (`cross-encoder/ms-marco-MiniLM-L-6-v2` 등) | 문서 유사도 정밀도 향상 (RAM 영향 500MB 내 검토) |
| **2️⃣ Agent별 모델 세분화** | Meeting→Whisper+Llama3, Photo→CLIP/ONNX, Knowledge→BGE/Reranker | 각 Agent 최적화 모델 구성 (모델 수 늘어나므로 관리 주의) |
| **3️⃣ Model Manager 확장** | GPU/CPU 자동 전환, lazy-load, 로컬 우선 캐시 | 로드 속도 및 자원 최적화 (초기화 복잡도 증가) |
| **4️⃣ 데스크톱 패키징** | PyInstaller 기반 단일 실행 파일/디렉터리 생성 | 배포 편의성 확보 |
| **5️⃣ 로컬 빌드 및 배포** | 스크립트로 코드 서명(`--sign-cmd`)·압축·릴리스 노트 생성 | 내부 배포 혹은 개인용 앱 완성 |
| **6️⃣ (선택) Upstream 반영** | 검증 완료 항목만 PR로 `develop`에 반영 | GitHub 코드베이스를 통제된 방식으로 유지 |

### 5.3 모델별 메모리 목표
| Agent | 모델 구성 | 비고 | 목표 메모리 |
| --- | --- | --- | --- |
| Knowledge | BGE + Reranker | 의미 검색 + 정밀 재정렬 | ≤ 500MB |
| Meeting | Whisper + Llama3 | 음성→요약, 액션 아이템 | ≤ 1.5GB |
| Photo | CLIP/ONNX | 태깅·중복 정리 | ≤ 1GB |
| ModelManager | Lazy Load | GPU/CPU 자동 전환 | Idle 300–500MB |

### 5.4 추가 테스트 항목 (검토 단계)
- BGE-only vs BGE+Reranker 정확도 비교.
- 모델 매니저의 lazy-loading 및 GPU/CPU 전환 시나리오.
- PyInstaller 패키지 실행 시 CLI와 동등하게 동작하는지 확인.
- 빌드 아티팩트(.exe/.dmg) 실행 및 CLI와의 동등성 확인.
- `/data/cache` 초기화 후 재생성 정상 여부.
- `INFOPILOT_CACHE_BACKEND=sqlite`, `INFOPILOT_CACHE_MAX_ENTRIES` 환경 변수 조합에서 캐시가 안정적으로 GC되는지 확인.
- `scripts/edge_adapter.py export/serve` 출력물로 모바일/Edge 검색이 정상 처리되는지 확인.

### 5.5 후속 아이디어
- OpenAI-compatible API 어댑터(Ollama/LM Studio 연동).
- 정책 엔진 기반 에이전트 제한(스마트 폴더별).
- 에이전트 간 컨텍스트 공유(회의 → 지식 추천 등).
- Edge Adapter(`scripts/edge_adapter.py export/serve`)를 활용한 모바일/임베디드 배포 자동화.
