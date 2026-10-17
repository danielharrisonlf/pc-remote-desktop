#!/usr/bin/env python3
"""Return success after the expected embedded UI content appears in AT-SPI."""

import argparse
import sys

import pyatspi


parser = argparse.ArgumentParser()
parser.add_argument("--contains", required=True)
args = parser.parse_args()


def walk(node):
    try:
        name = node.name or ""
        role = node.getRoleName()
    except Exception:
        return
    yield role, name
    try:
        count = node.getChildCount()
    except Exception:
        return
    for index in range(count):
        try:
            child = node.getChildAtIndex(index)
        except Exception:
            continue
        if child is not None:
            yield from walk(child)


try:
    desktop = pyatspi.Registry.getDesktop(0)
    matches = [(role, name) for role, name in walk(desktop) if args.contains in name]
except Exception as exc:
    print(f"AT-SPI query failed: {exc}", file=sys.stderr)
    sys.exit(2)

if not matches:
    print(f"accessible content not found: {args.contains}", file=sys.stderr)
    sys.exit(1)

for role, name in matches:
    print(f"accessible {role}: {name}")
