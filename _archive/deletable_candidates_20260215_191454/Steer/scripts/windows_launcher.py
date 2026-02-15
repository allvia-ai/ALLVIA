"""윈도우 전용 InfoPilot 런처.

PyInstaller로 빌드했을 때 더블 클릭만으로 파이프라인(스캔+학습)과
Chat 모드를 실행할 수 있는 간단한 콘솔 메뉴를 제공한다.

개발 환경에서도 `python windows_launcher.py` 로 바로 실행 가능하다.
"""
from __future__ import annotations

import os
import sys
from typing import Iterable, List

from core.config.paths import CACHE_DIR, CORPUS_PATH, DATA_DIR, TOPIC_MODEL_PATH


DEFAULT_SCAN_CSV = DATA_DIR / "found_files.csv"
DEFAULT_CORPUS = CORPUS_PATH
DEFAULT_MODEL = TOPIC_MODEL_PATH
DEFAULT_CACHE = CACHE_DIR


def run_infopilot(argv: Iterable[str]) -> None:
    """infopilot.py의 main 함수를 직접 호출한다."""
    from infopilot import main as infopilot_main

    original = sys.argv
    try:
        sys.argv = ["infopilot"] + list(argv)
        infopilot_main()
    except SystemExit as exc:  # argparse가 내부적으로 사용하는 종료 처리
        code = exc.code if isinstance(exc.code, int) else 0
        if code not in (0, None):
            print(f"❌ 명령이 실패했습니다. 종료 코드: {code}")
    finally:
        sys.argv = original


def ensure_default_dirs() -> None:
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    DEFAULT_CACHE.mkdir(parents=True, exist_ok=True)


def prompt_paths() -> List[str]:
    roots_raw = input("스캔할 루트 경로를 입력하세요 (여러 개는 세미콜론 ';' 구분, 공백 또는 Enter=전체 스캔): ").strip()
    if not roots_raw:
        return []
    roots: List[str] = []
    for raw in roots_raw.split(';'):
        candidate = raw.strip().strip('"')
        if candidate:
            roots.append(candidate)
    return roots


def run_pipeline_with_chat() -> None:
    ensure_default_dirs()
    roots = prompt_paths()

    cmd: List[str] = [
        "pipeline",
        "--out",
        str(DEFAULT_SCAN_CSV),
        "--corpus",
        str(DEFAULT_CORPUS),
        "--model",
        str(DEFAULT_MODEL),
        "--cache",
        str(DEFAULT_CACHE),
        "--launch-chat",
    ]
    for root in roots:
        cmd.extend(["--root", root])

    translate = input("번역 모드를 활성화할까요? (y/N): ").strip().lower()
    if translate in {"y", "yes", "1"}:
        cmd.append("--translate")

    print("\n🚀 파이프라인을 시작합니다. (중단하려면 Ctrl+C)\n")
    run_infopilot(cmd)


def run_chat_only() -> None:
    ensure_default_dirs()
    if not DEFAULT_MODEL.exists() or not DEFAULT_CORPUS.exists():
        print("⚠️ 모델 또는 코퍼스 파일이 없어 chat을 실행할 수 없습니다. 먼저 파이프라인을 수행하세요.")
        return

    translate = input("번역 모드를 활성화할까요? (y/N): ").strip().lower()
    rerank = input("Cross-Encoder 재랭킹을 사용할까요? (Y/n): ").strip().lower()

    cmd: List[str] = [
        "chat",
        "--scan_csv",
        str(DEFAULT_SCAN_CSV),
        "--corpus",
        str(DEFAULT_CORPUS),
        "--model",
        str(DEFAULT_MODEL),
        "--cache",
        str(DEFAULT_CACHE),
    ]

    if translate in {"y", "yes", "1"}:
        cmd.append("--translate")
    if rerank in {"n", "no", "0"}:
        cmd.append("--no-rerank")

    print("\n💬 Chat 모드를 실행합니다. (종료하려면 'exit')\n")
    run_infopilot(cmd)


def show_menu() -> None:
    while True:
        print("""
======================
 InfoPilot Windows 런처
======================
1) 전체 파이프라인 실행 후 Chat 자동 실행
2) Chat만 실행 (이미 학습된 모델 필요)
3) 종료
""")
        choice = input("메뉴를 선택하세요 (1-3): ").strip()
        if choice == "1":
            run_pipeline_with_chat()
        elif choice == "2":
            run_chat_only()
        elif choice == "3":
            print("👋 종료합니다.")
            break
        else:
            print("⚠️ 1~3 숫자 중에서 선택해주세요.")


def main() -> None:
    os.environ.setdefault("PYTHONUTF8", "1")  # Windows 콘솔 한글 깨짐 방지용
    show_menu()


if __name__ == "__main__":
    main()
