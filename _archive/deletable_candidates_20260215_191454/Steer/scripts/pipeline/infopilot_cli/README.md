# InfoPilot CLI modules

`scripts/pipeline/infopilot.py`는 CLI 엔트리포인트(Click 그룹/커맨드 등록)만 유지하고,
커맨드 구현/공통 유틸은 이 패키지로 분리합니다.

## 모듈 구성 (파이프라인 순서)
- `scan.py`: scan 실행/명령
- `scan_rows.py`: scan CSV 파서/정규화/정책 필터
- `train_config.py`: TrainConfig 생성, row limit, 기본값
- `steps.py`: extract/embed/train 실행
- `index.py`: 인덱스 재생성
- `chat.py`: chat 모드(자동 train 포함)
- `watch.py`: watchdog 기반 증분 watch 파이프라인
- `drift.py`: drift check/auto/reembed용 공통 로직
- `policy.py`: 정책 로드/캐시 한도/루트 파싱
- `session.py`: MLflow/리소스 로깅 컨텍스트
- `history.py`: 최근 경로 히스토리 저장/복원

## 엔트리포인트
- 개발 중: `python scripts/pipeline/infopilot.py ...`
- 호환 shim: `python infopilot.py ...` (내부적으로 위 CLI로 위임)
