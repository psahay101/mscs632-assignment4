#!/usr/bin/env python3
"""Weekly employee shift scheduler, Python version (MSCS-632 Assignment 4).

The program collects each employee's ranked shift preferences for the seven
days of the week, builds a schedule that follows the company's rules, resolves
conflicts, and prints the final week.

Company rules enforced by the scheduler:
  - an employee works at most one shift per day;
  - an employee works at most five days per week;
  - every shift has at least two and at most three employees.

When an employee's preferred shift is full, the employee is offered their
next-ranked shift, then any other open shift that day, and finally a shift on
the next day. Shifts still below two people at the end of a day are topped up
at random from employees who are free that day and have not worked five days.

Usage:
  python3 scheduler.py                                # enter preferences at prompts
  python3 scheduler.py --file ../data/employees.txt   # read preferences from a file
  python3 scheduler.py --file ../data/employees.txt --seed 7
"""
import argparse
import sys

DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
SHIFTS = ["Morning", "Afternoon", "Evening"]
SHIFT_CODES = {"m": 0, "morning": 0, "a": 1, "afternoon": 1, "e": 2, "evening": 2}
ORDINALS = ["1st", "2nd", "3rd"]

MIN_PER_SHIFT = 2
MAX_PER_SHIFT = 3
MAX_DAYS = 5
DEFAULT_SEED = 632

MASK64 = (1 << 64) - 1


def employee_count(count):
    return f"{count} employee" + ("" if count == 1 else "s")


class SplitMix64:
    """Seeded pseudo-random generator. The Rust version implements the same
    algorithm, so a given seed makes the same random choices in both."""

    def __init__(self, seed):
        self.state = seed & MASK64

    def next_u64(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK64
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK64
        return z ^ (z >> 31)

    def below(self, n):
        """Random integer in the range [0, n)."""
        return self.next_u64() % n


class Employee:
    def __init__(self, name, prefs):
        self.name = name
        self.prefs = prefs                   # one ranked list of shifts per day
        self.shift_on = [None] * len(DAYS)   # shift worked each day, or None
        self.mark_on = [""] * len(DAYS)      # "", "+" or "*" (see Scheduler)
        self.days_worked = 0


# ------------------------------------------------------------------ input

def parse_ranking(text):
    """Turn 'M A', 'm>a' or 'morning, evening' into shift indices, best
    first. '-', 'off' or an empty string means no shift wanted that day."""
    text = text.strip().lower()
    if text in ("", "-", "off"):
        return []
    ranking = []
    for token in text.replace(">", " ").replace(",", " ").split():
        if token not in SHIFT_CODES:
            raise ValueError(f"unknown shift '{token}' (use M, A, or E)")
        shift = SHIFT_CODES[token]
        if shift in ranking:
            raise ValueError(f"shift '{token}' is listed twice")
        ranking.append(shift)
    return ranking


def read_file(path):
    """Read 'name | Mon | Tue | ... | Sun' lines; '#' starts a comment."""
    employees = []
    with open(path, encoding="utf-8") as f:
        for line_no, raw in enumerate(f, start=1):
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            fields = line.split("|")
            if len(fields) != 1 + len(DAYS):
                raise ValueError(f"{path}:{line_no}: expected a name and "
                                 f"{len(DAYS)} day columns separated by '|'")
            name = fields[0].strip()
            if not name:
                raise ValueError(f"{path}:{line_no}: missing employee name")
            if any(e.name.lower() == name.lower() for e in employees):
                raise ValueError(f"{path}:{line_no}: duplicate employee '{name}'")
            try:
                prefs = [parse_ranking(cell) for cell in fields[1:]]
            except ValueError as err:
                raise ValueError(f"{path}:{line_no}: {err}") from None
            employees.append(Employee(name, prefs))
    return employees


def ask(prompt):
    """input() that returns None at the end of input instead of raising."""
    try:
        return input(prompt).strip()
    except EOFError:
        print()
        return None


def read_interactive():
    """Prompt for each employee's name and a ranked preference per day."""
    print("Enter each employee's ranked shift preferences for the week.")
    print("Shifts: M = morning, A = afternoon, E = evening, best first (e.g. 'M A').")
    print("Type '-' for a day off, or press Enter to repeat the previous day.\n")
    employees = []
    while True:
        name = ask("Employee name (blank to finish): ")
        if not name:
            break
        if "|" in name or "#" in name:
            print("  Names cannot contain '|' or '#'.")
            continue
        if any(e.name.lower() == name.lower() for e in employees):
            print(f"  {name} has already been entered.")
            continue
        prefs = []
        previous = "-"
        day = 0
        while day < len(DAYS):
            answer = ask(f"  {DAYS[day]:<9} [{previous}]: ")
            if not answer:
                answer = previous
            try:
                prefs.append(parse_ranking(answer))
            except ValueError as err:
                print(f"  {err}; please try again.")
                continue
            previous = answer
            day += 1
        employees.append(Employee(name, prefs))
    return employees


# ------------------------------------------------------------- scheduling

class Scheduler:
    """Builds the week one day at a time.

    Each day is processed in three passes: employees carried over from a full
    previous day, then that day's requests (fewest days worked first), then
    random top-ups for any shift still below the minimum. Every grid entry is
    an (employee index, mark) pair, where the mark is "" for a first choice,
    "+" for a placement after a conflict and "*" for a random top-up.
    """

    def __init__(self, employees, seed):
        self.emps = employees
        self.rng = SplitMix64(seed)
        self.grid = [[[] for _ in SHIFTS] for _ in DAYS]
        self.log = []
        # Minimum staffing can be guaranteed only if the team has enough
        # working days in total; otherwise the schedule is best effort.
        self.staffing_possible = (len(employees) * MAX_DAYS
                                  >= MIN_PER_SHIFT * len(SHIFTS) * len(DAYS))

    def build(self):
        carried = []
        for day in range(len(DAYS)):
            # 1. Employees bumped from a full day yesterday get first claim.
            for e in carried:
                self.place_carried(e, day)
            # 2. Today's requests, fewest days worked first so extra shifts
            #    are spread evenly; ties keep the input order (stable sort).
            requests = [i for i, emp in enumerate(self.emps) if emp.prefs[day]]
            requests.sort(key=lambda i: self.emps[i].days_worked)
            carried = []
            for e in requests:
                if self.place_request(e, day) == "carry":
                    carried.append(e)
            # 3. Make sure every shift reaches the minimum.
            self.fill_minimums(day)

    def place_request(self, e, day):
        emp = self.emps[e]
        if emp.shift_on[day] is not None:
            return "placed"                 # already placed as a carry-over
        if emp.days_worked >= MAX_DAYS:
            self.note(day, f"{emp.name} asked for {DAYS[day]} but has already "
                           f"worked {MAX_DAYS} days")
            return "skipped"
        ranking = emp.prefs[day]
        first = ranking[0]
        held = False
        # The employee's own ranking, best first.
        for rank, shift in enumerate(ranking):
            if len(self.grid[day][shift]) >= MAX_PER_SHIFT:
                continue                    # full: a conflict, try the next choice
            if not self.keeps_week_staffable(e, day, shift):
                held = True
                continue
            if rank == 0:
                self.assign(e, day, shift, "")
            else:
                self.assign(e, day, shift, "+")
                self.note(day, f"{self.why_moved(day, first)}: {emp.name} placed "
                               f"in {SHIFTS[shift]} ({ORDINALS[rank]} choice)")
            return "placed"
        # Any other shift with room on the same day.
        for shift in range(len(SHIFTS)):
            if shift in ranking or len(self.grid[day][shift]) >= MAX_PER_SHIFT:
                continue
            if not self.keeps_week_staffable(e, day, shift):
                held = True
                continue
            self.assign(e, day, shift, "+")
            self.note(day, f"{self.why_moved(day, first)}: {emp.name} moved "
                           f"to {SHIFTS[shift]}")
            return "placed"
        if held:
            self.note(day, f"{emp.name} not scheduled: their remaining days are "
                           f"needed to staff later shifts")
            return "held"
        # The whole day is full, so the request moves to the next day.
        if day + 1 == len(DAYS):
            self.note(day, f"all shifts are full and the week is over: "
                           f"{emp.name} could not be placed")
        elif emp.prefs[day + 1]:
            self.note(day, f"all shifts are full and {emp.name} already asked for "
                           f"{DAYS[day + 1]}: request dropped")
        else:
            self.note(day, f"all shifts are full: {emp.name} carried over to "
                           f"{DAYS[day + 1]}")
            return "carry"
        return "unplaced"

    def place_carried(self, e, day):
        """Place an employee bumped from yesterday, trying yesterday's
        ranking first and then the remaining shifts."""
        emp = self.emps[e]
        ranking = emp.prefs[day - 1]
        order = ranking + [s for s in range(len(SHIFTS)) if s not in ranking]
        for shift in order:
            if (len(self.grid[day][shift]) < MAX_PER_SHIFT
                    and self.keeps_week_staffable(e, day, shift)):
                self.assign(e, day, shift, "+")
                self.note(day, f"{emp.name} carried over from {DAYS[day - 1]}: "
                               f"placed in {SHIFTS[shift]}")
                break
        else:
            self.note(day, f"{emp.name} carried over from {DAYS[day - 1]} but "
                           f"could not be placed")

    def fill_minimums(self, day):
        """Top up shifts below the minimum with random employees who are free
        today and have not worked five days, skipping anyone whose remaining
        days are needed to staff the rest of the week."""
        for shift in range(len(SHIFTS)):
            while len(self.grid[day][shift]) < MIN_PER_SHIFT:
                free = [i for i, emp in enumerate(self.emps)
                        if emp.shift_on[day] is None and emp.days_worked < MAX_DAYS]
                if not free:
                    self.note(day, f"{SHIFTS[shift]} is understaffed: nobody free "
                                   f"has days left")
                    break
                safe = [i for i in free if self.keeps_week_staffable(i, day, shift)]
                pool = safe or free
                pick = pool[self.rng.below(len(pool))]
                self.assign(pick, day, shift, "*")
                self.note(day, f"{SHIFTS[shift]} below minimum: "
                               f"{self.emps[pick].name} added at random")

    def keeps_week_staffable(self, e, day, shift):
        """Return True if every remaining shift of the week can still reach
        MIN_PER_SHIFT after employee e takes this shift.

        Today's open minimum slots must be coverable by employees still free
        today. Each later day needs MIN_PER_SHIFT people on every shift, and
        an employee can give at most one shift per remaining day and at most
        the days left under the weekly cap, so the later days can be staffed
        exactly when those per-employee limits add up to the total need.
        """
        if not self.staffing_possible:
            return True
        counts = [len(people) for people in self.grid[day]]
        counts[shift] += 1
        open_today = sum(max(0, MIN_PER_SHIFT - c) for c in counts)
        days_after = len(DAYS) - day - 1
        capacity = 0
        free_today = []
        for i, emp in enumerate(self.emps):
            days_left = MAX_DAYS - emp.days_worked - (1 if i == e else 0)
            if i != e and emp.shift_on[day] is None and days_left > 0:
                free_today.append(days_left)
            else:
                capacity += min(days_left, days_after)
        if len(free_today) < open_today:
            return False
        # Today's open slots go to the free employees with the most days
        # left, which costs the rest of the week the fewest working days.
        free_today.sort(reverse=True)
        for k, days_left in enumerate(free_today):
            if k < open_today:
                days_left -= 1
            capacity += min(days_left, days_after)
        return capacity >= MIN_PER_SHIFT * len(SHIFTS) * days_after

    def why_moved(self, day, first):
        if len(self.grid[day][first]) >= MAX_PER_SHIFT:
            return f"{SHIFTS[first]} is full"
        return f"{SHIFTS[first]} would leave later shifts short"

    def assign(self, e, day, shift, mark):
        emp = self.emps[e]
        emp.shift_on[day] = shift
        emp.mark_on[day] = mark
        emp.days_worked += 1
        self.grid[day][shift].append((e, mark))

    def note(self, day, message):
        self.log.append((day, message))

    # ------------------------------------------------------------- output

    def report(self, seed):
        print(f"WEEKLY SHIFT SCHEDULE ({employee_count(len(self.emps))}, random seed {seed})")
        print(f"Rules: one shift per day, at most {MAX_DAYS} days each, "
              f"{MIN_PER_SHIFT}-{MAX_PER_SHIFT} employees per shift")
        if not self.staffing_possible:
            print(f"Warning: {employee_count(len(self.emps))} can work at most "
                  f"{len(self.emps) * MAX_DAYS} shifts, but the week needs "
                  f"{MIN_PER_SHIFT * len(SHIFTS) * len(DAYS)}; some shifts will be short")
        print()
        self.print_table()
        self.print_employees()
        self.print_log()
        return self.print_check()

    def print_table(self):
        cells = [[", ".join(self.emps[e].name + mark for e, mark in self.grid[d][s])
                  for s in range(len(SHIFTS))] for d in range(len(DAYS))]
        width = max([len(s) for s in SHIFTS] + [len(c) for row in cells for c in row])

        def row(first, values):
            return (f"{first:<10}| " + " | ".join(f"{v:<{width}}" for v in values)).rstrip()

        print(row("Day", SHIFTS))
        print("-" * 10 + "+" + "+".join("-" * (width + 2) for _ in SHIFTS))
        for d in range(len(DAYS)):
            print(row(DAYS[d], cells[d]))
        print(f"  + placed after a conflict    * added at random to reach the "
              f"{MIN_PER_SHIFT}-person minimum")
        print()

    def print_employees(self):
        name_w = max([len("Employee")] + [len(emp.name) for emp in self.emps])
        print(f"{'Employee':<{name_w}}  " + "".join(f"{d[:3]:<5}" for d in DAYS)
              + "Days  1st choice")
        for emp in self.emps:
            cells = []
            for d in range(len(DAYS)):
                if emp.shift_on[d] is None:
                    cells.append("-")
                else:
                    cells.append(SHIFTS[emp.shift_on[d]][0] + emp.mark_on[d])
            requested = sum(1 for ranking in emp.prefs if ranking)
            first = sum(1 for d in range(len(DAYS))
                        if emp.shift_on[d] is not None and emp.mark_on[d] == "")
            print(f"{emp.name:<{name_w}}  " + "".join(f"{c:<5}" for c in cells)
                  + f"{emp.days_worked:<6}{first}/{requested}")
        total = sum(emp.days_worked for emp in self.emps)
        marks = [emp.mark_on[d] for emp in self.emps for d in range(len(DAYS))
                 if emp.shift_on[d] is not None]
        print(f"{total} shifts filled: {marks.count('')} first choices, "
              f"{marks.count('+')} placed after conflicts, {marks.count('*')} added at random")
        print()

    def print_log(self):
        print("CONFLICTS AND ADJUSTMENTS")
        if not self.log:
            print("  none")
        for day, message in self.log:
            print(f"{DAYS[day]:<10}{message}")
        print()

    def check_rules(self):
        """Re-check the finished grid against every rule, independently of
        the bookkeeping used while building it."""
        problems = []
        shifts_per_employee = [0] * len(self.emps)
        for d, day in enumerate(DAYS):
            seen = set()
            for s, shift in enumerate(SHIFTS):
                people = [e for e, _ in self.grid[d][s]]
                if not MIN_PER_SHIFT <= len(people) <= MAX_PER_SHIFT:
                    problems.append(f"{day} {shift} has {len(people)} employees")
                for e in people:
                    if e in seen:
                        problems.append(f"{self.emps[e].name} works twice on {day}")
                    seen.add(e)
                    shifts_per_employee[e] += 1
        for e, count in enumerate(shifts_per_employee):
            if count > MAX_DAYS:
                problems.append(f"{self.emps[e].name} works {count} days")
        return problems

    def print_check(self):
        problems = self.check_rules()
        if not problems:
            print(f"RULE CHECK: PASS (no double shifts, nobody over {MAX_DAYS} days, "
                  f"{MIN_PER_SHIFT}-{MAX_PER_SHIFT} employees on every shift)")
            return True
        print("RULE CHECK: FAIL")
        for problem in problems:
            print(f"  {problem}")
        return False


def main():
    parser = argparse.ArgumentParser(description="Weekly employee shift scheduler")
    parser.add_argument("--file", help="read preferences from this file instead of prompting")
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED,
                        help=f"seed for the random staffing choices (default {DEFAULT_SEED})")
    args = parser.parse_args()
    if args.seed < 0:
        parser.error("--seed must be zero or positive")
    try:
        team = read_file(args.file) if args.file else read_interactive()
    except (OSError, ValueError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    if not team:
        print("error: no employees were entered", file=sys.stderr)
        return 1
    scheduler = Scheduler(team, args.seed)
    scheduler.build()
    return 0 if scheduler.report(args.seed) else 2


if __name__ == "__main__":
    sys.exit(main())
