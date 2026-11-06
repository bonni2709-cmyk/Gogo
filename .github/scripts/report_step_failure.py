#!/usr/bin/env python3
"""Аннотирует падение шага CI хвостом его лога.

Использование: report_step_failure.py <log-file> [title]
Логи Actions недоступны из песочницы агента, поэтому последние строки
шага публикуются как GitHub Annotation (читается через API).
"""
import sys


def main() -> int:
    if len(sys.argv) < 2:
        print("::error title=report::нет файла лога")
        return 2

    path = sys.argv[1]
    title = sys.argv[2] if len(sys.argv) > 2 else "step failed"

    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            lines = [line.strip() for line in fh if line.strip()]
    except FileNotFoundError:
        lines = []

    tail = lines[-40:]
    if not tail:
        tail = ["(пустой лог)"]

    print(f"::error title={title}::" + "%0A".join(tail))
    return 1


if __name__ == "__main__":
    sys.exit(main())
