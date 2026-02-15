# 아키텍처 개요

InfoPilot 데스크톱 비서는 세 개의 층으로 구성되어 있습니다.

- **코어 서비스**: `core/` 디렉터리에 위치하며 스캔·학습·검색·대화를 처리합니다. `core/data_pipeline`, `core/search`, `core/conversation` 하위 모듈은 공용 Python API를 제공합니다.
- **자동화 스크립트**: `scripts/`에 모여 있으며 `pipeline all` 또는 `run <command>` 형태로 스캔·학습·대화를 실행합니다. 최상위 진입점은 `scripts/infopilot.py`로, 실제 구현은 `scripts/pipeline/infopilot.py`에 위임됩니다.
- **데스크톱 UI**: `ui/`에 있는 CustomTkinter 기반 앱이 동일한 API를 불러 사용자 인터페이스를 제공합니다.

## 데이터 흐름

1. **스캔** (`infopilot.py run scan`): `core/data_pipeline/filefinder.FileFinder`를 이용해 `data/found_files.csv`를 생성하고 스캔 상태(`data/scan_state.json`)를 갱신합니다.
2. **학습** (`infopilot.py pipeline all` 또는 `infopilot.py run train`): 텍스트 청크를 정규화하고 `data/corpus.parquet`, `data/topic_model.joblib`, 문서 해시 캐시(`data/cache/chunk_cache.json`)를 갱신합니다. 증분 실행 시 변경된 문서만 재임베딩합니다.
3. **질의/대화** (`infopilot.py run chat`): `core/conversation/LNPChat`과 `core/search/retriever.Retriever`가 결합해 하이브리드 검색(semantic + BM25×0.35)·CrossEncoder rerank·Temporal weighting을 적용합니다.
4. **감시** (`infopilot.py run watch`): 파일 변경 이벤트를 감지해 코퍼스·인덱스를 증분으로 갱신하고, drift 탐지 결과는 `infopilot.py drift check/reembed`로 확인할 수 있습니다.

주요 산출물 경로는 `core/config/paths.py`에서 확인할 수 있습니다.

## 에이전트 구성

- **지식·검색 비서**: BGE-m3 SentenceTransformer 임베딩, 기본 BM25 가중치(0.35), Temporal weighting, Cross-Encoder 재랭킹을 조합합니다. CLI 플래그(`--lexical-weight`, `--rerank-min-score`, `--translate` 등)로 세부 설정을 조정할 수 있습니다. 자세한 구조는 `docs/agents/document/architecture.md` 참고.
- **회의 비서**: `core/agents/meeting/pipeline.py`가 STT, 요약, 감사 로그, 분석을 묶어 실행합니다. 세부 내용은 `docs/agents/meeting/architecture.md`에 정리되어 있습니다.
- **사진 비서**: `core/agents/photo/pipeline.py`가 태깅, 중복 감지, 베스트샷 추천을 수행합니다. `docs/agents/photo/architecture.md`에서 단계별 설명을 확인할 수 있습니다.

세 에이전트는 공통 인프라(`core/infra`의 모델 매니저, 스케줄러, 로깅)를 공유합니다.

### 스마트 폴더 정책 & 보안
- 정책 엔진(`core/data_pipeline/policies/engine.py`)은 폴더별 정책을 로딩하며, 캐시 전략·민감 폴더 제외 옵션을 확장할 예정입니다.
- 전역 접근 모드(폴더 미선택)에서도 캐시 용량 · 정리 정책이 일관되도록 `docs/plan/product_alignment.md` 지침을 따릅니다.
- 민감 폴더 기능이 활성화되면 정책 JSON에 제외 경로를 기록하고, 스캐너/에이전트는 해당 경로를 자동으로 건너뜁니다.

## CLI 대화 루프 요약

`scripts/pipeline/infopilot.py`는 필요 시 학습 산출물을 다시 생성한 뒤 `LNPChat` 인스턴스를 준비합니다. 이후에는

- 대화형 모드에서 질의를 입력받아 상위 결과, 번역, Temporal/lexical/임베딩 근거를 출력하거나,
- `--query`와 `--json` 옵션으로 단일 질의 결과를 JSON 형태로 반환합니다.

추가 보조 커맨드로 `logs show/clean`, `model quantize`, `drift check/reembed`를 제공해 MLOps 메트릭과 리소스 사용량을 추적할 수 있습니다.

세션 상태는 최근 질의와 선호 정보를 기록하므로 클릭/핀/좋아요·싫어요 피드백이 다음 검색에 반영됩니다.
