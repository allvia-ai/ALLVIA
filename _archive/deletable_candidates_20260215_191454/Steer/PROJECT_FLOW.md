# InfoPilot 흐름 가이드

이 문서는 저장소의 주요 흐름(코어 파이프라인, 스마트 폴더 정책, 대화 비서, 문서 정리 비서)을 한눈에 파악할 수 있도록 실행 명령과 함께 정리했습니다. 모든 명령은 저장소 루트에서 수행합니다.

---

## 1. 저장소 전체 흐름

### 구조 요약
- `core/`: 스캔·학습·검색·대화를 담당하는 핵심 모듈.
- `scripts/`: 파이프라인 실행과 에이전트 실행 스크립트.
- `ui/`: CustomTkinter 기반 데스크톱 앱.
- `docs/`: 기능별 최소 문서 모음 (architecture/guides/process/research/roadmap/ux).

### 기본 실행 순서
1. **의존성 설치**
   ```bash
   bash scripts/setup_env.sh
   # 또는 개발 단축 명령
   bash scripts/dev/setup_env.sh
   ```
   `scripts/setup_env.sh`는 `.env`가 없으면 `.env.example`을 복사합니다. 필요 시 수동으로 `cp .env.example .env` 후 값을 수정하세요.
2. **데이터 파이프라인**
   ```bash
   # 전체 파이프라인 한 번에 (증분 상태/해시 캐시 포함)
   python3 infopilot.py pipeline all \
     --out data/found_files.csv \
     --corpus data/corpus.parquet \
     --model data/topic_model.joblib \
     --cache data/cache \
     --state-file data/scan_state.json \
     --chunk-cache data/cache/chunk_cache.json

   # 필요 시 개별 단계
   python3 infopilot.py run scan --out data/found_files.csv
   python3 infopilot.py run train --scan_csv data/found_files.csv --state-file data/scan_state.json --chunk-cache data/cache/chunk_cache.json
   python3 infopilot.py run chat --model data/topic_model.joblib --corpus data/corpus.parquet --cache data/cache --lexical-weight 0.35
   python3 infopilot.py run watch --corpus data/corpus.parquet --model data/topic_model.joblib --cache data/cache
   ```

   로그/품질 점검용 보조 커맨드도 자주 사용합니다.

   ```bash
   python3 infopilot.py logs show
   python3 infopilot.py drift check --scan-csv data/found_files.csv --corpus data/corpus.parquet
   python3 infopilot.py model quantize --model sentence-transformers/all-MiniLM-L6-v2 --output models/sbert.onnx
   ```
3. **데스크톱 UI**
   ```bash
   python3 scripts/launch_desktop.py
   # 또는 직접 실행
   python3 ui/app.py
   ```

4. **오케스트레이션 (선택 사항)**
   ```bash
   # Prefect Flow로 전체 파이프라인 실행
   python3 scripts/prefect_dag.py --root ~/Documents --use-prefect

   # FastAPI 서버로 REST 제어
   python3 scripts/api_server.py
   # POST /pipeline/run  → 파이프라인 시작
   # GET  /pipeline/status → 진행 상황 조회
   # POST /pipeline/cancel → 중단
   ```

---

## 2. 스마트 폴더 흐름

스마트 폴더 정책은 스캔·학습·검색 대상과 보안/보존 정책을 제어합니다.

### 정책 위치 및 기본값
- 정책 파일: `core/config/smart_folders.json`
- 스키마/예시: `core/data_pipeline/policies/schema/` 및 `.../examples/`

### 정책을 활용한 실행
1. 정책 파일을 편집합니다.
2. 정책을 반영해 파이프라인을 실행합니다.
   ```bash
   python3 infopilot.py pipeline all \
     --out data/found_files.csv \
     --policy core/config/smart_folders.json \
     --corpus data/corpus.parquet \
     --model data/topic_model.joblib
   ```
3. 정책 범위를 강제하고 싶다면 대화 시 `--scope policy`를 사용합니다.
   ```bash
   python3 infopilot.py run chat \
     --policy core/config/smart_folders.json \
     --scope policy \
     --model data/topic_model.joblib \
     --corpus data/corpus.parquet \
     --cache data/cache
   ```
4. 정책 기반 예약 실행
   ```bash
   python3 infopilot.py schedule \
     --policy core/config/smart_folders.json \
     --agent knowledge_search \
     --output-root data/scheduled_runs
   ```

---

## 3. 대화 비서(지식·검색) 흐름

### 학습된 문서로 자연어 대화
```bash
python3 infopilot.py run chat \
  --model data/topic_model.joblib \
  --corpus data/corpus.parquet \
  --cache data/cache
```

### 단일 질의 + JSON 응답
```bash
python3 infopilot.py run chat \
  --query "보안 가이드 요약해 줘" \
  --json \
  --model data/topic_model.joblib \
  --corpus data/corpus.parquet \
  --cache data/cache
```

### 로컬 LLM 연결 확인
```bash
python3 scripts/check_local_llm.py --backend ollama --model llama3
```
환경 변수 설정 예시는 `docs/guides/local_llm.md` 참고.

### 회의/사진 비서 자동 호출
- 대화 비서에서 회의록 요약이나 사진 정리를 요청하면 오케스트레이터가 자동으로 `meeting_summary` 또는 `photo_manager` 에이전트를 선택합니다.
- 추가 정보가 필요하면 follow-up 메시지가 출력되어 오디오 파일 경로나 사진 폴더를 지정하도록 안내합니다.
- CLI 모드에서는 최근 사용 경로 목록이 함께 표시되어 번호 선택 또는 직접 입력으로 값을 채울 수 있고, 데스크톱 UI에서는 파일/폴더 선택 다이얼로그와 최근 항목 버튼이 제공됩니다.
- 작업이 시작되면 진행 단계가 상태 표시줄에 실시간으로 갱신되고, 취소 버튼으로 장시간 실행을 중단할 수 있습니다. 취소 시 다음 실행에서 `enable_resume` 옵션이 기본 활성화되어 동일한 오디오를 이어서 처리합니다.

---

## 4. 에이전트별 흐름

### 회의 비서 (STT → 요약)
```bash
python3 scripts/run_meeting_agent.py \
  --folder-path "/Users/me/AI Summary/녹음" \
  --policy-path core/config/smart_folders.json \
  --audio path/to/meeting.m4a \
  --output-dir data/meetings/output_001 \
  --output-json
```

### 사진 비서 (태깅·중복 정리)
```bash
python3 scripts/run_knowledge_agent.py \
  --roots "/Users/me/Pictures" \
  --output-dir data/photo_outputs
```

에이전트 실행 후 요약 결과(`summary.json`, `photo_report.json`)를 리뷰하세요.

---

## 부록: 테스트 및 릴리스 전 점검

- 벤치마크: `python3 -m scripts.benchmarks.ann_benchmark ...`
- 정확도 평가: `python3 -m scripts.benchmarks.accuracy_eval ...`
- 릴리스 가이드: `docs/process/release.md`
- 문서 비서 설계/운영: `docs/agents/document/README.md`
- 회의 비서 설계/운영: `docs/agents/meeting/README.md`
- 사진 비서 설계/운영: `docs/agents/photo/README.md`
- KPI 스냅샷: `python3 scripts/util/release_prepare.py --print`
