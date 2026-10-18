# MSCS-632 Assignment 4 — Implementing Control Structures

A weekly employee shift scheduler implemented twice, in **Python** and in
**Rust**. Python is dynamically typed and interpreted, while Rust is
statically typed, compiled, and checks ownership and exhaustiveness at compile
time. That contrast is the reason this pair was chosen.

Both programs read the same input, apply the same rules, and use the same
seeded random-number generator, so for a given input and seed they print
byte-for-byte identical schedules. The test in `tests/` checks that on
randomly generated teams.

Tested on Ubuntu 22.04 with Python 3.10.18 and Rust 1.98.1. Neither version
has third-party dependencies.

## Repository layout

| Path | Contents |
|---|---|
| `python/scheduler.py` | Python version (standard library only) |
| `rust/src/main.rs`, `rust/Cargo.toml` | Rust version (standard library only) |
| `data/employees.txt` | Sample week: 12 employees with ranked preferences |
| `tests/fuzz_test.py` | Randomized cross-language test |
| `outputs/` | Captured output of both programs and of the test |
| `screenshots/` | Terminal screenshots of the runs |

## Running

```bash
# Python
cd python
python3 scheduler.py --file ../data/employees.txt            # read preferences from a file
python3 scheduler.py                                         # enter preferences at prompts

# Rust
cd rust
cargo run --release -- --file ../data/employees.txt
cargo run --release                                          # enter preferences at prompts
```

`--seed N` changes the random choices used to top up short-staffed shifts
(default 632).

### Input format

One employee per line: a name and seven day columns separated by `|`. Each
day lists that day's shifts best first, using `M` (morning), `A` (afternoon),
and `E` (evening). `-` means the employee would like the day off.

```
# Name  | Mon | Tue | Wed | Thu | Fri   | Sat | Sun
Alice   | M A | M A | M   | -   | M     | -   | -
Lena    | -   | E   | E   | -   | M A E | -   | E
```

In interactive mode the program asks for each name and then each day's
ranking. Pressing Enter repeats the previous day's answer, and an invalid
answer is rejected and asked again.

## How the schedule is built

The week is built one day at a time, Monday to Sunday. Each day takes three
passes.

1. **Carried-over employees.** Anyone whose requested day was completely
   full the day before gets first claim on today, trying their original
   ranking first.
2. **Requests.** Today's requests are handled with the fewest-days-worked
   employees first, so extra shifts are spread evenly. Each employee gets
   the best-ranked shift with room. If their preferred shift is full, that is
   a **conflict**: they get their next choice, then any other open shift that
   day, and if the whole day is full they are carried over to the next day.
   This ranking is the optional bonus feature.
3. **Minimum staffing.** Any shift still below two people is topped up at
   random from employees who are free that day and have not worked five
   days.

The rules always hold: one shift per employee per day, at most five days per
employee, and two to three employees per shift.

A greedy day-by-day scheduler can use up the team's five-day allowances early
in the week and leave the weekend understaffed. To prevent that, every
assignment is checked first by `keeps_week_staffable`. An employee can work at
most one shift per remaining day and at most their remaining days under the
cap. The rest of the week can be staffed exactly when those per-employee
limits add up to the minimum the remaining shifts need. An assignment that
would break that is skipped and logged. As a result, any team with at least
nine employees (9 × 5 ≥ 42 required shifts) always gets a fully staffed week.
Smaller teams get a best-effort schedule with a warning.

The output has four parts:

1. The week as a day-by-shift table, with `+` marking placements after a
   conflict and `*` marking random top-ups.
2. A per-employee view showing how many first choices each person got.
3. A log of every conflict and adjustment.
4. A final rule check that re-verifies the finished grid independently of
   the bookkeeping used to build it.

## Control structures in each language

| Construct | Python (`python/scheduler.py`) | Rust (`rust/src/main.rs`) |
|---|---|---|
| Input loop until a blank name | `while True` with `break`/`continue` in `read_interactive` | `loop` with a `match` on `Option<String>` and a guard (`Some(name) if !name.is_empty()`) |
| Re-asking after an invalid answer | `while day < len(DAYS)` plus `try`/`except ValueError` and `continue` | `while day < DAYS.len()` plus `match` on `Result` |
| End of input | `except EOFError` | `Ok(0)` arm of a `match` on `read_line` |
| Trying ranked shifts in order | `for rank, shift in enumerate(...)` with `continue` and early `return` | `for (rank, &shift) in ranking.iter().enumerate()` with `continue` and early `return` |
| "No shift found" fallback | `for ... else` in `place_carried` | `Iterator::find` then `match` on `Some`/`None` |
| Next-day carry decision | `if` / `elif` / `else` chain | `match DAYS.get(day + 1)` with `None`, a guarded `Some`, and `Some` arms, checked for exhaustiveness by the compiler |
| Request outcome | returned strings compared with `==` | `enum Outcome` tested with `if let Outcome::CarryOver = ...` |
| Topping up a shift | `while` loop with `break` when nobody is free | `while` loop with `break` when nobody is free |
| Choosing a fallback list | `pool = safe or free` | `let pool = if safe.is_empty() { free } else { safe };` |
| Command-line parsing | `argparse` | `while let Some(arg) = args.next()` with `match` and the `?` operator |
| Error propagation | exceptions raised and caught in `main` | `Result` values, `?`, and `match` in `main` |

## Testing

`tests/fuzz_test.py` generates random teams of 5 to 18 employees with
random ranked preferences. It runs both programs on each team with the same
seed, then checks the following:

- the two outputs are identical;
- the printed table, parsed by the test itself, never has an employee twice
  on one day, never has anyone over five days, and never has more than three
  people on a shift;
- every shift has at least two people whenever the team can cover the week;
- every unmarked name sits on that employee's first choice.

```bash
(cd rust && cargo build --release)
python3 tests/fuzz_test.py 1000
```

Captured result: [`outputs/fuzz_test.txt`](outputs/fuzz_test.txt). As a
check that the test can catch real defects, disabling the staffing lookahead
makes 50 of 300 random teams fail it.

## Sample output

Full captured output: [`outputs/python_output.txt`](outputs/python_output.txt)
and [`outputs/rust_output.txt`](outputs/rust_output.txt), which are identical.

```
Day       | Morning            | Afternoon          | Evening
----------+--------------------+--------------------+--------------------
Monday    | Alice, Bruno, Chen | Dana+, Eli+, Farah | Gus, Hana+, Jade
Tuesday   | Ivan, Alice, Dana  | Kofi, Bruno, Farah | Lena, Eli, Hana
Wednesday | Gus, Ivan, Alice   | Jade, Bruno, Hana  | Chen, Lena, Eli
Thursday  | Kofi, Dana, Gus    | Farah, Jade, Bruno | Chen, Ivan
Friday    | Lena, Alice, Dana  | Kofi, Farah, Hana  | Chen, Eli, Gus
Saturday  | Ivan+, Kofi, Dana  | Bruno+, Jade, Eli  | Chen, Gus, Hana+
Sunday    | Farah, Ivan        | Kofi, Alice*       | Lena, Jade*
```
