//! Weekly employee shift scheduler, Rust version (MSCS-632 Assignment 4).
//!
//! The program collects each employee's ranked shift preferences for the
//! seven days of the week, builds a schedule that follows the company's
//! rules, resolves conflicts, and prints the final week.
//!
//! Company rules enforced by the scheduler:
//!   - an employee works at most one shift per day;
//!   - an employee works at most five days per week;
//!   - every shift has at least two and at most three employees.
//!
//! When an employee's preferred shift is full, the employee is offered their
//! next-ranked shift, then any other open shift that day, and finally a shift
//! on the next day. Shifts still below two people at the end of a day are
//! topped up at random from employees who are free that day and have not
//! worked five days.
//!
//! Usage:
//!   cargo run --release                                   # enter preferences at prompts
//!   cargo run --release -- --file ../data/employees.txt   # read preferences from a file
//!   cargo run --release -- --file ../data/employees.txt --seed 7

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

const DAYS: [&str; 7] = [
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
];
const SHIFTS: [&str; 3] = ["Morning", "Afternoon", "Evening"];
const ORDINALS: [&str; 3] = ["1st", "2nd", "3rd"];

const MIN_PER_SHIFT: usize = 2;
const MAX_PER_SHIFT: usize = 3;
const MAX_DAYS: usize = 5;
const DEFAULT_SEED: u64 = 632;

/// Seeded pseudo-random generator (SplitMix64). The Python version implements
/// the same algorithm, so a given seed makes the same random choices in both.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Random integer in the range [0, n).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn employee_count(count: usize) -> String {
    format!("{} employee{}", count, if count == 1 { "" } else { "s" })
}

/// How an employee came to be on a shift, shown as a mark after the name.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    FirstChoice,
    AfterConflict,
    Random,
}

impl Mark {
    fn symbol(self) -> &'static str {
        match self {
            Mark::FirstChoice => "",
            Mark::AfterConflict => "+",
            Mark::Random => "*",
        }
    }
}

/// What happened to one employee's request for one day.
enum Outcome {
    Placed,
    CarryOver,
    NotPlaced,
}

struct Employee {
    name: String,
    prefs: Vec<Vec<usize>>,                     // one ranked list of shifts per day
    shift_on: [Option<(usize, Mark)>; DAYS.len()], // shift worked each day, if any
    days_worked: usize,
}

impl Employee {
    fn new(name: &str, prefs: Vec<Vec<usize>>) -> Self {
        Employee { name: name.to_string(), prefs, shift_on: [None; DAYS.len()], days_worked: 0 }
    }
}

// ------------------------------------------------------------------- input

/// Turn "M A", "m>a" or "morning, evening" into shift indices, best first.
/// "-", "off" or an empty string means no shift wanted that day.
fn parse_ranking(text: &str) -> Result<Vec<usize>, String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() || text == "-" || text == "off" {
        return Ok(Vec::new());
    }
    let mut ranking = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == '>') {
        if token.is_empty() {
            continue;
        }
        let shift = match token {
            "m" | "morning" => 0,
            "a" | "afternoon" => 1,
            "e" | "evening" => 2,
            _ => return Err(format!("unknown shift '{}' (use M, A, or E)", token)),
        };
        if ranking.contains(&shift) {
            return Err(format!("shift '{}' is listed twice", token));
        }
        ranking.push(shift);
    }
    Ok(ranking)
}

/// Read "name | Mon | Tue | ... | Sun" lines; '#' starts a comment.
fn read_file(path: &str) -> Result<Vec<Employee>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {}", path, err))?;
    let mut employees: Vec<Employee> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() != 1 + DAYS.len() {
            return Err(format!(
                "{}:{}: expected a name and {} day columns separated by '|'",
                path, line_no, DAYS.len()
            ));
        }
        let name = fields[0].trim();
        if name.is_empty() {
            return Err(format!("{}:{}: missing employee name", path, line_no));
        }
        if employees.iter().any(|e| e.name.to_lowercase() == name.to_lowercase()) {
            return Err(format!("{}:{}: duplicate employee '{}'", path, line_no, name));
        }
        let mut prefs = Vec::with_capacity(DAYS.len());
        for cell in &fields[1..] {
            match parse_ranking(cell) {
                Ok(ranking) => prefs.push(ranking),
                Err(err) => return Err(format!("{}:{}: {}", path, line_no, err)),
            }
        }
        employees.push(Employee::new(name, prefs));
    }
    Ok(employees)
}

/// Print a prompt and read one trimmed line; None at the end of input.
fn ask(prompt: &str) -> Option<String> {
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => {
            println!();
            None
        }
        Ok(_) => Some(line.trim().to_string()),
    }
}

/// Prompt for each employee's name and a ranked preference per day.
fn read_interactive() -> Vec<Employee> {
    println!("Enter each employee's ranked shift preferences for the week.");
    println!("Shifts: M = morning, A = afternoon, E = evening, best first (e.g. 'M A').");
    println!("Type '-' for a day off, or press Enter to repeat the previous day.\n");
    let mut employees: Vec<Employee> = Vec::new();
    loop {
        let name = match ask("Employee name (blank to finish): ") {
            Some(name) if !name.is_empty() => name,
            _ => break,
        };
        if name.contains('|') || name.contains('#') {
            println!("  Names cannot contain '|' or '#'.");
            continue;
        }
        if employees.iter().any(|e| e.name.to_lowercase() == name.to_lowercase()) {
            println!("  {} has already been entered.", name);
            continue;
        }
        let mut prefs = Vec::with_capacity(DAYS.len());
        let mut previous = String::from("-");
        let mut day = 0;
        while day < DAYS.len() {
            let answer = match ask(&format!("  {:<9} [{}]: ", DAYS[day], previous)) {
                Some(text) if !text.is_empty() => text,
                _ => previous.clone(),
            };
            match parse_ranking(&answer) {
                Ok(ranking) => {
                    prefs.push(ranking);
                    previous = answer;
                    day += 1;
                }
                Err(err) => println!("  {}; please try again.", err),
            }
        }
        employees.push(Employee::new(&name, prefs));
    }
    employees
}

// -------------------------------------------------------------- scheduling

/// Builds the week one day at a time.
///
/// Each day is processed in three passes: employees carried over from a full
/// previous day, then that day's requests (fewest days worked first), then
/// random top-ups for any shift still below the minimum.
struct Scheduler {
    emps: Vec<Employee>,
    rng: SplitMix64,
    grid: [[Vec<usize>; SHIFTS.len()]; DAYS.len()], // employee indices per shift
    log: Vec<(usize, String)>,
    staffing_possible: bool,
}

impl Scheduler {
    fn new(emps: Vec<Employee>, seed: u64) -> Self {
        // Minimum staffing can be guaranteed only if the team has enough
        // working days in total; otherwise the schedule is best effort.
        let staffing_possible =
            emps.len() * MAX_DAYS >= MIN_PER_SHIFT * SHIFTS.len() * DAYS.len();
        Scheduler {
            emps,
            rng: SplitMix64::new(seed),
            grid: Default::default(),
            log: Vec::new(),
            staffing_possible,
        }
    }

    fn build(&mut self) {
        let mut carried: Vec<usize> = Vec::new();
        for day in 0..DAYS.len() {
            // 1. Employees bumped from a full day yesterday get first claim.
            for &e in &carried {
                self.place_carried(e, day);
            }
            // 2. Today's requests, fewest days worked first so extra shifts
            //    are spread evenly; ties keep the input order (stable sort).
            let mut requests: Vec<usize> =
                (0..self.emps.len()).filter(|&i| !self.emps[i].prefs[day].is_empty()).collect();
            requests.sort_by_key(|&i| self.emps[i].days_worked);
            carried.clear();
            for e in requests {
                if let Outcome::CarryOver = self.place_request(e, day) {
                    carried.push(e);
                }
            }
            // 3. Make sure every shift reaches the minimum.
            self.fill_minimums(day);
        }
    }

    fn place_request(&mut self, e: usize, day: usize) -> Outcome {
        if self.emps[e].shift_on[day].is_some() {
            return Outcome::Placed; // already placed as a carry-over
        }
        let name = self.emps[e].name.clone();
        if self.emps[e].days_worked >= MAX_DAYS {
            self.note(day, format!("{} asked for {} but has already worked {} days",
                                   name, DAYS[day], MAX_DAYS));
            return Outcome::NotPlaced;
        }
        let ranking = self.emps[e].prefs[day].clone();
        let first = ranking[0];
        let mut held = false;
        // The employee's own ranking, best first.
        for (rank, &shift) in ranking.iter().enumerate() {
            if self.grid[day][shift].len() >= MAX_PER_SHIFT {
                continue; // full: a conflict, try the next choice
            }
            if !self.keeps_week_staffable(e, day, shift) {
                held = true;
                continue;
            }
            if rank == 0 {
                self.assign(e, day, shift, Mark::FirstChoice);
            } else {
                self.assign(e, day, shift, Mark::AfterConflict);
                let why = self.why_moved(day, first);
                self.note(day, format!("{}: {} placed in {} ({} choice)",
                                       why, name, SHIFTS[shift], ORDINALS[rank]));
            }
            return Outcome::Placed;
        }
        // Any other shift with room on the same day.
        for (shift, shift_name) in SHIFTS.iter().enumerate() {
            if ranking.contains(&shift) || self.grid[day][shift].len() >= MAX_PER_SHIFT {
                continue;
            }
            if !self.keeps_week_staffable(e, day, shift) {
                held = true;
                continue;
            }
            self.assign(e, day, shift, Mark::AfterConflict);
            let why = self.why_moved(day, first);
            self.note(day, format!("{}: {} moved to {}", why, name, shift_name));
            return Outcome::Placed;
        }
        if held {
            self.note(day, format!("{} not scheduled: their remaining days are needed to \
                                    staff later shifts", name));
            return Outcome::NotPlaced;
        }
        // The whole day is full, so the request moves to the next day.
        match DAYS.get(day + 1) {
            None => self.note(day, format!("all shifts are full and the week is over: \
                                            {} could not be placed", name)),
            Some(next) if !self.emps[e].prefs[day + 1].is_empty() => self.note(
                day,
                format!("all shifts are full and {} already asked for {}: request dropped",
                        name, next),
            ),
            Some(next) => {
                self.note(day, format!("all shifts are full: {} carried over to {}", name, next));
                return Outcome::CarryOver;
            }
        }
        Outcome::NotPlaced
    }

    /// Place an employee bumped from yesterday, trying yesterday's ranking
    /// first and then the remaining shifts.
    fn place_carried(&mut self, e: usize, day: usize) {
        let ranking = &self.emps[e].prefs[day - 1];
        let order: Vec<usize> = ranking
            .iter()
            .copied()
            .chain((0..SHIFTS.len()).filter(|s| !ranking.contains(s)))
            .collect();
        let name = self.emps[e].name.clone();
        let chosen = order.into_iter().find(|&shift| {
            self.grid[day][shift].len() < MAX_PER_SHIFT && self.keeps_week_staffable(e, day, shift)
        });
        match chosen {
            Some(shift) => {
                self.assign(e, day, shift, Mark::AfterConflict);
                self.note(day, format!("{} carried over from {}: placed in {}",
                                       name, DAYS[day - 1], SHIFTS[shift]));
            }
            None => self.note(day, format!("{} carried over from {} but could not be placed",
                                           name, DAYS[day - 1])),
        }
    }

    /// Top up shifts below the minimum with random employees who are free
    /// today and have not worked five days, skipping anyone whose remaining
    /// days are needed to staff the rest of the week.
    fn fill_minimums(&mut self, day: usize) {
        for (shift, shift_name) in SHIFTS.iter().enumerate() {
            while self.grid[day][shift].len() < MIN_PER_SHIFT {
                let free: Vec<usize> = (0..self.emps.len())
                    .filter(|&i| {
                        self.emps[i].shift_on[day].is_none() && self.emps[i].days_worked < MAX_DAYS
                    })
                    .collect();
                if free.is_empty() {
                    self.note(day, format!("{} is understaffed: nobody free has days left",
                                           shift_name));
                    break;
                }
                let safe: Vec<usize> = free
                    .iter()
                    .copied()
                    .filter(|&i| self.keeps_week_staffable(i, day, shift))
                    .collect();
                let pool = if safe.is_empty() { free } else { safe };
                let pick = pool[self.rng.below(pool.len())];
                self.assign(pick, day, shift, Mark::Random);
                let name = self.emps[pick].name.clone();
                self.note(day, format!("{} below minimum: {} added at random", shift_name, name));
            }
        }
    }

    /// Return true if every remaining shift of the week can still reach
    /// MIN_PER_SHIFT after employee `e` takes this shift.
    ///
    /// Today's open minimum slots must be coverable by employees still free
    /// today. Each later day needs MIN_PER_SHIFT people on every shift, and an
    /// employee can give at most one shift per remaining day and at most the
    /// days left under the weekly cap, so the later days can be staffed
    /// exactly when those per-employee limits add up to the total need.
    fn keeps_week_staffable(&self, e: usize, day: usize, shift: usize) -> bool {
        if !self.staffing_possible {
            return true;
        }
        let mut counts: Vec<usize> = self.grid[day].iter().map(Vec::len).collect();
        counts[shift] += 1;
        let open_today: usize = counts.iter().map(|&c| MIN_PER_SHIFT.saturating_sub(c)).sum();
        let days_after = DAYS.len() - day - 1;
        let mut capacity = 0;
        let mut free_today = Vec::new();
        for (i, emp) in self.emps.iter().enumerate() {
            let days_left = MAX_DAYS.saturating_sub(emp.days_worked + usize::from(i == e));
            if i != e && emp.shift_on[day].is_none() && days_left > 0 {
                free_today.push(days_left);
            } else {
                capacity += days_left.min(days_after);
            }
        }
        if free_today.len() < open_today {
            return false;
        }
        // Today's open slots go to the free employees with the most days
        // left, which costs the rest of the week the fewest working days.
        free_today.sort_unstable_by(|a, b| b.cmp(a));
        for (k, &days_left) in free_today.iter().enumerate() {
            let left = if k < open_today { days_left - 1 } else { days_left };
            capacity += left.min(days_after);
        }
        capacity >= MIN_PER_SHIFT * SHIFTS.len() * days_after
    }

    fn why_moved(&self, day: usize, first: usize) -> String {
        if self.grid[day][first].len() >= MAX_PER_SHIFT {
            format!("{} is full", SHIFTS[first])
        } else {
            format!("{} would leave later shifts short", SHIFTS[first])
        }
    }

    fn assign(&mut self, e: usize, day: usize, shift: usize, mark: Mark) {
        let emp = &mut self.emps[e];
        emp.shift_on[day] = Some((shift, mark));
        emp.days_worked += 1;
        self.grid[day][shift].push(e);
    }

    fn note(&mut self, day: usize, message: String) {
        self.log.push((day, message));
    }

    // ------------------------------------------------------------- output

    fn report(&self, seed: u64) -> bool {
        println!("WEEKLY SHIFT SCHEDULE ({}, random seed {})", employee_count(self.emps.len()), seed);
        println!("Rules: one shift per day, at most {} days each, {}-{} employees per shift",
                 MAX_DAYS, MIN_PER_SHIFT, MAX_PER_SHIFT);
        if !self.staffing_possible {
            println!("Warning: {} can work at most {} shifts, but the week needs {}; \
                      some shifts will be short",
                     employee_count(self.emps.len()), self.emps.len() * MAX_DAYS,
                     MIN_PER_SHIFT * SHIFTS.len() * DAYS.len());
        }
        println!();
        self.print_table();
        self.print_employees();
        self.print_log();
        self.print_check()
    }

    fn print_table(&self) {
        let cells: Vec<Vec<String>> = (0..DAYS.len())
            .map(|d| {
                (0..SHIFTS.len())
                    .map(|s| {
                        self.grid[d][s]
                            .iter()
                            .map(|&e| {
                                let mark = self.emps[e].shift_on[d].map_or("", |(_, m)| m.symbol());
                                format!("{}{}", self.emps[e].name, mark)
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .collect()
            })
            .collect();
        let width = SHIFTS
            .iter()
            .map(|s| s.chars().count())
            .chain(cells.iter().flatten().map(|c| c.chars().count()))
            .max()
            .unwrap_or(0);
        let row = |first: &str, values: &[&str]| -> String {
            let joined: Vec<String> = values.iter().map(|v| format!("{:<w$}", v, w = width)).collect();
            format!("{:<10}| {}", first, joined.join(" | ")).trim_end().to_string()
        };
        println!("{}", row("Day", &SHIFTS));
        let dashes: Vec<String> = SHIFTS.iter().map(|_| "-".repeat(width + 2)).collect();
        println!("{}+{}", "-".repeat(10), dashes.join("+"));
        for (d, day) in DAYS.iter().enumerate() {
            let values: Vec<&str> = cells[d].iter().map(String::as_str).collect();
            println!("{}", row(day, &values));
        }
        println!("  + placed after a conflict    * added at random to reach the {}-person minimum",
                 MIN_PER_SHIFT);
        println!();
    }

    fn print_employees(&self) {
        let name_w = self
            .emps
            .iter()
            .map(|e| e.name.chars().count())
            .chain(std::iter::once("Employee".len()))
            .max()
            .unwrap_or(0);
        let day_heads: String = DAYS.iter().map(|d| format!("{:<5}", &d[..3])).collect();
        println!("{:<w$}  {}Days  1st choice", "Employee", day_heads, w = name_w);
        let (mut first_total, mut moved_total, mut random_total) = (0, 0, 0);
        for emp in &self.emps {
            let mut cells = String::new();
            let mut first = 0;
            for slot in emp.shift_on.iter() {
                let cell = match slot {
                    None => String::from("-"),
                    Some((shift, mark)) => {
                        match mark {
                            Mark::FirstChoice => first += 1,
                            Mark::AfterConflict => moved_total += 1,
                            Mark::Random => random_total += 1,
                        }
                        format!("{}{}", &SHIFTS[*shift][..1], mark.symbol())
                    }
                };
                cells.push_str(&format!("{:<5}", cell));
            }
            first_total += first;
            let requested = emp.prefs.iter().filter(|r| !r.is_empty()).count();
            println!("{:<w$}  {}{:<6}{}/{}", emp.name, cells, emp.days_worked, first, requested,
                     w = name_w);
        }
        let total: usize = self.emps.iter().map(|e| e.days_worked).sum();
        println!("{} shifts filled: {} first choices, {} placed after conflicts, {} added at random",
                 total, first_total, moved_total, random_total);
        println!();
    }

    fn print_log(&self) {
        println!("CONFLICTS AND ADJUSTMENTS");
        if self.log.is_empty() {
            println!("  none");
        }
        for (day, message) in &self.log {
            println!("{:<10}{}", DAYS[*day], message);
        }
        println!();
    }

    /// Re-check the finished grid against every rule, independently of the
    /// bookkeeping used while building it.
    fn check_rules(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut shifts_per_employee = vec![0; self.emps.len()];
        for (d, day) in DAYS.iter().enumerate() {
            let mut seen = vec![false; self.emps.len()];
            for (s, shift) in SHIFTS.iter().enumerate() {
                let people = &self.grid[d][s];
                if !(MIN_PER_SHIFT..=MAX_PER_SHIFT).contains(&people.len()) {
                    problems.push(format!("{} {} has {} employees", day, shift, people.len()));
                }
                for &e in people {
                    if seen[e] {
                        problems.push(format!("{} works twice on {}", self.emps[e].name, day));
                    }
                    seen[e] = true;
                    shifts_per_employee[e] += 1;
                }
            }
        }
        for (e, &count) in shifts_per_employee.iter().enumerate() {
            if count > MAX_DAYS {
                problems.push(format!("{} works {} days", self.emps[e].name, count));
            }
        }
        problems
    }

    fn print_check(&self) -> bool {
        let problems = self.check_rules();
        if problems.is_empty() {
            println!("RULE CHECK: PASS (no double shifts, nobody over {} days, {}-{} employees on \
                      every shift)", MAX_DAYS, MIN_PER_SHIFT, MAX_PER_SHIFT);
            return true;
        }
        println!("RULE CHECK: FAIL");
        for problem in &problems {
            println!("  {}", problem);
        }
        false
    }
}

fn usage() -> String {
    format!("usage: scheduler [--file PATH] [--seed N]\n\n\
             Weekly employee shift scheduler\n\n\
             options:\n  \
             --file PATH  read preferences from this file instead of prompting\n  \
             --seed N     seed for the random staffing choices (default {})", DEFAULT_SEED)
}

fn parse_args() -> Result<(Option<String>, u64), String> {
    let mut file = None;
    let mut seed = DEFAULT_SEED;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--file" => file = Some(args.next().ok_or("--file needs a path")?),
            "--seed" => {
                let value = args.next().ok_or("--seed needs a number")?;
                seed = value
                    .parse()
                    .map_err(|_| format!("--seed must be zero or positive, not '{}'", value))?;
            }
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{}'\n{}", other, usage())),
        }
    }
    Ok((file, seed))
}

fn main() -> ExitCode {
    let (file, seed) = match parse_args() {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("error: {}", err);
            return ExitCode::from(1);
        }
    };
    let employees = match file {
        Some(path) => match read_file(&path) {
            Ok(employees) => employees,
            Err(err) => {
                eprintln!("error: {}", err);
                return ExitCode::from(1);
            }
        },
        None => read_interactive(),
    };
    if employees.is_empty() {
        eprintln!("error: no employees were entered");
        return ExitCode::from(1);
    }
    let mut scheduler = Scheduler::new(employees, seed);
    scheduler.build();
    if scheduler.report(seed) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
