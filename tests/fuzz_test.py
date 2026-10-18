#!/usr/bin/env python3
"""Randomized test for both schedulers.

Generates random teams with random ranked preferences, runs the Python and
Rust programs on each with the same seed, and checks that

  1. the two programs print exactly the same output,
  2. the printed schedule obeys every rule, verified here from the printed
     table rather than from either program's own rule check:
       - nobody appears twice on the same day,
       - nobody works more than five days,
       - no shift has more than three people,
       - every shift has at least two people whenever the team has enough
         working days (team size x 5 >= 42),
  3. every unmarked name sits on that employee's first choice for the day.

Usage (from the repository root, after `cargo build --release` in rust/):
  python3 tests/fuzz_test.py [trials]
"""
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PYTHON_CMD = [sys.executable, os.path.join(ROOT, "python", "scheduler.py")]
RUST_CMD = [os.path.join(ROOT, "rust", "target", "release", "scheduler")]
DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
SHIFTS = ["Morning", "Afternoon", "Evening"]


def random_team(rng):
    size = rng.randint(5, 18)
    team = {}
    for n in range(size):
        off_rate = rng.uniform(0.0, 0.8)
        days = []
        for _ in DAYS:
            if rng.random() < off_rate:
                days.append([])
            else:
                days.append(rng.sample("MAE", rng.randint(1, 3)))
        team[f"Emp{n:02d}"] = days
    return team


def write_team(team, path):
    with open(path, "w") as f:
        for name, days in team.items():
            f.write(name + " | " + " | ".join(" ".join(d) or "-" for d in days) + "\n")


def parse_table(output):
    """Return {day: [[(name, mark), ...] per shift]} from the printed table."""
    table = {}
    for line in output.splitlines():
        head = line[:10].strip()
        if head in DAYS and "|" in line and head not in table:
            cells = [c.strip() for c in line.split("|")[1:]]
            cells += [""] * (len(SHIFTS) - len(cells))
            shifts = []
            for cell in cells:
                people = []
                for entry in filter(None, (p.strip() for p in cell.split(","))):
                    mark = entry[-1] if entry[-1] in "+*" else ""
                    people.append((entry.rstrip("+*"), mark))
                shifts.append(people)
            table[head] = shifts
    return table


def check(team, output):
    problems = []
    table = parse_table(output)
    if sorted(table) != sorted(DAYS):
        return ["schedule table missing or incomplete"]
    enough = len(team) * 5 >= 2 * len(SHIFTS) * len(DAYS)
    days_worked = {name: 0 for name in team}
    for d, day in enumerate(DAYS):
        seen = set()
        for s, people in enumerate(table[day]):
            if len(people) > 3:
                problems.append(f"{day} {SHIFTS[s]} has {len(people)} people")
            if enough and len(people) < 2:
                problems.append(f"{day} {SHIFTS[s]} has only {len(people)} people")
            for name, mark in people:
                if name in seen:
                    problems.append(f"{name} twice on {day}")
                seen.add(name)
                days_worked[name] += 1
                ranking = team[name][d]
                if mark == "" and (not ranking or ranking[0] != SHIFTS[s][0]):
                    problems.append(f"{name} unmarked on {day} {SHIFTS[s]} but ranking is {ranking}")
    for name, count in days_worked.items():
        if count > 5:
            problems.append(f"{name} works {count} days")
    if enough and "RULE CHECK: PASS" not in output:
        problems.append("program reported a rule failure for a staffable team")
    return problems


def main():
    trials = int(sys.argv[1]) if len(sys.argv) > 1 else 500
    failures = staffable = 0
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "team.txt")
        for trial in range(trials):
            team = random_team(random.Random(trial))
            write_team(team, path)
            args = ["--file", path, "--seed", str(trial)]
            py = subprocess.run(PYTHON_CMD + args, capture_output=True, text=True)
            rs = subprocess.run(RUST_CMD + args, capture_output=True, text=True)
            problems = []
            if py.stdout != rs.stdout or py.returncode != rs.returncode:
                problems.append("Python and Rust outputs differ")
            problems += check(team, py.stdout)
            staffable += len(team) * 5 >= 42
            if problems:
                failures += 1
                print(f"trial {trial} ({len(team)} employees): " + "; ".join(problems[:5]))
    print(f"{trials} random teams ({staffable} large enough to staff every shift), "
          f"{failures} failures; Python and Rust outputs "
          f"{'identical in every trial' if failures == 0 else 'see above'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
