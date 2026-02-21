# Steer OS Build/Deploy Runbook (macOS)

`Steer OS.app`가 오래된 바이너리를 물거나, 빌드 산출물 앱과 `/Applications` 앱이 섞이는 문제를 막기 위한 운영 문서입니다.

## 원칙

- 실행은 항상 `/Applications/Steer OS.app`만 사용합니다.
- 개발 산출물 앱(`web/src-tauri/target/release/bundle/macos/Steer OS.app`)은 직접 실행하지 않습니다.
- 재배포는 병합 복사가 아니라 **완전 교체**로 진행합니다.

## 표준 명령 (권장)

```bash
./scripts/rebuild_and_deploy.sh
```

이 스크립트는 아래를 자동으로 수행합니다.

1. `core` 릴리즈 빌드
2. 실제 서버 바이너리(`local_os_agent`)를 Tauri externalBin(`core-aarch64-apple-darwin`)으로 동기화
3. `npm run tauri build`
4. 실행 중인 `Steer OS` 프로세스 종료
5. `/Applications/Steer OS.app` 완전 교체 배포
6. `http://127.0.0.1:5680/api/system/health` 헬스체크

## 빠른 개발 루프 (권장)

패키징/앱 교체를 매번 하지 말고, 먼저 CLI로 코어 검증을 끝낸 뒤 마지막에 1회만 배포합니다.

```bash
# 1) 코어만 빠르게 검증 (기본 debug, 별도 포트 15680)
./scripts/validate_core_cli.sh --goal "메모장 열어서 박대엽이라고 써줘"

# 2) 검증 통과 후 최종 1회 배포
./scripts/rebuild_and_deploy.sh
```

- `validate_core_cli.sh`는 `/Applications` 앱을 건드리지 않습니다.
- 기본 포트가 `15680`이므로 현재 실행 중인 앱(`5680`)과 충돌하지 않습니다.
- `--release`를 주면 릴리즈 바이너리로도 동일 검증이 가능합니다.

## 빠른 확인 체크리스트

- `app`/`core` 프로세스가 둘 다 존재
- `127.0.0.1:5680` LISTEN
- `curl http://127.0.0.1:5680/api/system/health` 정상 응답

## 자주 만나는 증상과 원인

- `Network Error`: `core`가 죽었거나 뜨지 않음
- 앱은 열리는데 타이핑 실패(예: AppleScript 1002):
  `시스템 설정 > 개인정보 보호 및 보안 > 손쉬운 사용/자동화` 권한 점검 필요
- Spotlight에서 `Steer OS`가 2개 보임:
  운영용(`/Applications`)과 개발 산출물 앱이 동시에 존재
  개발용은 `Steer OS Dev.app` 이름으로 분리 유지 권장
