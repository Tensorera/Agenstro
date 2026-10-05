"""Deliberately wrong implementation for the red phase of the example."""
import sys


def add(left: int, right: int) -> int:
    return left - right


if __name__ == "__main__":
    print(add(int(sys.argv[1]), int(sys.argv[2])))
