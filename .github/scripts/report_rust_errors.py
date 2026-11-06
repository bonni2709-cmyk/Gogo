#!/usr/bin/env python3
"""Выводит ошибки rustc из --message-format=json в GitHub Annotations.

Использование: report_rust_errors.py <cargo-json-output> <exit-status>
Файлы логов Actions недоступны из песочницы агента, поэтому ошибки
показываются через Annotations API.
"""
import json
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")
MAX_ERRORS = 8
MAX_LINES_PER_ERROR = 14


def emit(title: str, text: str) -> None:
    """Одна аннотация на ошибку: GitHub ограничивает их число на шаг."""
    lines = text.splitlines()[:MAX_LINES_PER_ERROR]
    # %0A — escape перевода строки внутри команды workflow.
    joined = "%0A".join(line.strip() for line in lines if line.strip())
    if joined:
        print(f"::error title={title}::{joined}")


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: report_rust_errors.py <json-log> <exit-status>")
        return 2

    path, status = sys.argv[1], int(sys.argv[2])
    emitted = 0

    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for raw in fh:
                raw = raw.strip()
                if not raw.startswith("{"):
                    continue
                try:
                    payload = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                if payload.get("reason") != "compiler-message":
                    continue
                msg = payload.get("message") or {}
                if msg.get("level") != "error":
                    continue
                rendered = ANSI.sub("", msg.get("rendered") or msg.get("message", ""))
                if rendered.strip():
                    emit("Rust compiler", rendered)
                    emitted += 1
                if emitted >= MAX_ERRORS:
                    break
    except FileNotFoundError:
        pass

    if emitted == 0:
        # Ошибки до этапа компиляции (манифест, тулчейн) печатаются из stderr.
        import os

        stderr_path = os.path.splitext(path)[0] + ".err"
        try:
            with open(stderr_path, encoding="utf-8", errors="replace") as fh:
                tail = "".join(fh.readlines()[-40:])
            if tail.strip():
                emit("cargo", tail)
        except FileNotFoundError:
            pass
        if status != 0:
            print(f"::error title=cargo::cargo завершился с кодом {status}")

    sys.exit(status if status != 0 else 0)


if __name__ == "__main__":
    main()
